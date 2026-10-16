// Post-processing: bright-pass downsample, separable blur and the final composite
// (bloom, tone curve, grading, vignette). All passes draw one fullscreen triangle.

struct Params {
    // Size of one source texel in UV.
    texel: vec2<f32>,
    // Blur direction (1,0) or (0,1).
    dir: vec2<f32>,
    // x = bloom threshold, y = bloom intensity, z = contrast, w = saturation
    bloom: vec4<f32>,
    // x = vignette strength, y = exposure
    grade: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> p: Params;
@group(0) @binding(3) var bloom_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the screen.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VsOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Downsample with a 4-tap box and keep only what is brighter than the threshold
// (soft knee, so the bloom fades in instead of popping).
@fragment
fn fs_bright(i: VsOut) -> @location(0) vec4<f32> {
    let t = p.texel;
    var c = textureSample(src, samp, i.uv + vec2<f32>(-t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, -t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(-t.x, t.y)).rgb;
    c += textureSample(src, samp, i.uv + vec2<f32>(t.x, t.y)).rgb;
    c *= 0.25;
    let l = luma(c);
    let knee = 0.25;
    let soft = clamp(l - p.bloom.x + knee, 0.0, 2.0 * knee);
    let contrib = max(soft * soft / (4.0 * knee + 1e-4), l - p.bloom.x) / max(l, 1e-4);
    return vec4<f32>(c * contrib, 1.0);
}

// 9-tap Gaussian along `dir`, using linear filtering to read two texels per tap.
@fragment
fn fs_blur(i: VsOut) -> @location(0) vec4<f32> {
    let d = p.dir * p.texel;
    var c = textureSample(src, samp, i.uv).rgb * 0.2270270270;
    c += textureSample(src, samp, i.uv + d * 1.3846153846).rgb * 0.3162162162;
    c += textureSample(src, samp, i.uv - d * 1.3846153846).rgb * 0.3162162162;
    c += textureSample(src, samp, i.uv + d * 3.2307692308).rgb * 0.0702702703;
    c += textureSample(src, samp, i.uv - d * 3.2307692308).rgb * 0.0702702703;
    return vec4<f32>(c, 1.0);
}

// Identity up to `k`, then a smooth shoulder towards 1: keeps the game's authored look
// while letting lamps and neon exceed 1 without hard clipping.
fn shoulder(x: vec3<f32>) -> vec3<f32> {
    let k = 0.78;
    let over = max(x - vec3<f32>(k), vec3<f32>(0.0));
    let squeezed = vec3<f32>(k) + (1.0 - k) * (vec3<f32>(1.0) - exp(-over / (1.0 - k)));
    return select(x, squeezed, x > vec3<f32>(k));
}

@fragment
fn fs_composite(i: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(src, samp, i.uv).rgb * p.grade.y;
    c += textureSample(bloom_tex, samp, i.uv).rgb * p.bloom.y;
    c = shoulder(c);
    // Contrast around mid grey, then saturation.
    c = (c - vec3<f32>(0.5)) * p.bloom.z + vec3<f32>(0.5);
    let l = luma(c);
    c = mix(vec3<f32>(l), c, p.bloom.w);
    // Vignette.
    let v = i.uv - vec2<f32>(0.5);
    c *= 1.0 - p.grade.x * dot(v, v) * 1.6;
    return vec4<f32>(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
