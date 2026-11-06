#[derive(Clone)]
pub struct Pipeline {
    pub blending_bind_group_layout: wgpu::BindGroupLayout,
    pub render_pipeline: wgpu::RenderPipeline,
    pub canvas_bind_group_layout: wgpu::BindGroupLayout,
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Pipeline {
    /// Create a new compositor pipeline.
    pub fn new(device: &wgpu::Device) -> Self {
        let canvas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("canvas_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ..buffer_entry(0, wgpu::BufferBindingType::Uniform)
                }],
            });

        // This bind group changes per composition run.
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let blending_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blending_group_layout"),
                entries: &[
                    // atlas texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        },
                        count: None,
                    },
                    // chunks
                    buffer_entry(1, storage),
                    // layers
                    buffer_entry(2, storage),
                    // silos
                    buffer_entry(3, storage),
                    // background color
                    buffer_entry(4, wgpu::BufferBindingType::Uniform),
                ],
            });

        // Loads the shader and creates the render pipeline.
        let render_pipeline = {
            let shader = device.create_shader_module(shader_load());

            let render_pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("render_pipeline_layout"),
                    bind_group_layouts: &[
                        Some(&canvas_bind_group_layout),
                        Some(&blending_bind_group_layout),
                    ],
                    immediate_size: 0,
                });

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                cache: None,
                label: Some("render_pipeline"),
                layout: Some(&render_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    entry_point: Some("vs_main"),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: crate::tex::TEX_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
            })
        };

        Self {
            canvas_bind_group_layout,
            blending_bind_group_layout,
            render_pipeline,
        }
    }
}

#[cfg(any(not(debug_assertions), target_arch = "wasm32"))]
fn shader_load() -> wgpu::ShaderModuleDescriptor<'static> {
    // In release mode, the final binary includes the file directly so that
    // the binary does not rely on the shader file being at a specific location.
    wgpu::include_wgsl!("shader.wgsl")
}

#[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
fn shader_load() -> wgpu::ShaderModuleDescriptor<'static> {
    // In debug mode, this reads directly from a file so that recompilation
    // will not be necessary in the event that only the shader file changes.
    wgpu::ShaderModuleDescriptor {
        label: Some("Dynamically loaded shader module"),
        source: wgpu::ShaderSource::Wgsl({
            use std::fs::OpenOptions;
            use std::io::Read;
            let mut file = OpenOptions::new()
                .read(true)
                .open("./libs/compositor/src/shader.wgsl")
                .unwrap();

            let mut buf = String::new();
            file.read_to_string(&mut buf).unwrap();
            buf.into()
        }),
    }
}
