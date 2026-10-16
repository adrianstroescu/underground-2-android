//! The PC game's post-processing, pass for pass (see CLAUDE.md, "Post-processing chain").
//!
//! The scene resolves into an 8-bit target as on the PC (A8R8G8B8). Lamp and car-light
//! glows are not drawn into the scene: they go to their own target (`glow_target`), which
//! feeds the light-trail buffer that decays every frame. After the colour treatment
//! (gainmap, tint, rampmap), an "overbright" bloom at half size is added to the trails,
//! and the screen is scene + trails.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

/// 8-bit, like the original's render targets.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The trail buffer decays by a factor every frame; 16-bit keeps faint values from
/// sticking at 1/255 at 120 Hz.
pub const TRAIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Params {
    src: [f32; 4],
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
}

/// The speed blur of one frame (speed2.exe 0x5cc460).
#[derive(Clone, Copy, Debug)]
pub struct SpeedBlur {
    /// Blend of the blurred world over the sharp one (BLENDFACTOR, a multiple of 1/255).
    pub amount: f32,
    /// Tap spacing of the first pass; the second uses 4×.
    pub k: f32,
    /// Normalised camera velocity in D3D view space (x right, y up, z forward).
    pub dir: [f32; 3],
}

/// What the game's visual treatment decided for this frame (see `visual.rs`).
#[derive(Clone, Copy, Debug)]
pub struct PostFrame {
    pub speed_blur: Option<SpeedBlur>,
    /// Gain map curve parameters for R, G, B and the luminance alpha.
    pub gain: [f32; 4],
    pub saturation: f32,
    pub tint: [f32; 4],
    pub overbright_offset: f32,
    pub overbright_grey: f32,
    /// Half-size scene added to the bloom (BLENDFACTOR 0xB0 in a race, 0xA0 in menus).
    pub half_add: f32,
    /// Bloom added to the trails each frame (0 when bloom is off).
    pub bloom: f32,
    /// Trail decay and weight for this frame's length, per channel.
    pub trail_decay: [f32; 3],
    pub trail_weight: [f32; 3],
}

impl Default for PostFrame {
    fn default() -> Self {
        Self {
            speed_blur: None,
            gain: [0.5; 4],
            saturation: 0.4,
            tint: [0.0; 4],
            overbright_offset: 0.3627,
            overbright_grey: 150.0 / 255.0,
            half_add: 176.0 / 255.0,
            bloom: 22.0 / 255.0,
            trail_decay: [120.0 / 255.0, 90.0 / 255.0, 170.0 / 255.0],
            trail_weight: [1.0; 3],
        }
    }
}

/// The original's contrast curve (speed2.exe 0x5bbdb0): a bias around `pivot` set by
/// `p` (0.5 = identity).
pub fn gain_curve(x: f32, p: f32, pivot: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let pivot = if pivot <= 0.1 { 0.1 } else { pivot };
    let t = (1.0 - x / pivot) * (1.0 / p - 2.0);
    if x < pivot {
        x / (t + 1.0)
    } else {
        (t - x) / (t - 1.0)
    }
}

/// The 32×32 gain map (speed2.exe 0x5bbe80): at texel (x, y), R = f(y, p0), G = f(x, p1),
/// B = f(y, p2) and A = f(x, p3), each truncated to 8 bits.
pub fn gain_map(p: [f32; 4]) -> Vec<u8> {
    const N: usize = 32;
    const PIVOT: f32 = 0.4;
    let byte = |v: f32| (v * 255.0) as u8;
    let mut out = vec![0u8; N * N * 4];
    for y in 0..N {
        let u = y as f32 / (N - 1) as f32;
        for x in 0..N {
            let v = x as f32 / (N - 1) as f32;
            let o = (y * N + x) * 4;
            out[o] = byte(gain_curve(u, p[0], PIVOT));
            out[o + 1] = byte(gain_curve(v, p[1], PIVOT));
            out[o + 2] = byte(gain_curve(u, p[2], PIVOT));
            out[o + 3] = byte(gain_curve(v, p[3], PIVOT));
        }
    }
    out
}

/// The 1×256 ramp map (speed2.exe 0x5bbf80): blue = x / (1 + (1 − x)(1/0.45 − 2)).
pub fn ramp_map() -> Vec<u8> {
    let mut out = vec![0u8; 256 * 4];
    for i in 0..256 {
        let x = i as f32 / 255.0;
        let y = if x <= 0.0 {
            0.0
        } else if x >= 1.0 {
            1.0
        } else {
            x / (1.0 + (1.0 - x) * (1.0 / 0.45 - 2.0))
        };
        let a = (x * 255.0) as u8;
        out[i * 4..i * 4 + 4].copy_from_slice(&[a, a, (y * 255.0) as u8, a]);
    }
    out
}

struct Targets {
    width: u32,
    height: u32,
    /// World (and, without speed blur, everything) resolves here.
    s0: wgpu::TextureView,
    /// The car pass resolves here when the speed blur splits the scene.
    s1: wgpu::TextureView,
    tmp: wgpu::TextureView,
    graded: wgpu::TextureView,
    h1: wgpu::TextureView,
    h2: wgpu::TextureView,
    h3: wgpu::TextureView,
    glows: wgpu::TextureView,
    glows_msaa: Option<wgpu::TextureView>,
    trail: [wgpu::TextureView; 2],
    bg_zoom1: wgpu::BindGroup,
    bg_zoom2: wgpu::BindGroup,
    bg_blend: wgpu::BindGroup,
    bg_grade: [wgpu::BindGroup; 2],
    bg_half: wgpu::BindGroup,
    bg_over: wgpu::BindGroup,
    bg_blur: [wgpu::BindGroup; 4],
    bg_final: [wgpu::BindGroup; 2],
}

const P_ZOOM1: usize = 0;
const P_ZOOM2: usize = 1;
const P_BLEND: usize = 2;
const P_GRADE: usize = 3;
const P_HALF: usize = 4;
const P_OVER: usize = 5;
const P_BLUR: usize = 6; // 4 of them
const P_FINAL: usize = 10;
const PARAMS: usize = 11;

pub struct PostFx {
    samples: u32,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    zoom: wgpu::RenderPipeline,
    /// Drawn inside the car pass (scene + glow targets, depth attached).
    pub blend_blur: wgpu::RenderPipeline,
    grade: wgpu::RenderPipeline,
    half: wgpu::RenderPipeline,
    overbright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    final_pass: wgpu::RenderPipeline,
    params: Vec<wgpu::Buffer>,
    gain_tex: wgpu::Texture,
    gain_view: wgpu::TextureView,
    ramp_view: wgpu::TextureView,
    black: wgpu::TextureView,
    gain_params: Option<[f32; 4]>,
    targets: Option<Targets>,
    /// Which trail buffer holds the last frame.
    trail_front: usize,
}

impl PostFx {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat, samples: u32, depth: wgpu::TextureFormat) -> Self {
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                tex(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    },
                    count: None,
                },
            ],
        });
        // Every post pass of the original samples with clamp and bilinear filtering.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let target = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
        let pipe = |entry: &str, targets: &[Option<wgpu::ColorTargetState>], samples: u32, depth_stencil: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil,
                multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview: None,
                cache: None,
            })
        };
        let no_glow = Some(wgpu::ColorTargetState { format: SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() });
        let blend_blur = pipe(
            "fs_blend_blur",
            &[target(SCENE_FORMAT), no_glow],
            samples,
            Some(wgpu::DepthStencilState {
                format: depth,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
        );
        let params = (0..PARAMS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("post params"),
                    size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let lut = |label, w: u32, h: u32, data: &[u8]| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            write_texture(queue, &t, w, h, data);
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            (t, v)
        };
        let (gain_tex, gain_view) = lut("gain map", 32, 32, &gain_map([0.5; 4]));
        let (_, ramp_view) = lut("ramp map", 1, 256, &ramp_map());
        let (_, black) = lut("black", 1, 1, &[0, 0, 0, 0]);
        Self {
            samples,
            zoom: pipe("fs_zoom", &[target(SCENE_FORMAT)], 1, None),
            blend_blur,
            grade: pipe("fs_grade", &[target(SCENE_FORMAT)], 1, None),
            half: pipe("fs_half", &[target(SCENE_FORMAT)], 1, None),
            overbright: pipe("fs_overbright", &[target(SCENE_FORMAT)], 1, None),
            blur: pipe("fs_blur", &[target(SCENE_FORMAT)], 1, None),
            final_pass: pipe("fs_final", &[target(output_format), target(TRAIL_FORMAT)], 1, None),
            layout,
            sampler,
            params,
            gain_tex,
            gain_view,
            ramp_view,
            black,
            gain_params: None,
            targets: None,
            trail_front: 0,
        }
    }

    /// (Re)creates the targets for a `width` x `height` screen.
    pub fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if !self.targets.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            self.targets = Some(self.make_targets(device, width, height));
        }
    }

    /// Resolve target of the world pass.
    pub fn world_target(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("post targets").s0
    }

    /// Resolve target of the car pass when the speed blur splits the scene.
    pub fn car_target(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("post targets").s1
    }

    /// The glow target: (attachment, resolve target).
    pub fn glow_target(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        let t = self.targets.as_ref().expect("post targets");
        match &t.glows_msaa {
            Some(ms) => (ms, Some(&t.glows)),
            None => (&t.glows, None),
        }
    }

    /// Bind group of `blend_blur` (world and its blurred copy).
    pub fn blend_bind_group(&self) -> &wgpu::BindGroup {
        &self.targets.as_ref().expect("post targets").bg_blend
    }

    fn make_targets(&self, device: &wgpu::Device, width: u32, height: u32) -> Targets {
        let mk = |w: u32, h: u32, format, samples, label: &str| {
            let usage = if samples > 1 {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
            };
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (hw, hh) = ((width / 2).max(1), (height / 2).max(1));
        let s0 = mk(width, height, SCENE_FORMAT, 1, "scene");
        let s1 = mk(width, height, SCENE_FORMAT, 1, "scene with cars");
        let tmp = mk(width, height, SCENE_FORMAT, 1, "blur");
        let graded = mk(width, height, SCENE_FORMAT, 1, "graded");
        let h1 = mk(hw, hh, SCENE_FORMAT, 1, "half 1");
        let h2 = mk(hw, hh, SCENE_FORMAT, 1, "half 2");
        let h3 = mk(hw, hh, SCENE_FORMAT, 1, "half 3");
        let glows = mk(width, height, SCENE_FORMAT, 1, "glows");
        let glows_msaa = (self.samples > 1).then(|| mk(width, height, SCENE_FORMAT, self.samples, "glows msaa"));
        let trail = [mk(width, height, TRAIL_FORMAT, 1, "trail a"), mk(width, height, TRAIL_FORMAT, 1, "trail b")];
        let bg = |t: [&wgpu::TextureView; 4], p: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(t[0]) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(t[1]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(t[2]) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(t[3]) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 5, resource: self.params[p].as_entire_binding() },
                ],
            })
        };
        let k = &self.black;
        Targets {
            bg_zoom1: bg([&s0, k, k, k], P_ZOOM1),
            bg_zoom2: bg([&tmp, k, k, k], P_ZOOM2),
            // The second zoom pass writes `graded`, free until the colour pass.
            bg_blend: bg([&s0, &graded, k, k], P_BLEND),
            bg_grade: [bg([&s0, &self.gain_view, &self.ramp_view, k], P_GRADE), bg([&s1, &self.gain_view, &self.ramp_view, k], P_GRADE)],
            bg_half: bg([&graded, k, k, k], P_HALF),
            bg_over: bg([&h3, k, k, k], P_OVER),
            bg_blur: [
                bg([&h1, k, k, k], P_BLUR),
                bg([&h2, k, k, k], P_BLUR + 1),
                bg([&h1, k, k, k], P_BLUR + 2),
                bg([&h2, &h3, k, k], P_BLUR + 3),
            ],
            bg_final: [bg([&graded, &h1, &glows, &trail[0]], P_FINAL), bg([&graded, &h1, &glows, &trail[1]], P_FINAL)],
            width,
            height,
            s0,
            s1,
            tmp,
            graded,
            h1,
            h2,
            h3,
            glows,
            glows_msaa,
            trail,
        }
    }

    fn write(&self, queue: &wgpu::Queue, i: usize, size: (u32, u32), a: [f32; 4], b: [f32; 4], c: [f32; 4]) {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let p = Params { src: [w, h, 1.0 / w, 1.0 / h], a, b, c };
        queue.write_buffer(&self.params[i], 0, bytemuck::bytes_of(&p));
    }

    /// Rebuilds the gain map when its curve parameters change (the game does the same).
    fn update_gain(&mut self, queue: &wgpu::Queue, p: [f32; 4]) {
        if self.gain_params != Some(p) {
            write_texture(queue, &self.gain_tex, 32, 32, &gain_map(p));
            self.gain_params = Some(p);
        }
    }

    /// The speed blur's two zoom passes: world → tmp → graded (used as scratch).
    pub fn run_speed_blur(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, blur: &SpeedBlur) {
        let Some(t) = &self.targets else { return };
        let size = (t.width, t.height);
        let [x, y, z] = blur.dir;
        self.write(queue, P_ZOOM1, size, [blur.k, x, y, z], [0.0; 4], [0.0; 4]);
        self.write(queue, P_ZOOM2, size, [blur.k * 4.0, x, y, z], [0.0; 4], [0.0; 4]);
        self.write(queue, P_BLEND, size, [blur.amount, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4]);
        fullscreen(encoder, &t.tmp, &self.zoom, &t.bg_zoom1);
        fullscreen(encoder, &t.graded, &self.zoom, &t.bg_zoom2);
    }

    /// Colour treatment, bloom and trails into `output`; `overlay` draws on top (the HUD)
    /// in the same pass. `split` says the scene is in the car target (speed blur).
    pub fn run(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        f: &PostFrame,
        split: bool,
        overlay: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        self.update_gain(queue, f.gain);
        let Some(t) = &self.targets else { return };
        let full = (t.width, t.height);
        let half = ((t.width / 2).max(1), (t.height / 2).max(1));
        let z = [0.0; 4];
        self.write(queue, P_GRADE, full, [f.saturation, 0.0, 0.0, 0.0], f.tint, z);
        self.write(queue, P_HALF, full, z, z, z);
        self.write(queue, P_OVER, half, [f.overbright_offset, f.overbright_grey, 0.0, 0.0], z, z);
        self.write(queue, P_BLUR, half, [1.0, 0.0, 0.0, 0.0], z, z);
        self.write(queue, P_BLUR + 1, half, [0.0, 1.0, 0.0, 0.0], z, z);
        self.write(queue, P_BLUR + 2, half, [1.0, 0.0, 0.0, 0.0], z, z);
        self.write(queue, P_BLUR + 3, half, [0.0, 1.0, f.half_add, 0.0], z, z);
        let [dr, dg, db] = f.trail_decay;
        let [wr, wg, wb] = f.trail_weight;
        self.write(queue, P_FINAL, full, [dr, dg, db, 0.0], [wr, wg, wb, 0.0], [f.bloom, 0.0, 0.0, 0.0]);

        fullscreen(encoder, &t.graded, &self.grade, &t.bg_grade[split as usize]);
        fullscreen(encoder, &t.h3, &self.half, &t.bg_half);
        fullscreen(encoder, &t.h1, &self.overbright, &t.bg_over);
        // Two horizontal + vertical rounds (speed2.exe 0x7ff110 = 2), ending in h1.
        fullscreen(encoder, &t.h2, &self.blur, &t.bg_blur[0]);
        fullscreen(encoder, &t.h1, &self.blur, &t.bg_blur[1]);
        fullscreen(encoder, &t.h2, &self.blur, &t.bg_blur[2]);
        fullscreen(encoder, &t.h1, &self.blur, &t.bg_blur[3]);

        let back = 1 - self.trail_front;
        let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("final"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &t.trail[back],
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        p.set_pipeline(&self.final_pass);
        p.set_bind_group(0, &t.bg_final[self.trail_front], &[]);
        p.draw(0..3, 0..1);
        overlay(&mut p);
        drop(p);
        self.trail_front = back;
    }
}

fn fullscreen(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup) {
    let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("post"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    p.set_pipeline(pipe);
    p.set_bind_group(0, bg, &[]);
    p.draw(0..3, 0..1);
}

fn write_texture(queue: &wgpu::Queue, t: &wgpu::Texture, w: u32, h: u32, data: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_is_continuous_and_identity_at_half() {
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!((gain_curve(x, 0.5, 0.4) - x).abs() < 1e-6);
        }
        assert!((gain_curve(0.4, 0.26, 0.4) - 0.4).abs() < 1e-6);
        assert!((gain_curve(0.999, 0.26, 0.4) - 1.0).abs() < 0.01);
    }
}
