//! Draws a movie frame (YUV 4:2:0) to the screen, scaled to fit with black bars.

pub struct MovieView {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    planes: Option<Planes>,
}

struct Planes {
    size: (u32, u32),
    tex: [wgpu::Texture; 3],
    bind: wgpu::BindGroup,
}

impl MovieView {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
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
            label: Some("movie"),
            entries: &[
                tex(0),
                tex(1),
                tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("movie"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/movie.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("movie"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("movie"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("movie"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, layout, sampler, planes: None }
    }

    fn ensure(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.planes.as_ref().is_some_and(|p| p.size == (w, h)) {
            return;
        }
        let mk = |w: u32, h: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("movie plane"),
                size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let tex = [mk(w, h), mk(w.div_ceil(2), h.div_ceil(2)), mk(w.div_ceil(2), h.div_ceil(2))];
        let views: Vec<wgpu::TextureView> = tex.iter().map(|t| t.create_view(&Default::default())).collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("movie"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.planes = Some(Planes { size: (w, h), tex, bind });
    }

    /// Uploads `frame` (when given) and draws the current picture into `target`.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        frame: Option<&ug2_movie::Frame>,
    ) {
        if let Some(f) = frame {
            self.ensure(device, f.width, f.height);
            let p = self.planes.as_ref().unwrap();
            let (cw, ch) = (f.width.div_ceil(2), f.height.div_ceil(2));
            for (t, data, w, h) in [(&p.tex[0], &f.y, f.width, f.height), (&p.tex[1], &f.u, cw, ch), (&p.tex[2], &f.v, cw, ch)] {
                if data.len() < (w * h) as usize {
                    continue;
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("movie") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("movie"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some(p) = &self.planes {
                // Fit the picture's aspect inside the screen.
                let (sw, sh) = (size.0 as f32, size.1 as f32);
                let aspect = p.size.0 as f32 / p.size.1 as f32;
                let (w, h) = if sw / sh > aspect { (sh * aspect, sh) } else { (sw, sw / aspect) };
                pass.set_viewport((sw - w) * 0.5, (sh - h) * 0.5, w, h, 0.0, 1.0);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &p.bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
    }
}
