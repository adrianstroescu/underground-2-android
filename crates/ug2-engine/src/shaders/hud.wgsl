// 2D overlay: pixel-space triangles with per-vertex colour.

struct Screen {
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    let ndc = pos / screen.size.xy * 2.0 - 1.0;
    o.clip = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    o.color = color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    return i.color;
}
