// World, sky and car geometry.

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    fog_color: vec4<f32>,
    // x = fog start, y = fog end, z = fog strength
    fog: vec4<f32>,
    light_dir: vec4<f32>,
    ambient: vec4<f32>,
};

struct Model {
    model: mat4x4<f32>,
    tint: vec4<f32>,
    // x = lit (car shading), y = alpha cutoff, z = fog on, w = vertex colour scale
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
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
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, i.uv);
    var rgb: vec3<f32>;
    var a = tex.a * m.tint.a;
    if (m.params.x > 0.5) {
        // Car: moonlight + city ambient, specular highlight and a cheap sky reflection.
        let n = normalize(i.normal);
        let l = normalize(g.light_dir.xyz);
        let v = normalize(g.cam_pos.xyz - i.world);
        let diff = max(dot(n, l), 0.0);
        let h = normalize(l + v);
        let spec = pow(max(dot(n, h), 0.0), 60.0) * 0.8;
        let fres = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.0);
        let r = reflect(-v, n);
        let env = mix(vec3<f32>(0.04, 0.05, 0.08), vec3<f32>(0.30, 0.38, 0.55), clamp(r.z * 0.5 + 0.5, 0.0, 1.0));
        let base = tex.rgb * m.tint.rgb;
        rgb = base * (g.ambient.rgb + diff * 0.75) + vec3<f32>(spec) + env * (0.15 + 0.6 * fres);
    } else {
        rgb = tex.rgb * m.tint.rgb * i.color.rgb * m.params.w;
        a = a * i.color.a;
    }
    if (a < m.params.y) {
        discard;
    }
    if (m.params.z > 0.5) {
        let d = distance(i.world, g.cam_pos.xyz);
        let f = clamp((d - g.fog.x) / (g.fog.y - g.fog.x), 0.0, 1.0) * g.fog.z;
        rgb = mix(rgb, g.fog_color.rgb, f);
    }
    return vec4<f32>(rgb, a);
}
