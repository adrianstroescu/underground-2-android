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
    // Car lighting: the CAR effect's HarmonicCoeff (second-order SH in car space).
    sh: array<vec4<f32>, 10>,
};

// A light material as the CAR effect's constants.
struct Material {
    diffuse_min: vec4<f32>,
    diffuse_range: vec4<f32>,
    // w = specular power
    specular_min: vec4<f32>,
    // w = envmap power
    specular_range: vec4<f32>,
    envmap_min: vec4<f32>,
    envmap_range: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    tint: vec4<f32>,
    // x = mode (0 world, 1 car, 2 glass, 3 unlit), y = alpha cutoff, z = fog on,
    // w = vertex colour scale
    params: vec4<f32>,
    // Light emitted on top of the lit colour (lamps), as a multiple of the texture.
    emissive: vec4<f32>,
    // x = light material of the car paint
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var refl_tex: texture_2d<f32>;
@group(0) @binding(2) var refl_samp: sampler;
@group(0) @binding(3) var headlight_tex: texture_2d<f32>;
@group(0) @binding(4) var<uniform> materials: array<Material, 192>;
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
    // Light material of the draw (cars), from the instance index.
    @location(5) @interpolate(flat) material: u32,
    // Model-space normal and eye direction (the car effect lights in car space).
    @location(6) local_normal: vec3<f32>,
    @location(7) local_view: vec3<f32>,
};

@vertex
fn vs_main(v: VsIn, @builtin(instance_index) material: u32) -> VsOut {
    let world = m.model * vec4<f32>(v.pos, 1.0);
    var o: VsOut;
    o.clip = g.view_proj * world;
    o.uv = v.uv;
    o.color = v.color;
    o.normal = (m.model * vec4<f32>(v.normal, 0.0)).xyz;
    o.world = world.xyz;
    o.depth = o.clip.w;
    o.material = material;
    o.local_normal = v.normal;
    // Eye in model space (the model matrix is a rotation and a translation).
    let r = mat3x3<f32>(m.model[0].xyz, m.model[1].xyz, m.model[2].xyz);
    o.local_view = transpose(r) * (g.cam_pos.xyz - m.model[3].xyz) - v.pos;
    return o;
}

// The CAR effect's lighting: second-order spherical harmonics in car space.
fn sh_eval(n: vec3<f32>) -> vec4<f32> {
    return g.sh[0] + g.sh[1] * (n.x * n.x) + g.sh[2] * (n.y * n.y) + g.sh[3] * (n.z * n.z)
        + g.sh[4] * (n.x * n.y) + g.sh[5] * (n.x * n.z) + g.sh[6] * (n.z * n.y)
        + g.sh[7] * n.x + g.sh[8] * n.y + g.sh[9] * n.z;
}

// What a car reflects (the game uses a cube map of its surroundings): night sky above,
// the fog colour at the horizon, dark street below.
fn env_color(r: vec3<f32>) -> vec3<f32> {
    let horizon = g.fog_color.rgb;
    if (r.z > 0.0) {
        return mix(horizon, vec3<f32>(0.20, 0.17, 0.15), clamp(r.z * 2.0, 0.0, 1.0));
    }
    return mix(horizon, vec3<f32>(0.04, 0.04, 0.05), clamp(-r.z * 4.0, 0.0, 1.0));
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
        // Car, as the game's CAR effect (vertex terms per pixel here).
        var base = tex.rgb * m.tint.rgb;
        var base_a = a;
        if (m.emissive.w > 0.5) {
            // Paint with the car's detail atlas laid over it.
            base = mix(m.tint.rgb, tex.rgb, tex.a);
            base_a = m.tint.a;
            a = base_a;
        }
        var id = i.material;
        if (id == 65535u) {
            id = u32(m.extra.x);
        }
        let mat = materials[min(id, 191u)];
        let n = normalize(i.local_normal);
        let v = normalize(i.local_view);
        let ndv = max(dot(v, n), 0.01);
        // Diffuse: SH on the normal, scaled from DiffuseMin to Max by N.V.
        let diff = sh_eval(n).rgb * (mat.diffuse_min.rgb + ndv * mat.diffuse_range.rgb) * 0.5;
        let diff_a = (mat.diffuse_min.a + ndv * mat.diffuse_range.a) * 0.5;
        // Specular: SH on the reflection vector, its alpha raised to the power.
        let r = 2.0 * ndv * n - v;
        let sr = sh_eval(r);
        let spec_light = sr.rgb * pow(max(clamp(sr.a, 0.0, 1.0), 1e-4), mat.specular_min.w);
        let spec = spec_light * (mat.specular_min.rgb + pow(ndv, mat.specular_min.w) * mat.specular_range.rgb) * 0.5;
        // Envmap, weighted from EnvmapMin to Max by N.V^EnvmapPower.
        let rw = normalize((m.model * vec4<f32>(r, 0.0)).xyz);
        let env_k = mat.envmap_min.rgb + pow(ndv, mat.specular_range.w) * mat.envmap_range.rgb;
        let shine = 2.0 * env_color(rw) * env_k + spec;
        let lit = 2.0 * base * diff;
        let lit_a = 2.0 * base_a * diff_a;
        // The texture alpha times the diffuse alpha (a fresnel term) weights the shine.
        rgb = 2.0 * shine * lit_a + lit;
        if (m.params.x > 1.5) {
            // Glass blends with the alpha the effect computes.
            a = clamp(lit_a, 0.0, 1.0);
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
