// World, sky and car geometry.

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    fog_color: vec4<f32>,
    // x = fog start, y = where it would be complete, z = most fog (view depth, metres)
    fog: vec4<f32>,
    light_dir: vec4<f32>,
    ambient: vec4<f32>,
    // x, y = target size in pixels, z = road reflection strength
    screen: vec4<f32>,
    // Fragments with dot(world, xyz) + w < 0 are clipped (the reflection pass).
    clip: vec4<f32>,
    // The player's headlight projector and its colour (w = 1 when on).
    headlight_vp: mat4x4<f32>,
    headlight_color: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    tint: vec4<f32>,
    // x = mode (0 world, 1 car, 2 glass, 3 unlit), y = alpha cutoff, z = fog on,
    // w = vertex colour scale
    params: vec4<f32>,
    // Light emitted on top of the lit colour (lamps), as a multiple of the texture.
    emissive: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var refl_tex: texture_2d<f32>;
@group(0) @binding(2) var refl_samp: sampler;
@group(0) @binding(3) var headlight_tex: texture_2d<f32>;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
@group(2) @binding(0) var<uniform> m: Model;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) world: vec3<f32>,
    // Distance along the view direction (fog, as the game's per-vertex fog).
    @location(4) depth: f32,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let world = m.model * vec4<f32>(v.pos, 1.0);
    var o: VsOut;
    o.clip = g.view_proj * world;
    o.uv = v.uv;
    o.color = v.color;
    o.normal = (m.model * vec4<f32>(v.normal, 0.0)).xyz;
    o.world = world.xyz;
    o.depth = o.clip.w;
    return o;
}

// Light the player's headlights throw on world geometry (the game's worldheadlight
// effect): the HEADLIGHTS pattern projected from the car, nothing behind the lamps.
fn headlight(world: vec3<f32>) -> vec3<f32> {
    if (g.headlight_color.w < 0.5) {
        return vec3<f32>(0.0);
    }
    let p = g.headlight_vp * vec4<f32>(world, 1.0);
    if (p.w <= 0.05) {
        return vec3<f32>(0.0);
    }
    let uv = vec2<f32>(p.x / p.w * 0.5 + 0.5, 0.5 - p.y / p.w * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(headlight_tex, refl_samp, uv, 0.0).rgb * g.headlight_color.rgb;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, i.uv);
    var rgb: vec3<f32>;
    var a = tex.a * m.tint.a;
    if (m.params.x > 3.5) {
        // Street, as the game's worldreflect effect: baked lighting plus the mirrored
        // city at the same screen position, weighted by texture alpha x vertex alpha.
        // The surface itself is opaque.
        rgb = tex.rgb * m.tint.rgb * (i.color.rgb + headlight(i.world)) * m.params.w;
        let r = textureSampleLevel(refl_tex, refl_samp, i.clip.xy / g.screen.xy, 0.0).rgb;
        rgb = rgb + r * (tex.a * i.color.a * g.screen.z);
        a = 1.0;
    } else if (m.params.x > 2.5) {
        // Unlit (shadows, decals).
        rgb = tex.rgb * m.tint.rgb;
    } else if (m.params.x > 0.5) {
        // Car: moonlight + city ambient, specular highlight and a cheap sky reflection.
        let n = normalize(i.normal);
        let l = normalize(g.light_dir.xyz);
        let v = normalize(g.cam_pos.xyz - i.world);
        let diff = max(dot(n, l), 0.0);
        let h = normalize(l + v);
        let spec = pow(max(dot(n, h), 0.0), 60.0) * 0.8;
        let fres = pow(1.0 - clamp(abs(dot(n, v)), 0.0, 1.0), 4.0);
        let r = reflect(-v, n);
        // Night sky above, city glow at the horizon, dark street below.
        let up = r.z * 0.5 + 0.5;
        let env = mix(mix(vec3<f32>(0.03, 0.035, 0.05), vec3<f32>(0.42, 0.40, 0.45), smoothstep(0.35, 0.5, up)),
                      vec3<f32>(0.20, 0.27, 0.42), smoothstep(0.55, 0.9, up));
        var base = tex.rgb * m.tint.rgb;
        if (m.emissive.w > 0.5) {
            // Paint with the car's detail atlas laid over it.
            base = mix(m.tint.rgb, tex.rgb, tex.a);
            a = m.tint.a;
        }
        if (m.params.x > 1.5) {
            // Glass: mostly reflection; more opaque at grazing angles.
            rgb = base * g.ambient.rgb + env * (0.35 + 0.65 * fres) + vec3<f32>(spec * 1.2);
            a = clamp(max(a, 0.55) + fres * 0.4, 0.0, 0.95);
        } else {
            rgb = base * (g.ambient.rgb + diff * 0.75) + vec3<f32>(spec) + env * (0.12 + 0.6 * fres);
        }
        rgb = rgb + tex.rgb * m.emissive.rgb;
    } else {
        rgb = tex.rgb * m.tint.rgb * (i.color.rgb + headlight(i.world)) * m.params.w;
        a = a * i.color.a;
    }
    if (a < m.params.y || dot(i.world, g.clip.xyz) + g.clip.w < 0.0) {
        discard;
    }
    if (m.params.z > 0.5) {
        let f = min(clamp((i.depth - g.fog.x) / (g.fog.y - g.fog.x), 0.0, 1.0), g.fog.z);
        rgb = mix(rgb, g.fog_color.rgb, f);
    }
    return vec4<f32>(rgb, a);
}
