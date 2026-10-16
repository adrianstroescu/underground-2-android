// A movie frame: YUV 4:2:0 planes to RGB (full-range BT.601, as VP6 decodes).

@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var u_tex: texture_2d<f32>;
@group(0) @binding(2) var v_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    // VP6 pictures are stored bottom-up.
    let uv = vec2<f32>(i.uv.x, 1.0 - i.uv.y);
    let y = textureSample(y_tex, samp, uv).r;
    let u = textureSample(u_tex, samp, uv).r - 0.5;
    let v = textureSample(v_tex, samp, uv).r - 0.5;
    let rgb = vec3<f32>(y + 1.402 * v, y - 0.344136 * u - 0.714136 * v, y + 1.772 * u);
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
