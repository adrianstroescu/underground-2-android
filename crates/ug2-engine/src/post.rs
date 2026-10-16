//! Post-processing: the scene is drawn into an HDR target; bright areas bloom (the
//! original's "over bright" glow on neon, lamps and headlights) through a quarter-size
//! bright pass and a separable blur; a final pass adds the bloom, applies a soft tone
//! shoulder, contrast, saturation and a vignette, and writes the screen image.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    texel: [f32; 2],
    dir: [f32; 2],
    bloom: [f32; 4],
    grade: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct PostSettings {
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette: f32,
    pub exposure: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self { bloom_threshold: 0.72, bloom_intensity: 0.6, contrast: 1.06, saturation: 1.12, vignette: 0.35, exposure: 1.0 }
    }
}

struct Targets {
    width: u32,
    height: u32,
    scene: wgpu::TextureView,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    bloom_size: (u32, u32),
    bright: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    composite: wgpu::BindGroup,
}

pub struct PostFx {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// Uniforms for the bright, horizontal blur, vertical blur and composite passes.
    params: [wgpu::Buffer; 4],
    targets: Option<Targets>,
}

impl PostFx {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                    },
                    count: None,
                },
                tex_entry(3),
            ],
        });
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
        let pipe = |entry: &str, format: wgpu::TextureFormat| {
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
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let params = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        Self {
            bright: pipe("fs_bright", HDR_FORMAT),
            blur: pipe("fs_blur", HDR_FORMAT),
            composite: pipe("fs_composite", output_format),
            layout,
            sampler,
            params,
            targets: None,
        }
    }

    /// (Re)creates the targets for a `width` x `height` screen.
    pub fn ensure(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if !self.targets.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            self.targets = Some(self.make_targets(device, width, height));
        }
    }

    /// The HDR texture the scene resolves into; call [`PostFx::ensure`] first.
    pub fn scene(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.scene)
    }

    fn make_targets(&self, device: &wgpu::Device, width: u32, height: u32) -> Targets {
        let mk = |w: u32, h: u32, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let bloom_size = ((width / 4).max(1), (height / 4).max(1));
        let scene = mk(width, height, "scene");
        let bloom_a = mk(bloom_size.0, bloom_size.1, "bloom a");
        let bloom_b = mk(bloom_size.0, bloom_size.1, "bloom b");
        let bg = |src: &wgpu::TextureView, extra: &wgpu::TextureView, params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(extra) },
                ],
            })
        };
        // Each pass reads textures it is not writing to; slot 3 is only used by the composite.
        let bright = bg(&scene, &bloom_b, &self.params[0]);
        let blur_h = bg(&bloom_a, &scene, &self.params[1]);
        let blur_v = bg(&bloom_b, &scene, &self.params[2]);
        let composite = bg(&scene, &bloom_a, &self.params[3]);
        Targets { width, height, scene, bloom_a, bloom_b, bloom_size, bright, blur_h, blur_v, composite }
    }

    /// Bloom and composite into `output`; `overlay` draws on top (the HUD) in the same pass.
    pub fn run(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        s: &PostSettings,
        overlay: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        let Some(t) = &self.targets else { return };
        let bloom = [s.bloom_threshold, s.bloom_intensity, s.contrast, s.saturation];
        let grade = [s.vignette, s.exposure, 0.0, 0.0];
        let scene_texel = [1.0 / t.width as f32, 1.0 / t.height as f32];
        let bloom_texel = [1.0 / t.bloom_size.0 as f32, 1.0 / t.bloom_size.1 as f32];
        let write = |i: usize, texel: [f32; 2], dir: [f32; 2]| {
            queue.write_buffer(&self.params[i], 0, bytemuck::bytes_of(&Params { texel, dir, bloom, grade }));
        };
        write(0, scene_texel, [0.0, 0.0]);
        write(1, bloom_texel, [1.0, 0.0]);
        write(2, bloom_texel, [0.0, 1.0]);
        write(3, scene_texel, [0.0, 0.0]);

        let pass = |encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup| {
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
        };
        pass(encoder, &t.bloom_a, &self.bright, &t.bright);
        pass(encoder, &t.bloom_b, &self.blur, &t.blur_h);
        pass(encoder, &t.bloom_a, &self.blur, &t.blur_v);

        let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        p.set_pipeline(&self.composite);
        p.set_bind_group(0, &t.composite, &[]);
        p.draw(0..3, 0..1);
        overlay(&mut p);
    }
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
