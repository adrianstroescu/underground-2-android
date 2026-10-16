// The PC game's post-processing chain (speed2.exe + an apitrace capture; see CLAUDE.md,
// "Post-processing chain"). Every pass draws one fullscreen triangle. Intermediate
// targets of the original are 8-bit, so values are rounded to 1/255 between the steps
// that this file merges into one pass.

struct Params {
    // Source size in texels (x, y) and 1/size (z, w).
    src: vec4<f32>,
    // Pass-specific values, see each entry point.
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
};

@group(0) @binding(0) var t0: texture_2d<f32>;
@group(0) @binding(1) var t1: texture_2d<f32>;
@group(0) @binding(2) var t2: texture_2d<f32>;
@group(0) @binding(3) var t3: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;
@group(0) @binding(5) var<uniform> p: Params;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VsOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// An 8-bit render target between two of the original's passes.
fn q(c: vec4<f32>) -> vec4<f32> {
    return round(clamp(c, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0) / 255.0;
}

// Speed blur (0x5cc460): four taps along the zoom from the direction of travel,
// tap k at uv + k·offset, offset = K·(n.z·(2u−1) − n.x, n.z·(2v−1) − n.y).
// a = (K, n.x, n.y, n.z)
@fragment
fn fs_zoom(i: VsOut) -> @location(0) vec4<f32> {
    let k = p.a.x;
    let n = p.a.yzw;
    let off = k * vec2<f32>(n.z * (2.0 * i.uv.x - 1.0) - n.x, n.z * (2.0 * i.uv.y - 1.0) - n.y);
    var c = vec4<f32>(0.0);
    for (var t = 0; t < 4; t++) {
        c += textureSampleLevel(t0, samp, i.uv + off * f32(t), 0.0);
    }
    return q(c * 0.25);
}

// The world with its blurred copy blended on top (BLENDFACTOR/INVBLENDFACTOR), drawn
// first in the car pass. t0 = world, t1 = blurred; a.x = blend factor.
@fragment
fn fs_blend_blur(i: VsOut) -> @location(0) vec4<f32> {
    let w = textureSampleLevel(t0, samp, i.uv, 0.0);
    let b = textureSampleLevel(t1, samp, i.uv, 0.0);
    return mix(w, b, p.a.x);
}

// Colour treatment: the gainmap, tint (saturation) and rampmap effects of the original
// in one pass. t0 = scene, t1 = gain map 32x32, t2 = ramp map 1x256.
// a = (SaturationScale, -, -, -), b = TintColor (rgb, w = mix towards the gain alpha).
@fragment
fn fs_grade(i: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(t0, samp, i.uv, 0.0);
    // gainmap: lookups at (lum, R) and (G, B); the alpha of the first says how much of
    // the curve applies.
    let lum = dot(c, vec4<f32>(0.3, 0.59, 0.11, 0.0));
    let g1 = textureSampleLevel(t1, samp, vec2<f32>(lum, c.r), 0.0);
    let g2 = textureSampleLevel(t1, samp, vec2<f32>(c.g, c.b), 0.0);
    let k = clamp(1.0 - g1.a, 0.0, 1.0);
    var o = q(vec4<f32>(
        g1.r * k + c.r * (1.0 - k),
        g2.g * k + c.g * (1.0 - k),
        g2.b * k + c.b * (1.0 - k),
        g1.a * k + g1.r * (1.0 - k),
    ));
    // tint: add TintColor where the colour is not warm (max(R, G) not above B).
    let warm = max(o.r, o.g) - o.b;
    let w = clamp(1.0 - warm, 0.0, 1.0) * p.a.x;
    o = q(vec4<f32>(o.rgb + w * p.b.rgb, o.a));
    // rampmap: blue through the ramp, then everything mixed towards the alpha by TintColor.w.
    let ramp = textureSampleLevel(t2, samp, vec2<f32>(o.g, o.b), 0.0).b;
    let tw = p.b.w;
    let rgb = vec3<f32>(o.r, o.g, ramp) * (1.0 - tw) + vec3<f32>(o.a * tw);
    return q(vec4<f32>(rgb, (1.0 - tw) * (1.0 - tw) + tw));
}

// Half-size copy: one bilinear tap between four texels (the original's 2x2 average).
@fragment
fn fs_half(i: VsOut) -> @location(0) vec4<f32> {
    return q(textureSampleLevel(t0, samp, i.uv, 0.0));
}

// OverBright: (sat(4·(c − offset)) · 0.588·Σ)², alpha 0.
// a = (OverBrightOffset, OverBrightGreyScale, -, -)
@fragment
fn fs_overbright(i: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(t0, samp, i.uv, 0.0).rgb;
    let r = clamp(4.0 * (c - vec3<f32>(p.a.x)), vec3<f32>(0.0), vec3<f32>(1.0));
    let s = dot(r, vec3<f32>(p.a.y));
    let v = r * s;
    return q(vec4<f32>(v * v, 0.0));
}

// Blur: texels at ±1 and ±3 along a.xy (in texels), weights 1/3 and 1/6, no centre tap.
// a.z > 0 also adds t1 · a.z (the half-size scene, BLENDFACTOR/ONE).
@fragment
fn fs_blur(i: VsOut) -> @location(0) vec4<f32> {
    let d = p.a.xy * p.src.zw;
    var c = textureSampleLevel(t0, samp, i.uv + d, 0.0) / 3.0;
    c += textureSampleLevel(t0, samp, i.uv + d * 3.0, 0.0) / 6.0;
    c += textureSampleLevel(t0, samp, i.uv - d, 0.0) / 3.0;
    c += textureSampleLevel(t0, samp, i.uv - d * 3.0, 0.0) / 6.0;
    c = q(c);
    if p.a.z > 0.0 {
        c = q(c + textureSampleLevel(t1, samp, i.uv, 0.0) * p.a.z);
    }
    return c;
}

struct FinalOut {
    @location(0) screen: vec4<f32>,
    @location(1) trail: vec4<f32>,
};

// Light trails and output. t0 = graded scene, t1 = bloom (half size), t2 = this frame's
// glows, t3 = last frame's trail buffer.
// trail = sat(k·trail + w·(glows + bloom·b)); screen = sat(scene + trail).
// a = per-channel decay k, b = per-channel weight w (both for this frame's length),
// c.x = bloom strength.
@fragment
fn fs_final(i: VsOut) -> FinalOut {
    let scene = textureSampleLevel(t0, samp, i.uv, 0.0);
    let bloom = textureSampleLevel(t1, samp, i.uv, 0.0).rgb;
    let glows = textureSampleLevel(t2, samp, i.uv, 0.0).rgb;
    let last = textureSampleLevel(t3, samp, i.uv, 0.0).rgb;
    let trail = clamp(last * p.a.rgb + p.b.rgb * (glows + bloom * p.c.x), vec3<f32>(0.0), vec3<f32>(1.0));
    var o: FinalOut;
    o.screen = vec4<f32>(clamp(scene.rgb + trail, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
    o.trail = vec4<f32>(trail, 1.0);
    return o;
}
