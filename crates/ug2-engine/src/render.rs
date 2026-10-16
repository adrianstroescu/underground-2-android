//! wgpu renderer (Vulkan on Android). The world is pre-merged per scenery section and
//! texture, so a frame is a few thousand indexed draws.

use std::collections::HashMap;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::assets::{Blend, DrawRange, GpuVertex, MeshData, PreparedTexture, Role, PAINT};
use crate::cars::CarModel;
use crate::hud::HudVertex;
use crate::post::{PostFrame, PostFx, SCENE_FORMAT, TRAIL_FORMAT};

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub bc_textures: bool,
    /// Format of the HDR scene targets: 32-bit RG11B10 float when it can be rendered to
    /// with blending and 4x MSAA (half the bandwidth), else 64-bit RGBA16 float.
    pub hdr_format: wgpu::TextureFormat,
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
        // wgpu only trusts the adapter's own format capabilities (MSAA resolve of
        // RG11B10) with TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES.
        let small_hdr = adapter.features().contains(
            wgpu::Features::RG11B10UFLOAT_RENDERABLE | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        ) && {
            let f = adapter.get_texture_format_features(wgpu::TextureFormat::Rg11b10Ufloat);
            f.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
                && f.flags.contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
                && f.flags.sample_count_supported(4)
                && f.flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        };
        if small_hdr {
            features |= wgpu::Features::RG11B10UFLOAT_RENDERABLE | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }
        let hdr_format = if small_hdr { wgpu::TextureFormat::Rg11b10Ufloat } else { wgpu::TextureFormat::Rgba16Float };
        log::info!("HDR format {hdr_format:?}");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ug2"),
            required_features: features,
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))?;
        device.on_uncaptured_error(Box::new(|e| log::error!("wgpu: {e}")));
        Ok(Self { instance, adapter, device, queue, bc_textures, hdr_format })
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
    screen: [f32; 4],
    clip: [f32; 4],
    /// The player's headlight projector (world to clip) and its colour (w = 1 when on).
    headlight_vp: [[f32; 4]; 4],
    headlight_color: [f32; 4],
    /// Car lighting: the CAR effect's HarmonicCoeff (second-order SH in car space).
    sh: [[f32; 4]; 10],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniform {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
    params: [f32; 4],
    emissive: [f32; 4],
    /// x = light material id of the car paint.
    extra: [f32; 4],
}

/// One light material as the CAR effect's constants (see `ug2_formats::material`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMaterial {
    diffuse_min: [f32; 4],
    diffuse_range: [f32; 4],
    /// w = specular power.
    specular_min: [f32; 4],
    /// w = envmap power.
    specular_range: [f32; 4],
    envmap_min: [f32; 4],
    envmap_range: [f32; 4],
}

/// Material table size; id 0 is "none".
const MATERIALS: usize = 192;

/// HarmonicCoeff the PC game sent for a car in a City Hall race at night (apitrace).
/// The game builds these per car from the lights around it.
const CAR_SH: [[f32; 4]; 10] = [
    [0.2771666, 0.2771666, 0.2771666, 0.3640937],
    [-0.04462969, -0.04462969, -0.04462969, -0.05862677],
    [0.03838885, 0.03838885, 0.03838885, 0.05042865],
    [0.1475934, 0.1475934, 0.1475934, 0.1938827],
    [0.2480376, 0.2480376, 0.2480376, 0.3258289],
    [-0.00464404, -0.00464404, -0.00464404, -0.006100534],
    [-0.04610886, -0.04610886, -0.04610886, -0.06056986],
    [0.0758772, 0.0758772, 0.0758772, 0.09967436],
    [-0.02627384, -0.02627384, -0.02627384, -0.03451402],
    [0.4666717, 0.4666717, 0.4666717, 0.6130328],
];

/// Shading modes (`params.x` in mesh.wgsl).
const MODE_WORLD: f32 = 0.0;
const MODE_CAR: f32 = 1.0;
const MODE_GLASS: f32 = 2.0;
const MODE_UNLIT: f32 = 3.0;
const MODE_ROAD: f32 = 4.0;
/// Unlit sprite coloured by its vertices (glows).
const MODE_SPRITE: f32 = 5.0;

const SLOT_STRIDE: u64 = 256;
const SLOT_WORLD: u32 = 0;
const SLOT_WORLD_CUTOUT: u32 = 1;
const SLOT_WORLD_BLEND: u32 = 2;
const SLOT_SKY: u32 = 3;
const SLOT_GLOW: u32 = 4;
const SLOT_ROAD: u32 = 5;
/// Additive world ranges (neon, lit windows): drawn without fog, as in the PC game.
const SLOT_WORLD_ADD: u32 = 6;
const WORLD_SLOTS: u32 = 7;
/// Uniform slots per drawn car, at `WORLD_SLOTS + index * CAR_SLOTS`.
const CAR_SLOTS: u32 = 11;
const CAR_BODY: u32 = 0;
const CAR_PAINT: u32 = 1;
const CAR_GLASS: u32 = 2;
const CAR_HEAD: u32 = 3;
const CAR_BRAKE: u32 = 4;
const CAR_SHADOW: u32 = 5;
const CAR_WHEEL0: u32 = 6;
const CAR_PAINT_DETAIL: u32 = 10;

struct CarGpu {
    body: GpuMesh,
    wheel: Option<GpuMesh>,
    shadow: Option<GpuMesh>,
}

/// One car to draw this frame.
#[derive(Clone, Copy)]
pub struct CarDraw {
    /// Index returned by [`Renderer::add_car_model`].
    pub model: usize,
    pub transform: Mat4,
    pub wheels: [Mat4; 4],
    pub paint: [f32; 3],
    /// Replacement for a traffic car's skin texture.
    pub skin: Option<u32>,
    /// 0 = tail lights only, 1 = full brake lights.
    pub brake: f32,
    /// Window tint colour (white = stock).
    pub glass: [f32; 3],
    /// Light material of the body paint (`CARSKIN`), as a `DrawRange::material` id.
    pub paint_material: u16,
}

struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    ranges: Vec<DrawRange>,
    bbox_min: Vec3,
    bbox_max: Vec3,
    /// Scenery group set (0 = always drawn).
    group: u16,
    /// Scenery section number.
    section: u32,
}

/// A frontend vertex: screen pixels, texture coordinates, colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FeVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A run of frontend triangles sharing a texture (hash, 0 = untextured).
#[derive(Clone, Copy, Debug)]
pub struct FeBatch {
    pub texture: u32,
    pub first: u32,
    pub count: u32,
}

/// Everything the renderer needs to draw one frame.
pub struct FrameData<'a> {
    pub view: Mat4,
    pub fov_y: f32,
    /// Horizontal offset of the image in clip space (off-centre lens).
    pub lens_shift: f32,
    pub cam_pos: Vec3,
    pub cars: &'a [CarDraw],
    /// World-space additive quads (light glows), drawn with `glow_texture`.
    pub glows: &'a [GpuVertex],
    pub glow_texture: u32,
    pub draw_distance: f32,
    pub vertex_light_scale: f32,
    /// Height of the street plane to mirror the city in; `None` disables reflections.
    pub reflect_height: Option<f32>,
    /// Road reflection strength, 0..1.
    pub reflect_strength: f32,
    /// The player's headlight projector (world to clip) and colour, as the game's
    /// worldheadlight effect: the HEADLIGHTS pattern lights the world it falls on.
    pub headlight: Option<(Mat4, [f32; 3])>,
    pub hud: &'a [HudVertex],
    /// Frontend (FEng) quads, drawn under the HUD, and their batches.
    pub fe: &'a [FeVertex],
    pub fe_batches: &'a [FeBatch],
    /// The visual treatment of this frame (speed blur, colour, bloom, trails).
    pub post: PostFrame,
}

pub struct Renderer {
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    globals_layout: wgpu::BindGroupLayout,
    /// Globals for the mirrored pass (its bind group uses a dummy reflection texture).
    refl_globals_buf: wgpu::Buffer,
    refl_globals_bg: wgpu::BindGroup,
    refl_sampler: wgpu::Sampler,
    reflection: Option<Reflection>,
    /// 1x1 black texture bound where nothing better is loaded yet.
    dummy_view: wgpu::TextureView,
    /// The game's HEADLIGHTS beam pattern, once the world textures are in.
    headlight_view: Option<wgpu::TextureView>,
    /// The car light materials (`GpuMaterial` x MATERIALS).
    material_buf: wgpu::Buffer,
    pipe_refl_opaque: wgpu::RenderPipeline,
    pipe_refl_sky: wgpu::RenderPipeline,
    pipe_refl_add: wgpu::RenderPipeline,
    model_buf: wgpu::Buffer,
    model_bg: wgpu::BindGroup,
    model_layout: wgpu::BindGroupLayout,
    model_slots: u32,
    glow_vbuf: wgpu::Buffer,
    glow_capacity: u64,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipe_opaque: wgpu::RenderPipeline,
    pipe_blend: wgpu::RenderPipeline,
    pipe_add: wgpu::RenderPipeline,
    pipe_sky: wgpu::RenderPipeline,
    pipe_hud: wgpu::RenderPipeline,
    /// Lamp and car-light glows, into the glow target.
    pipe_glow: wgpu::RenderPipeline,
    pipe_fe: wgpu::RenderPipeline,
    fe_vbuf: wgpu::Buffer,
    fe_capacity: u64,
    /// Frontend textures (clamped sampling), by name hash.
    fe_textures: HashMap<u32, wgpu::BindGroup>,
    fe_sampler: wgpu::Sampler,
    hud_buf: wgpu::Buffer,
    hud_bg: wgpu::BindGroup,
    hud_vbuf: wgpu::Buffer,
    hud_capacity: u64,
    textures: HashMap<u32, wgpu::BindGroup>,
    white: wgpu::BindGroup,
    sections: Vec<GpuMesh>,
    /// Which scenery group sets are drawn (race barriers); index 0 is unused.
    groups_on: Vec<bool>,
    /// Which scenery sections are visible from where the camera is (empty: all).
    sections_on: Vec<bool>,
    sky: Vec<GpuMesh>,
    car_models: Vec<CarGpu>,
    targets: Option<Targets>,
    post: PostFx,
    pub stats_draws: u32,
}

/// Half-resolution target the mirrored city is drawn into.
struct Reflection {
    width: u32,
    height: u32,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}

struct Targets {
    width: u32,
    height: u32,
    depth: wgpu::TextureView,
    msaa: Option<wgpu::TextureView>,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// D3DRS_FOGCOLOR of the PC game at night (0xFF5D5F5A, read from an apitrace capture).
pub const FOG_COLOR: [f32; 4] = [93.0 / 255.0, 95.0 / 255.0, 90.0 / 255.0, 1.0];
/// The world effect's FogValue (250, 1/240, 0.6): linear in view depth from 10 m, at
/// most 40 % from 106 m on.
const FOG: [f32; 4] = [10.0, 250.0, 0.4, 0.0];

impl Renderer {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, msaa: u32) -> Self {
        let device = &gpu.device;
        // The scene is 8-bit as on the PC (A8R8G8B8); the post passes write `format`.
        let scene_format = SCENE_FORMAT;
        let sample_count = if msaa > 1
            && gpu.adapter.get_texture_format_features(scene_format).flags.sample_count_supported(msaa)
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
            entries: &[
                uniform_entry(0, false, std::mem::size_of::<Globals>() as u64),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform_entry(4, false, (std::mem::size_of::<GpuMaterial>() * MATERIALS) as u64),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
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
        let refl_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no reflection"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: scene_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let material_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("materials"),
            size: (std::mem::size_of::<GpuMaterial>() * MATERIALS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg =
            globals_bind_group(device, &globals_layout, &globals_buf, &dummy_view, &refl_sampler, &dummy_view, &material_buf);
        let refl_globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reflection globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let refl_globals_bg =
            globals_bind_group(device, &globals_layout, &refl_globals_buf, &dummy_view, &refl_sampler, &dummy_view, &material_buf);
        let model_slots = WORLD_SLOTS + CAR_SLOTS * 8;
        let (model_buf, model_bg) = create_model_buffer(device, &model_layout, model_slots);
        let glow_capacity = 64 * 1024;
        let glow_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glows"),
            size: glow_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
        // Main-pass pipelines have two targets, the scene and the glow target; only
        // `pipe_glow` writes the second. The mirrored pass has the scene only.
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let mesh_pipeline_t = |label: &str,
                               entry: &str,
                               targets: &[Option<wgpu::ColorTargetState>],
                               depth_write: bool,
                               depth_test: bool,
                               samples: u32| {
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
                multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview: None,
                cache: None,
            })
        };
        let no_glow = Some(wgpu::ColorTargetState { format: scene_format, blend: None, write_mask: wgpu::ColorWrites::empty() });
        let mesh_pipeline_n = |label: &str, blend: Option<wgpu::BlendState>, depth_write: bool, depth_test: bool, samples: u32| {
            let scene = Some(wgpu::ColorTargetState { format: scene_format, blend, write_mask: wgpu::ColorWrites::ALL });
            if samples == sample_count && !label.starts_with("refl") {
                mesh_pipeline_t(label, "fs_main", &[scene, no_glow.clone()], depth_write, depth_test, samples)
            } else {
                mesh_pipeline_t(label, "fs_main", &[scene], depth_write, depth_test, samples)
            }
        };
        let mesh_pipeline = |label: &str, blend: Option<wgpu::BlendState>, depth_write: bool, depth_test: bool| {
            mesh_pipeline_n(label, blend, depth_write, depth_test, sample_count)
        };
        // SRCALPHA/ONE into the glow target, RGB only, depth tested (as the PC game).
        let pipe_glow = mesh_pipeline_t(
            "glow",
            "fs_glow",
            &[
                Some(wgpu::ColorTargetState { format: scene_format, blend: None, write_mask: wgpu::ColorWrites::empty() }),
                Some(wgpu::ColorTargetState {
                    format: scene_format,
                    blend: Some(wgpu::BlendState { color: additive, alpha: additive }),
                    write_mask: wgpu::ColorWrites::COLOR,
                }),
            ],
            false,
            true,
            sample_count,
        );
        let pipe_opaque = mesh_pipeline("opaque", None, true, true);
        let pipe_blend = mesh_pipeline("blend", Some(wgpu::BlendState::ALPHA_BLENDING), false, true);
        let pipe_add = mesh_pipeline("additive", Some(wgpu::BlendState { color: additive, alpha: additive }), false, true);
        let pipe_sky = mesh_pipeline("sky", Some(wgpu::BlendState::ALPHA_BLENDING), false, false);
        // The mirrored pass renders single-sampled at half resolution.
        let pipe_refl_opaque = mesh_pipeline_n("refl opaque", None, true, true, 1);
        let pipe_refl_sky = mesh_pipeline_n("refl sky", Some(wgpu::BlendState::ALPHA_BLENDING), false, false, 1);
        let pipe_refl_add = mesh_pipeline_n("refl additive", Some(wgpu::BlendState { color: additive, alpha: additive }), false, true, 1);

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
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &hud_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState { format: TRAIL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() }),
                ],
            }),
            multiview: None,
            cache: None,
        });
        let fe_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fe"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fe.wgsl").into()),
        });
        let fe_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fe"),
            bind_group_layouts: &[&hud_layout_bg, &tex_layout],
            push_constant_ranges: &[],
        });
        let fe_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let pipe_fe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fe"),
            layout: Some(&fe_pl),
            vertex: wgpu::VertexState {
                module: &fe_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<FeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &fe_attrs,
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fe_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: TRAIL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() }),
                ],
            }),
            multiview: None,
            cache: None,
        });
        let fe_capacity = 64 * 1024;
        let fe_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fe vertices"),
            size: fe_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fe_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fe"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
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
        let white = create_texture_bg(gpu, &tex_layout, &sampler, &white_tex).expect("white texture").0;

        Self {
            format,
            sample_count,
            globals_buf,
            globals_bg,
            globals_layout,
            refl_globals_buf,
            refl_globals_bg,
            refl_sampler,
            reflection: None,
            dummy_view,
            headlight_view: None,
            material_buf,
            pipe_refl_opaque,
            pipe_refl_sky,
            pipe_refl_add,
            model_buf,
            model_bg,
            model_layout,
            model_slots,
            glow_vbuf,
            glow_capacity,
            tex_layout,
            sampler,
            pipe_opaque,
            pipe_blend,
            pipe_add,
            pipe_sky,
            pipe_hud,
            pipe_glow,
            pipe_fe,
            fe_vbuf,
            fe_capacity,
            fe_textures: HashMap::new(),
            fe_sampler,
            hud_buf,
            hud_bg,
            hud_vbuf,
            hud_capacity,
            textures: HashMap::new(),
            white,
            sections: Vec::new(),
            groups_on: Vec::new(),
            sections_on: Vec::new(),
            sky: Vec::new(),
            car_models: Vec::new(),
            targets: None,
            post: PostFx::new(device, &gpu.queue, format, sample_count, DEPTH),
            stats_draws: 0,
        }
    }

    /// A frontend texture (menus, fonts), kept for the whole session.
    pub fn upload_fe_texture(&mut self, gpu: &Gpu, t: &PreparedTexture) {
        if self.fe_textures.contains_key(&t.hash) {
            return;
        }
        if let Some((bg, _)) = create_texture_bg(gpu, &self.tex_layout, &self.fe_sampler, t) {
            self.fe_textures.insert(t.hash, bg);
        }
    }

    pub fn upload_texture(&mut self, gpu: &Gpu, t: &PreparedTexture) {
        if self.textures.contains_key(&t.hash) {
            return;
        }
        if let Some((bg, view)) = create_texture_bg(gpu, &self.tex_layout, &self.sampler, t) {
            self.textures.insert(t.hash, bg);
            if t.hash == ug2_formats::bin_hash("HEADLIGHTS") {
                self.headlight_view = Some(view);
                self.rebind_globals(gpu);
            }
        }
    }

    /// Rebuilds the globals bind groups after the reflection target or the headlight
    /// pattern changed.
    fn rebind_globals(&mut self, gpu: &Gpu) {
        let head = self.headlight_view.as_ref().unwrap_or(&self.dummy_view);
        let refl = self.reflection.as_ref().map_or(&self.dummy_view, |r| &r.color);
        let mats = &self.material_buf;
        self.globals_bg =
            globals_bind_group(&gpu.device, &self.globals_layout, &self.globals_buf, refl, &self.refl_sampler, head, mats);
        self.refl_globals_bg = globals_bind_group(
            &gpu.device,
            &self.globals_layout,
            &self.refl_globals_buf,
            &self.dummy_view,
            &self.refl_sampler,
            head,
            mats,
        );
    }

    /// Uploads the car light materials; `DrawRange::material` n is `materials[n - 1]`.
    pub fn set_materials(&mut self, gpu: &Gpu, materials: &[ug2_formats::material::LightMaterial]) {
        let mut table = vec![GpuMaterial::zeroed(); MATERIALS];
        for (i, m) in materials.iter().take(MATERIALS - 1).enumerate() {
            let v3 = |a: [f32; 3], w: f32| [a[0], a[1], a[2], w];
            table[i + 1] = GpuMaterial {
                diffuse_min: m.diffuse_min,
                diffuse_range: m.diffuse_range,
                specular_min: v3(m.specular_min, m.specular_power),
                specular_range: v3(m.specular_range, m.envmap_power),
                envmap_min: v3(m.envmap_min, 0.0),
                envmap_range: v3(m.envmap_range, 0.0),
            };
        }
        gpu.queue.write_buffer(&self.material_buf, 0, bytemuck::cast_slice(&table));
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
        Some(GpuMesh { vbuf, ibuf, ranges: m.ranges.clone(), bbox_min: m.bbox_min, bbox_max: m.bbox_max, group: 0, section: 0 })
    }

    pub fn add_section(&mut self, gpu: &Gpu, mesh: &MeshData, sky: bool, group: u16, section: u32) {
        if let Some(mut m) = Self::upload_mesh(gpu, mesh) {
            m.group = group;
            m.section = section;
            if sky {
                self.sky.push(m);
            } else {
                self.sections.push(m);
            }
        }
    }

    /// Switches scenery group sets on or off (`on[set]`); sets past the end are off.
    pub fn set_groups_on(&mut self, on: Vec<bool>) {
        self.groups_on = on;
    }

    /// Draws only the given scenery sections (`on[number]`, numbers past the end are
    /// drawn); empty draws every section.
    pub fn set_sections_on(&mut self, on: Vec<bool>) {
        self.sections_on = on;
    }

    fn section_on(&self, m: &GpuMesh) -> bool {
        (m.group == 0 || self.groups_on.get(m.group as usize).copied().unwrap_or(false))
            && self.sections_on.get(m.section as usize).copied().unwrap_or(true)
    }

    /// Drops the world (sections, sky and the given textures) before another region loads.
    pub fn clear_world(&mut self, textures: &[u32]) {
        self.sections.clear();
        self.sections_on.clear();
        self.sky.clear();
        for h in textures {
            self.textures.remove(h);
        }
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Uploads a car model; [`CarDraw::model`] refers to the returned index.
    pub fn add_car_model(&mut self, gpu: &Gpu, car: &CarModel) -> Option<usize> {
        let body = Self::upload_mesh(gpu, &car.body)?;
        let wheel = car.wheel.as_ref().and_then(|w| Self::upload_mesh(gpu, w));
        let shadow = car.shadow.as_ref().and_then(|s| Self::upload_mesh(gpu, s));
        self.car_models.push(CarGpu { body, wheel, shadow });
        Some(self.car_models.len() - 1)
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
        let msaa = (self.sample_count > 1).then(|| mk(SCENE_FORMAT, self.sample_count, "msaa"));
        self.targets = Some(Targets { width, height, depth, msaa });
    }

    /// Writes globals and model slots; returns the view-projection and, when reflections
    /// are on, the mirrored view-projection.
    fn write_uniforms(&mut self, gpu: &Gpu, f: &FrameData, width: u32, height: u32) -> (Mat4, Option<Mat4>) {
        let aspect = width as f32 / height.max(1) as f32;
        let proj = Mat4::from_translation(Vec3::new(f.lens_shift, 0.0, 0.0)) * Mat4::perspective_rh(f.fov_y, aspect, 0.3, 4000.0);
        let view_proj = proj * f.view;
        let light = Vec3::new(0.35, -0.45, 0.82).normalize();
        let strength = if f.reflect_height.is_some() { f.reflect_strength } else { 0.0 };
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam_pos: f.cam_pos.extend(1.0).into(),
            fog_color: FOG_COLOR,
            fog: FOG,
            light_dir: light.extend(0.0).into(),
            ambient: [0.30, 0.33, 0.42, 1.0],
            screen: [width as f32, height as f32, strength, 0.0],
            clip: [0.0, 0.0, 0.0, 1.0],
            headlight_vp: f.headlight.map_or(Mat4::ZERO, |h| h.0).to_cols_array_2d(),
            headlight_color: f.headlight.map_or([0.0; 4], |h| [h.1[0], h.1[1], h.1[2], 1.0]),
            sh: CAR_SH,
        };
        gpu.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        // Mirror the camera in the street plane z = h and clip what is below it.
        let refl_view_proj = f.reflect_height.map(|h| {
            let mirror = Mat4::from_translation(Vec3::Z * h)
                * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0))
                * Mat4::from_translation(-Vec3::Z * h);
            let vp = proj * f.view * mirror;
            let cam = mirror.transform_point3(f.cam_pos);
            let rg = Globals {
                view_proj: vp.to_cols_array_2d(),
                cam_pos: cam.extend(1.0).into(),
                clip: [0.0, 0.0, 1.0, -(h - 0.05)],
                ..globals
            };
            gpu.queue.write_buffer(&self.refl_globals_buf, 0, bytemuck::bytes_of(&rg));
            vp
        });

        let vs = f.vertex_light_scale;
        let slot = |model: Mat4, tint: [f32; 4], params: [f32; 4], emissive: [f32; 4]| ModelUniform {
            model: model.to_cols_array_2d(),
            tint,
            params,
            emissive,
            extra: [0.0; 4],
        };
        let one = [1.0, 1.0, 1.0, 1.0];
        let none = [0.0; 4];
        let mut slots = Vec::with_capacity((WORLD_SLOTS + CAR_SLOTS * f.cars.len() as u32) as usize);
        slots.push(slot(Mat4::IDENTITY, one, [MODE_WORLD, 0.0, 1.0, vs], none));
        slots.push(slot(Mat4::IDENTITY, one, [MODE_WORLD, 0.5, 1.0, vs], none));
        slots.push(slot(Mat4::IDENTITY, one, [MODE_WORLD, 0.01, 1.0, vs], none));
        // The dome (centred on the origin by the loader) follows the camera, shrunk so it
        // stays inside the far plane; it is drawn first without depth, so only direction matters.
        let sky_m = Mat4::from_translation(f.cam_pos - Vec3::Z * 60.0) * Mat4::from_scale(Vec3::splat(0.3));
        slots.push(slot(sky_m, one, [MODE_WORLD, 0.0, 0.0, vs], none));
        // Glows: additive, so unfogged (the PC game turns fog off for additive draws).
        slots.push(slot(Mat4::IDENTITY, one, [MODE_SPRITE, 0.0, 0.0, 1.0], none));
        slots.push(slot(Mat4::IDENTITY, one, [MODE_ROAD, 0.0, 1.0, vs], none));
        slots.push(slot(Mat4::IDENTITY, one, [MODE_WORLD, 0.01, 0.0, vs], none));
        for c in f.cars {
            let first = slots.len();
            let t = c.transform;
            let p = c.paint;
            let lit = [MODE_CAR, 0.5, 1.0, 1.0];
            slots.push(slot(t, one, lit, none));
            slots.push(slot(t, [p[0], p[1], p[2], 1.0], lit, none));
            let g = c.glass;
            slots.push(slot(t, [g[0], g[1], g[2], 1.0], [MODE_GLASS, 0.0, 1.0, 1.0], none));
            // Headlights are always on (the city is at night); brake lights glow dimly as
            // tail lights and fully when braking.
            slots.push(slot(t, one, lit, [1.3, 1.3, 1.2, 0.0]));
            let b = 0.6 + 2.4 * c.brake.clamp(0.0, 1.0);
            slots.push(slot(t, one, lit, [b, b * 0.5, b * 0.5, 0.0]));
            slots.push(slot(t, [1.0, 1.0, 1.0, 0.85], [MODE_UNLIT, 0.0, 1.0, 1.0], none));
            for w in &c.wheels {
                slots.push(slot(*w, one, lit, none));
            }
            // emissive.w flags "blend paint and texture by the texture's alpha".
            slots.push(slot(t, [p[0], p[1], p[2], 1.0], lit, [0.0, 0.0, 0.0, 1.0]));
            for s in &mut slots[first..] {
                s.extra[0] = c.paint_material as f32;
            }
        }
        if slots.len() as u32 > self.model_slots {
            self.model_slots = (slots.len() as u32).next_power_of_two();
            let (buf, bg) = create_model_buffer(&gpu.device, &self.model_layout, self.model_slots);
            self.model_buf = buf;
            self.model_bg = bg;
        }
        let mut bytes = vec![0u8; SLOT_STRIDE as usize * slots.len()];
        for (i, s) in slots.iter().enumerate() {
            let o = i * SLOT_STRIDE as usize;
            bytes[o..o + std::mem::size_of::<ModelUniform>()].copy_from_slice(bytemuck::bytes_of(s));
        }
        gpu.queue.write_buffer(&self.model_buf, 0, &bytes);
        (view_proj, refl_view_proj)
    }

    /// The road reflection target; rebinds the main globals to sample it.
    fn ensure_reflection(&mut self, gpu: &Gpu) {
        // The PC game renders the mirrored city into a 320x240 target at any screen
        // size (seen in an apitrace capture), which is what softens it.
        let (w, h) = (320, 240);
        if self.reflection.as_ref().is_some_and(|r| r.width == w && r.height == h) {
            return;
        }
        let mk = |format, usage, label| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color = mk(SCENE_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, "reflection");
        let depth = mk(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, "reflection depth");
        self.reflection = Some(Reflection { width: w, height: h, color, depth });
        self.rebind_globals(gpu);
    }

    /// Renders a frame into `target` (a swapchain image or an offscreen texture).
    pub fn render(&mut self, gpu: &Gpu, target: &wgpu::TextureView, width: u32, height: u32, f: &FrameData) {
        self.ensure_targets(gpu, width, height);
        self.post.ensure(&gpu.device, width, height);
        if f.reflect_height.is_some() {
            self.ensure_reflection(gpu);
        }
        let (view_proj, refl_view_proj) = self.write_uniforms(gpu, f, width, height);
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
        let fe_bytes: &[u8] = bytemuck::cast_slice(f.fe);
        if fe_bytes.len() as u64 > self.fe_capacity {
            self.fe_capacity = (fe_bytes.len() as u64).next_power_of_two();
            self.fe_vbuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fe vertices"),
                size: self.fe_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !fe_bytes.is_empty() {
            gpu.queue.write_buffer(&self.fe_vbuf, 0, fe_bytes);
        }
        let glow_bytes: &[u8] = bytemuck::cast_slice(f.glows);
        if glow_bytes.len() as u64 > self.glow_capacity {
            self.glow_capacity = (glow_bytes.len() as u64).next_power_of_two();
            self.glow_vbuf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("glows"),
                size: self.glow_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !glow_bytes.is_empty() {
            gpu.queue.write_buffer(&self.glow_vbuf, 0, glow_bytes);
        }

        let planes = frustum_planes(&view_proj);
        let visible: Vec<usize> = (0..self.sections.len())
            .filter(|&i| {
                let m = &self.sections[i];
                self.section_on(m)
                    && aabb_distance(f.cam_pos, m.bbox_min, m.bbox_max) < f.draw_distance
                    && aabb_in_frustum(&planes, m.bbox_min, m.bbox_max)
            })
            .collect();

        let targets = self.targets.as_ref().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let mut draws = 0u32;
        // The city mirrored in the street, at half resolution and reduced range.
        let car_list: Vec<(u32, &CarDraw, &CarGpu)> = f
            .cars
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((WORLD_SLOTS + i as u32 * CAR_SLOTS, c, self.car_models.get(c.model)?)))
            .filter(|(_, c, _)| c.transform.w_axis.truncate().distance(f.cam_pos) < f.draw_distance)
            .collect();
        if let (Some(rvp), Some(refl)) = (refl_view_proj, &self.reflection) {
            let rplanes = frustum_planes(&rvp);
            let range = f.draw_distance.min(180.0);
            let c = FOG_COLOR;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reflection"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &refl.color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &refl.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.refl_globals_bg, &[]);
            pass.set_pipeline(&self.pipe_refl_sky);
            for m in &self.sky {
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend != Blend::Additive).then_some((SLOT_SKY, r.texture))
                });
            }
            pass.set_pipeline(&self.pipe_refl_opaque);
            let near: Vec<usize> = (0..self.sections.len())
                .filter(|&i| {
                    let m = &self.sections[i];
                    self.section_on(m)
                        && aabb_distance(f.cam_pos, m.bbox_min, m.bbox_max) < range
                        && aabb_in_frustum(&rplanes, m.bbox_min, m.bbox_max)
                })
                .collect();
            for &i in &near {
                // Opaque only: foliage and fences barely show in a blurred reflection.
                draws += draw_ranges(&mut pass, &self.sections[i], &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Opaque).then_some((SLOT_WORLD, r.texture))
                });
            }
            for &(base, c, g) in car_list.iter().filter(|(_, c, _)| c.transform.w_axis.truncate().distance(f.cam_pos) < 60.0) {
                draws += draw_ranges(&mut pass, &g.body, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Opaque).then(|| car_slot(base, c, r))
                });
            }
            pass.set_pipeline(&self.pipe_refl_add);
            for &i in &near {
                draws += draw_ranges(&mut pass, &self.sections[i], &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Additive).then_some((SLOT_WORLD_ADD, r.texture))
                });
            }
            if !f.glows.is_empty() {
                pass.set_bind_group(2, &self.model_bg, &[SLOT_GLOW * SLOT_STRIDE as u32]);
                pass.set_bind_group(1, self.textures.get(&f.glow_texture).unwrap_or(&self.white), &[]);
                pass.set_vertex_buffer(0, self.glow_vbuf.slice(..glow_bytes.len() as u64));
                pass.draw(0..f.glows.len() as u32, 0..1);
                draws += 1;
            }
        }
        let blur = f.post.speed_blur;
        let c = FOG_COLOR;
        let clear = wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 };
        let glow_count = f.glows.len() as u32;
        {
            // World: sky, opaque, translucent, additive. As on the PC, the cars follow
            // after the speed blur, so they stay sharp.
            let world = self.post.world_target();
            let (view, resolve) = match &targets.msaa {
                Some(ms) => (ms, Some(world)),
                None => (world, None),
            };
            let (glow_view, glow_resolve) = self.post.glow_target();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: resolve,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: glow_view,
                        resolve_target: if blur.is_some() { None } else { glow_resolve },
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: if glow_resolve.is_some() || blur.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        // The car pass after the speed blur draws against the world's depth.
                        store: if blur.is_some() { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
                    }),
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
                    (r.blend != Blend::Additive).then_some((SLOT_SKY, r.texture))
                });
            }

            // Opaque + cutout world.
            pass.set_pipeline(&self.pipe_opaque);
            for &i in &visible {
                let m = &self.sections[i];
                let road = if refl_view_proj.is_some() { SLOT_ROAD } else { SLOT_WORLD };
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| match r.blend {
                    Blend::Opaque if r.role == Role::Road => Some((road, r.texture)),
                    Blend::Opaque => Some((SLOT_WORLD, r.texture)),
                    Blend::Cutout => Some((SLOT_WORLD_CUTOUT, r.texture)),
                    Blend::Blend | Blend::Additive => None,
                });
            }
            // Translucent world, far to near.
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
                    (r.blend == Blend::Blend).then_some((SLOT_WORLD_BLEND, r.texture))
                });
            }
            // Additive world (neon, lamps).
            pass.set_pipeline(&self.pipe_add);
            for &i in &visible {
                let m = &self.sections[i];
                draws += draw_ranges(&mut pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                    (r.blend == Blend::Additive).then_some((SLOT_WORLD_ADD, r.texture))
                });
            }
            if blur.is_none() {
                draws += self.draw_cars_and_glows(&mut pass, f, &car_list, glow_count, glow_bytes.len() as u64);
            }
        }
        if let Some(b) = &blur {
            self.post.run_speed_blur(&gpu.queue, &mut encoder, b);
            let cars = self.post.car_target();
            let (view, resolve) = match &targets.msaa {
                Some(ms) => (ms, Some(cars)),
                None => (cars, None),
            };
            let (glow_view, glow_resolve) = self.post.glow_target();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cars"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: resolve,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(clear),
                            store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: glow_view,
                        resolve_target: glow_resolve,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: if glow_resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // The world with its blurred copy over it, then the cars.
            pass.set_pipeline(&self.post.blend_blur);
            pass.set_bind_group(0, self.post.blend_bind_group(), &[]);
            pass.draw(0..3, 0..1);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            draws += 1 + self.draw_cars_and_glows(&mut pass, f, &car_list, glow_count, glow_bytes.len() as u64);
        }
        // Colour treatment, bloom and light trails into the screen image, then the HUD.
        let hud_count = f.hud.len() as u32;
        let (pipe_hud, hud_bg, hud_vbuf) = (&self.pipe_hud, &self.hud_bg, &self.hud_vbuf);
        let (pipe_fe, fe_vbuf, fe_textures, white) = (&self.pipe_fe, &self.fe_vbuf, &self.fe_textures, &self.white);
        let fe_batches = f.fe_batches;
        self.post.run(&gpu.queue, &mut encoder, target, &f.post, blur.is_some(), |pass| {
            if !fe_batches.is_empty() {
                pass.set_pipeline(pipe_fe);
                pass.set_bind_group(0, hud_bg, &[]);
                pass.set_vertex_buffer(0, fe_vbuf.slice(..fe_bytes.len() as u64));
                for b in fe_batches {
                    let tex = if b.texture == 0 { white } else { fe_textures.get(&b.texture).unwrap_or(white) };
                    pass.set_bind_group(1, tex, &[]);
                    pass.draw(b.first..b.first + b.count, 0..1);
                }
            }
            if hud_count > 0 {
                pass.set_pipeline(pipe_hud);
                pass.set_bind_group(0, hud_bg, &[]);
                pass.set_vertex_buffer(0, hud_vbuf.slice(..hud_bytes.len() as u64));
                pass.draw(0..hud_count, 0..1);
            }
        });
        draws += if blur.is_some() { 10 } else { 8 };
        gpu.queue.submit(Some(encoder.finish()));
        self.stats_draws = draws;
    }
}

impl Renderer {
    /// Cars (bodies, wheels, shadows, glass and light lenses), then the glows into the
    /// glow target. Order as on the PC: after the world and its speed blur.
    fn draw_cars_and_glows(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        f: &FrameData,
        cars: &[(u32, &CarDraw, &CarGpu)],
        glow_count: u32,
        glow_bytes: u64,
    ) -> u32 {
        let mut draws = 0;
        pass.set_pipeline(&self.pipe_opaque);
        for &(base, c, g) in cars {
            let solid = |r: &DrawRange| matches!(r.blend, Blend::Opaque | Blend::Cutout);
            draws += draw_ranges(pass, &g.body, &self.textures, &self.white, &self.model_bg, |r| {
                solid(r).then(|| car_slot(base, c, r))
            });
            if let Some(m) = &g.wheel {
                for w in 0..4 {
                    draws += draw_ranges(pass, m, &self.textures, &self.white, &self.model_bg, |r| {
                        solid(r).then_some((base + CAR_WHEEL0 + w, r.texture))
                    });
                }
            }
        }
        // Car shadows, then glass, far to near.
        pass.set_pipeline(&self.pipe_blend);
        let mut far_first = cars.to_vec();
        far_first.sort_by(|a, b| {
            let da = a.1.transform.w_axis.truncate().distance_squared(f.cam_pos);
            let db = b.1.transform.w_axis.truncate().distance_squared(f.cam_pos);
            db.total_cmp(&da)
        });
        for &(base, _, g) in &far_first {
            if let Some(m) = &g.shadow {
                draws += draw_ranges(pass, m, &self.textures, &self.white, &self.model_bg, |r| Some((base + CAR_SHADOW, r.texture)));
            }
        }
        for &(base, c, g) in &far_first {
            draws += draw_ranges(pass, &g.body, &self.textures, &self.white, &self.model_bg, |r| {
                (r.blend == Blend::Blend).then(|| car_slot(base, c, r))
            });
        }
        // Head and brake light lenses (additive textures).
        pass.set_pipeline(&self.pipe_add);
        for &(base, c, g) in &far_first {
            draws += draw_ranges(pass, &g.body, &self.textures, &self.white, &self.model_bg, |r| {
                (r.blend == Blend::Additive).then(|| car_slot(base, c, r))
            });
        }
        if glow_count > 0 {
            pass.set_pipeline(&self.pipe_glow);
            pass.set_bind_group(2, &self.model_bg, &[SLOT_GLOW * SLOT_STRIDE as u32]);
            pass.set_bind_group(1, self.textures.get(&f.glow_texture).unwrap_or(&self.white), &[]);
            pass.set_vertex_buffer(0, self.glow_vbuf.slice(..glow_bytes));
            pass.draw(0..glow_count, 0..1);
            draws += 1;
        }
        draws
    }
}

fn create_texture_bg(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    t: &PreparedTexture,
) -> Option<(wgpu::BindGroup, wgpu::TextureView)> {
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
    let bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    Some((bg, view))
}

/// Uniform slot and texture for a car body range.
fn car_slot(base: u32, c: &CarDraw, r: &DrawRange) -> (u32, u32) {
    match r.role {
        Role::Paint => (base + CAR_PAINT, PAINT),
        Role::PaintDetail => (base + CAR_PAINT_DETAIL, r.texture),
        Role::Skin => (base + CAR_BODY, c.skin.unwrap_or(r.texture)),
        Role::Glass => (base + CAR_GLASS, r.texture),
        Role::Headlight => (base + CAR_HEAD, r.texture),
        Role::Brakelight => (base + CAR_BRAKE, r.texture),
        Role::Shadow => (base + CAR_SHADOW, r.texture),
        Role::Normal | Role::Road => (base + CAR_BODY, r.texture),
    }
}

fn globals_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buf: &wgpu::Buffer,
    reflection: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    headlight: &wgpu::TextureView,
    materials: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(reflection) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(headlight) },
            wgpu::BindGroupEntry { binding: 4, resource: materials.as_entire_binding() },
        ],
    })
}

fn create_model_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, slots: u32) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("models"),
        size: SLOT_STRIDE * slots as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("models"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buf,
                offset: 0,
                size: NonZeroU64::new(std::mem::size_of::<ModelUniform>() as u64),
            }),
        }],
    });
    (buf, bg)
}

/// Draws the ranges `slot_for` selects, returning `(uniform slot, texture hash)`.
fn draw_ranges(
    pass: &mut wgpu::RenderPass<'_>,
    m: &GpuMesh,
    textures: &HashMap<u32, wgpu::BindGroup>,
    white: &wgpu::BindGroup,
    model_bg: &wgpu::BindGroup,
    slot_for: impl Fn(&DrawRange) -> Option<(u32, u32)>,
) -> u32 {
    let mut bound = false;
    let mut current_slot = u32::MAX;
    let mut n = 0;
    for r in &m.ranges {
        let Some((slot, texture)) = slot_for(r) else { continue };
        if !bound {
            pass.set_vertex_buffer(0, m.vbuf.slice(..));
            pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            bound = true;
        }
        if slot != current_slot {
            pass.set_bind_group(2, model_bg, &[slot * SLOT_STRIDE as u32]);
            current_slot = slot;
        }
        let tex = if texture == PAINT { white } else { textures.get(&texture).unwrap_or(white) };
        pass.set_bind_group(1, tex, &[]);
        // The instance index carries the range's light material to the shader.
        let mat = r.material as u32;
        pass.draw_indexed(r.first_index..r.first_index + r.index_count, 0, mat..mat + 1);
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
