//! wgpu renderer (Vulkan on Android). The world is pre-merged per scenery section and
//! texture, so a frame is a few thousand indexed draws.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::assets::{Blend, DrawRange, GpuVertex, MeshData, PreparedTexture, PAINT};
use crate::hud::HudVertex;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub bc_textures: bool,
}

impl Gpu {
    pub fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> anyhow::Result<Self> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            force_fallback_adapter: false,
        }))?;
        let info = adapter.get_info();
        let bc_textures = adapter.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        log::info!("GPU: {} ({:?}, {}), BC textures: {bc_textures}", info.name, info.backend, info.driver_info);
        let mut features = wgpu::Features::empty();
        if bc_textures {
            features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ug2"),
            required_features: features,
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))?;
        device.on_uncaptured_error(Box::new(|e| log::error!("wgpu: {e}")));
        Ok(Self { instance, adapter, device, queue, bc_textures })
    }

    pub fn surface_config(&self, surface: &wgpu::Surface<'_>, width: u32, height: u32) -> wgpu::SurfaceConfiguration {
        let caps = surface.get_capabilities(&self.adapter);
        // The game was authored for gamma-space blending; keep it that way.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    light_dir: [f32; 4],
    ambient: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniform {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    params: [f32; 4],
}

const SLOT_STRIDE: u64 = 256;
const SLOTS: usize = 16;
const SLOT_WORLD: u32 = 0;
const SLOT_WORLD_CUTOUT: u32 = 1;
const SLOT_WORLD_BLEND: u32 = 2;
const SLOT_SKY: u32 = 3;
const SLOT_CAR: u32 = 4;
const SLOT_CAR_PAINT: u32 = 5;
const SLOT_WHEEL0: u32 = 6;

struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    ranges: Vec<DrawRange>,
    bbox_min: Vec3,
    bbox_max: Vec3,
}

/// Everything the renderer needs to draw one frame.
pub struct FrameData<'a> {
    pub view: Mat4,
    pub fov_y: f32,
    pub cam_pos: Vec3,
    pub car: Option<Mat4>,
    pub wheels: [Mat4; 4],
    pub paint: [f32; 3],
    pub draw_distance: f32,
    pub vertex_light_scale: f32,
    pub hud: &'a [HudVertex],
}

pub struct Renderer {
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    model_buf: wgpu::Buffer,
    model_bg: wgpu::BindGroup,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipe_opaque: wgpu::RenderPipeline,
    pipe_blend: wgpu::RenderPipeline,
    pipe_add: wgpu::RenderPipeline,
    pipe_sky: wgpu::RenderPipeline,
    pipe_hud: wgpu::RenderPipeline,
    hud_buf: wgpu::Buffer,
    hud_bg: wgpu::BindGroup,
    hud_vbuf: wgpu::Buffer,
    hud_capacity: u64,
    textures: HashMap<u32, wgpu::BindGroup>,
    white: wgpu::BindGroup,
    sections: Vec<GpuMesh>,
    sky: Vec<GpuMesh>,
    car_body: Option<GpuMesh>,
    car_wheel: Option<GpuMesh>,
    targets: Option<Targets>,
    pub stats_draws: u32,
}

struct Targets {
    width: u32,
    height: u32,
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const FOG_COLOR: [f32; 4] = [0.055, 0.07, 0.11, 1.0];

impl Renderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, msaa: u32) -> Self {
        let device = &gpu.device;
        let sample_count = if msaa > 1
            && gpu.adapter.get_texture_format_features(format).flags.sample_count_supported(msaa)
            && gpu.adapter.get_texture_format_features(DEPTH).flags.sample_count_supported(msaa)
        {
            msaa
        } else {
            1
        };

        let uniform_entry = |binding, dynamic: bool, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: NonZeroU64::new(size),
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, false, std::mem::size_of::<Globals>() as u64)],
        });
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
            entries: &[uniform_entry(0, true, std::mem::size_of::<ModelUniform>() as u64)],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });
        let model_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("models"),
            size: SLOT_STRIDE * SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("models"),
            layout: &model_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &model_buf,
                    offset: 0,
                    size: NonZeroU64::new(std::mem::size_of::<ModelUniform>() as u64),
                }),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("diffuse"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mesh.wgsl").into()),
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[&globals_layout, &tex_layout, &model_layout],
            push_constant_ranges: &[],
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32x2];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attrs,
        };
        let mesh_pipeline = |label: &str, blend: Option<wgpu::BlendState>, depth_write: bool, depth_test: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[vertex_layout.clone()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: depth_write,
                    depth_compare: if depth_test { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always },
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let pipe_opaque = mesh_pipeline("opaque", None, true, true);
        let pipe_blend = mesh_pipeline("blend", Some(wgpu::BlendState::ALPHA_BLENDING), false, true);
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipe_add = mesh_pipeline("additive", Some(wgpu::BlendState { color: additive, alpha: additive }), false, true);
        let pipe_sky = mesh_pipeline("sky", Some(wgpu::BlendState::ALPHA_BLENDING), false, false);

        // HUD
        let hud_layout_bg = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud"),
            entries: &[uniform_entry(0, false, 16)],
        });
        let hud_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hud_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hud"),
            layout: &hud_layout_bg,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: hud_buf.as_entire_binding() }],
        });
        let hud_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/hud.wgsl").into()),
        });
        let hud_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud"),
            bind_group_layouts: &[&hud_layout_bg],
            push_constant_ranges: &[],
        });
        let hud_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
        let pipe_hud = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&hud_pl),
            vertex: wgpu::VertexState {
                module: &hud_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &hud_attrs,
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &hud_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let hud_capacity = 64 * 1024;
        let hud_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hud vertices"),
            size: hud_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let white_tex = PreparedTexture {
            hash: 0,
            width: 4,
            height: 4,
            format: wgpu::TextureFormat::Rgba8Unorm,
            mips: 1,
            data: vec![255; 64],
        };
        let white = create_texture_bg(gpu, &tex_layout, &sampler, &white_tex).expect("white texture");

        Self {
            format,
            sample_count,
            globals_buf,
            globals_bg,
            model_buf,
            model_bg,
            tex_layout,
            sampler,
            pipe_opaque,
            pipe_blend,
            pipe_add,
            pipe_sky,
            pipe_hud,
            hud_buf,
            hud_bg,
            hud_vbuf,
            hud_capacity,
            textures: HashMap::new(),
            white,
            sections: Vec::new(),
            sky: Vec::new(),
            car_body: None,
            car_wheel: None,
            targets: None,
            stats_draws: 0,
        }
    }

    pub fn upload_texture(&mut self, gpu: &Gpu, t: &PreparedTexture) {
        if let Some(bg) = create_texture_bg(gpu, &self.tex_layout, &self.sampler, t) {
            self.textures.insert(t.hash, bg);
        }
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    fn upload_mesh(gpu: &Gpu, m: &MeshData) -> Option<GpuMesh> {
        if m.indices.is_empty() {
            return None;
        }
        let vbuf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&m.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Some(GpuMesh { vbuf, ibuf, ranges: m.ranges.clone(), bbox_min: m.bbox_min, bbox_max: m.bbox_max })
    }

    pub fn add_section(&mut self, gpu: &Gpu, mesh: &MeshData, sky: bool) {
        if let Some(m) = Self::upload_mesh(gpu, mesh) {
            if sky {
                self.sky.push(m);
            } else {
                self.sections.push(m);
            }
        }
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    pub fn set_car(&mut self, gpu: &Gpu, body: &MeshData, wheel: Option<&MeshData>) {
        self.car_body = Self::upload_mesh(gpu, body);
        self.car_wheel = wheel.and_then(|w| Self::upload_mesh(gpu, w));
    }

    fn ensure_targets(&mut self, gpu: &Gpu, width: u32, height: u32) {
        if self.targets.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            return;
        }
        let mk = |format, samples, label| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let depth = mk(DEPTH, self.sample_count, "depth");
        let msaa = (self.sample_count > 1).then(|| mk(self.format, self.sample_count, "msaa"));
        self.targets = Some(Targets { width, height, depth, msaa });
    }

    fn write_uniforms(&self, gpu: &Gpu, f: &FrameData, aspect: f32) -> Mat4 {
        let proj = Mat4::perspective_rh(f.fov_y, aspect, 0.3, 4000.0);
        let view_proj = proj * f.view;
        let light = Vec3::new(0.35, -0.45, 0.82).normalize();
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_pos: f.cam_pos.extend(1.0).into(),
            fog_color: FOG_COLOR,
            fog: [f.draw_distance * 0.25, f.draw_distance, 0.92, 0.0],
            light_dir: light.extend(0.0).into(),
            ambient: [0.30, 0.33, 0.42, 1.0],
        };
        gpu.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        let vs = f.vertex_light_scale;
        let slot = |model: Mat4, tint: [f32; 4], params: [f32; 4]| ModelUniform {
            model: model.to_cols_array_2d(),
            tint,
            params,
        };
        let one = [1.0, 1.0, 1.0, 1.0];
        let mut slots = vec![slot(Mat4::IDENTITY, one, [0.0; 4]); SLOTS];
        slots[SLOT_WORLD as usize] = slot(Mat4::IDENTITY, one, [0.0, 0.0, 1.0, vs]);
        slots[SLOT_WORLD_CUTOUT as usize] = slot(Mat4::IDENTITY, one, [0.0, 0.5, 1.0, vs]);
        slots[SLOT_WORLD_BLEND as usize] = slot(Mat4::IDENTITY, one, [0.0, 0.01, 1.0, vs]);
        // The dome (centred on the origin by the loader) follows the camera, shrunk so it
        // stays inside the far plane; it is drawn first without depth, so only direction matters.
        let sky_m = Mat4::from_translation(f.cam_pos - Vec3::Z * 60.0) * Mat4::from_scale(Vec3::splat(0.3));
        slots[SLOT_SKY as usize] = slot(sky_m, one, [0.0, 0.0, 0.0, vs]);
        let car = f.car.unwrap_or(Mat4::IDENTITY);
        slots[SLOT_CAR as usize] = slot(car, one, [1.0, 0.5, 1.0, 1.0]);
        let p = f.paint;
        slots[SLOT_CAR_PAINT as usize] = slot(car, [p[0], p[1], p[2], 1.0], [1.0, 0.5, 1.0, 1.0]);
        for (i, w) in f.wheels.iter().enumerate() {
            slots[SLOT_WHEEL0 as usize + i] = slot(*w, one, [1.0, 0.5, 1.0, 1.0]);
        }
        let mut bytes = vec![0u8; SLOT_STRIDE as usize * SLOTS];
        for (i, s) in slots.iter().enumerate() {
            let o = i * SLOT_STRIDE as usize;
            bytes[o..o + std::mem::size_of::<ModelUniform>()].copy_from_slice(bytemuck::bytes_of(s));
        }
        gpu.queue.write_buffer(&self.model_buf, 0, &bytes);
        view_proj
    }

    /// Renders a frame into `target` (a swapchain image or an offscreen texture).
    pub fn render(&mut self, gpu: &Gpu, target: &wgpu::TextureView, width: u32, height: u32, f: &FrameData) {
        self.ensure_targets(gpu, width, height);
        let view_proj = self.write_uniforms(gpu, f, width as f32 / height.max(1) as f32);
        gpu.queue.write_buffer(&self.hud_buf, 0, bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]));

        let hud_bytes: &[u8] = bytemuck::cast_slice(f.hud);
        if hud_bytes.len() as u64 > self.hud_capacity {
            self.hud_capacity = (hud_bytes.len() as u64).next_power_of_two();
            self.hud_vbuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hud vertices"),
                size: self.hud_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !hud_bytes.is_empty() {
            gpu.queue.write_buffer(&self.hud_vbuf, 0, hud_bytes);
        }

        let planes = frustum_planes(&view_proj);
        let visible: Vec<usize> = (0..self.sections.len())
            .filter(|&i| {
                let m = &self.sections[i];
                aabb_distance(f.cam_pos, m.bbox_min, m.bbox_max) < f.draw_distance
                    && aabb_in_frustum(&planes, m.bbox_min, m.bbox_max)
            })
            .collect();

        let targets = self.targets.as_ref().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let mut draws = 0u32;
        {
            let (view, resolve) = match &targets.msaa {
                Some(ms) => (ms, Some(target)),
                None => (target, None),
            };
            let c = FOG_COLOR;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }),
                        store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);

            // Sky first, no depth.
            pass.set_pipeline(&self.pipe_sky);
            for m in &self.sky {
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend != Blend::Additive).then_some(SLOT_SKY)
                });
            }

            // Opaque + cutout world.
            pass.set_pipeline(&self.pipe_opaque);
            for &i in &visible {
                let m = &self.sections[i];
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| match r.blend {
                    Blend::Opaque => Some(SLOT_WORLD),
                    Blend::Cutout => Some(SLOT_WORLD_CUTOUT),
                    Blend::Blend | Blend::Additive => None,
                });
            }
            // Car body and wheels.
            if f.car.is_some() {
                if let Some(m) = &self.car_body {
                    draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                        matches!(r.blend, Blend::Opaque | Blend::Cutout)
                            .then_some(if r.texture == PAINT { SLOT_CAR_PAINT } else { SLOT_CAR })
                    });
                }
                if let Some(m) = &self.car_wheel {
                    for w in 0..4 {
                        draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                            matches!(r.blend, Blend::Opaque | Blend::Cutout).then_some(SLOT_WHEEL0 + w)
                        });
                    }
                }
            }

            // Translucent: world far to near, then the car.
            pass.set_pipeline(&self.pipe_blend);
            let mut order = visible.clone();
            order.sort_by(|&a, &b| {
                let da = aabb_distance(f.cam_pos, self.sections[a].bbox_min, self.sections[a].bbox_max);
                let db = aabb_distance(f.cam_pos, self.sections[b].bbox_min, self.sections[b].bbox_max);
                db.total_cmp(&da)
            });
            for &i in &order {
                let m = &self.sections[i];
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Blend).then_some(SLOT_WORLD_BLEND)
                });
            }
            if f.car.is_some() {
                if let Some(m) = &self.car_body {
                    draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                        (r.blend == Blend::Blend).then_some(if r.texture == PAINT { SLOT_CAR_PAINT } else { SLOT_CAR })
                    });
                }
            }
            // Glows last: order does not matter for additive blending.
            pass.set_pipeline(&self.pipe_add);
            for &i in &visible {
                let m = &self.sections[i];
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Additive).then_some(SLOT_WORLD_BLEND)
                });
            }

            if !f.hud.is_empty() {
                pass.set_pipeline(&self.pipe_hud);
                pass.set_bind_group(0, &self.hud_bg, &[]);
                pass.set_vertex_buffer(0, self.hud_vbuf.slice(..hud_bytes.len() as u64));
                pass.draw(0..f.hud.len() as u32, 0..1);
                draws += 1;
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        self.stats_draws = draws;
    }
}

fn create_texture_bg(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    t: &PreparedTexture,
) -> Option<wgpu::BindGroup> {
    let (bw, bh) = t.format.block_dimensions();
    let block_bytes = t.format.block_copy_size(None)? as u64;
    let mut expected = 0u64;
    for i in 0..t.mips {
        let w = (t.width >> i).max(1).div_ceil(bw) as u64;
        let h = (t.height >> i).max(1).div_ceil(bh) as u64;
        expected += w * h * block_bytes;
    }
    if expected != t.data.len() as u64 {
        log::warn!("texture {:08X}: {} bytes, expected {expected}", t.hash, t.data.len());
        return None;
    }
    let max = gpu.device.limits().max_texture_dimension_2d;
    if t.width > max || t.height > max {
        return None;
    }
    let tex = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
            mip_level_count: t.mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: t.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &t.data,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    }))
}

fn draw_ranges(
    pass: &mut wgpu::RenderPass<'_>,
    m: &GpuMesh,
    textures: &HashMap<u32, wgpu::BindGroup>,
    white: &wgpu::BindGroup,
    model_bg: &wgpu::BindGroup,
    slot_for: impl Fn(&DrawRange) -> Option<u32>,
) -> u32 {
    let mut bound = false;
    let mut current_slot = u32::MAX;
    let mut n = 0;
    for r in &m.ranges {
        let Some(slot) = slot_for(r) else { continue };
        if !bound {
            pass.set_vertex_buffer(0, m.vbuf.slice(..));
            pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            bound = true;
        }
        if slot != current_slot {
            pass.set_bind_group(2, model_bg, &[slot * SLOT_STRIDE as u32]);
            current_slot = slot;
        }
        let tex = if r.texture == PAINT { white } else { textures.get(&r.texture).unwrap_or(white) };
        pass.set_bind_group(1, tex, &[]);
        pass.draw_indexed(r.first_index..r.first_index + r.index_count, 0, 0..1);
        n += 1;
    }
    n
}

fn frustum_planes(m: &Mat4) -> [Vec4; 6] {
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2]
}

fn aabb_in_frustum(planes: &[Vec4; 6], min: Vec3, max: Vec3) -> bool {
    planes.iter().all(|p| {
        let v = Vec3::new(
            if p.x >= 0.0 { max.x } else { min.x },
            if p.y >= 0.0 { max.y } else { min.y },
            if p.z >= 0.0 { max.z } else { min.z },
        );
        p.truncate().dot(v) + p.w >= 0.0
    })
}

fn aabb_distance(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    (p.clamp(min, max) - p).length()
}
