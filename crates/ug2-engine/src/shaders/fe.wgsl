// The original frontend (FEng) quads: pixel-space, textured, per-vertex colour,
// alpha blended.

struct Screen {
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var t_tex: texture_2d<f32>;
@group(1) @binding(1) var s_tex: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    let ndc = pos / screen.size.xy * 2.0 - 1.0;
    o.clip = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    return textureSample(t_tex, s_tex, i.uv) * i.color;
}

// Textures that do not blend (the track maps): colour only, alpha from the vertex.
@fragment
fn fs_opaque(i: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_tex, s_tex, i.uv);
    return vec4<f32>(t.rgb * i.color.rgb, i.color.a);
}
