pub mod blend;
pub mod buffer;
pub mod pipeline;
pub mod tex;

pub mod canvas;

use std::num::NonZeroU32;

use blend::BlendingMode;
use buffer::CompositorBuffers;
use canvas::{CompositorAtlasTiling, CompositorCanvasTiling};
use pipeline::Pipeline;
use wgpu::{CommandEncoder, TextureView};

use crate::tex::TextureExt;

#[derive(Debug)]
pub struct ChunkTile {
    pub col: u32,
    pub row: u32,
    /// Texture index into an atlas.
    pub atlas_index: NonZeroU32,
    /// Mask texture index into an atlas.
    pub mask_atlas_index: Option<NonZeroU32>,
    /// Clipping texture index into an atlas.
    pub clip_atlas_index: Option<NonZeroU32>,
    pub layer_index: u32,
}

/// Compositing layer information.
#[derive(Debug)]
pub struct CompositeLayer {
    /// Opacity (0.0..=1.0) of the layer.
    pub opacity: f32,
    /// Blending mode of the layer.
    pub blend: BlendingMode,
    pub clipped: bool,
    pub hidden: bool,
    pub mask_hidden: bool,
}

/// Output target of a compositor pipeline.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    buffers: CompositorBuffers,
    atlas_texture: wgpu::Texture,
    /// Cached bind groups, rebuilt only when a bound buffer is reallocated.
    bind_groups: Option<BindGroups>,
}

/// Bind groups for a render pass, along with the layouts they were made for.
struct BindGroups {
    canvas_layout: wgpu::BindGroupLayout,
    blending_layout: wgpu::BindGroupLayout,
    canvas: wgpu::BindGroup,
    blending: wgpu::BindGroup,
}

impl BindGroups {
    fn new(
        device: &wgpu::Device,
        pipeline: &Pipeline,
        buffers: &CompositorBuffers,
        atlas_texture: &wgpu::Texture,
    ) -> Self {
        let canvas = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &pipeline.canvas_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.canvas.buffer().as_entire_binding(),
            }],
            label: Some("canvas_bind_group"),
        });

        // Storage buffers are bound whole, since they may hold more than the
        // current data. The shader only indexes valid entries.
        let atlas_view = atlas_texture.create_array_view();
        let blending = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &pipeline.blending_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffers.chunks.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffers.layers.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffers.silos.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffers.background.buffer().as_entire_binding(),
                },
            ],
            label: Some("blending_bind_group"),
        });

        Self {
            canvas_layout: pipeline.canvas_bind_group_layout.clone(),
            blending_layout: pipeline.blending_bind_group_layout.clone(),
            canvas,
            blending,
        }
    }

    fn matches(&self, pipeline: &Pipeline) -> bool {
        self.canvas_layout == pipeline.canvas_bind_group_layout
            && self.blending_layout == pipeline.blending_bind_group_layout
    }
}

impl Compositor {
    /// Create a new compositor target.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas: CompositorCanvasTiling,
        atlas_data: CompositorAtlasTiling,
        atlas_texture: wgpu::Texture,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            buffers: CompositorBuffers::new(device, queue, canvas, atlas_data),
            atlas_texture,
            bind_groups: None,
        }
    }

    pub fn load_layer_buffer(&mut self, layers: &[CompositeLayer]) {
        self.buffers.load_layer_buffer(layers);
    }

    pub fn load_chunk_buffer(&mut self, chunks_data: &[ChunkTile]) {
        self.buffers.load_chunk_buffer(chunks_data);
    }

    pub fn set_flipped(&mut self, horizontally: bool, vertically: bool) {
        if self
            .buffers
            .canvas
            .data_mut()
            .set_flipped(horizontally, vertically)
        {
            self.buffers.canvas.load_buffer(&self.queue);
        }
    }

    pub fn set_background(&mut self, bg: Option<[f32; 4]>) {
        let bg = bg.unwrap_or([0.0; 4]);
        if self.buffers.background.data() != &bg {
            *self.buffers.background.data_mut() = bg;
            self.buffers.background.load_buffer(&self.queue);
        }
    }

    /// Render composite layers using the compositor pipeline.
    pub fn render(&mut self, pipeline: &Pipeline, output_view: TextureView) {
        if self.buffers.is_empty() {
            // No layers to composite, skip rendering.
            return;
        }

        self.buffers.upload_chunks();

        // Rebuild the bind groups only if a bound buffer was reallocated or a
        // different pipeline layout is being used.
        let reallocated = self.buffers.take_reallocated();
        if reallocated
            || self
                .bind_groups
                .as_ref()
                .is_some_and(|groups| !groups.matches(pipeline))
        {
            self.bind_groups = None;
        }
        if self.bind_groups.is_none() {
            self.bind_groups = Some(BindGroups::new(
                &self.device,
                pipeline,
                &self.buffers,
                &self.atlas_texture,
            ));
        }

        self.queue.submit([{
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

            self.render_command(pipeline, &mut encoder, output_view);

            encoder.finish()
        }]);
    }

    fn render_command(
        &self,
        pipeline: &Pipeline,
        encoder: &mut CommandEncoder,
        output_view: TextureView,
    ) {
        let bind_groups = self
            .bind_groups
            .as_ref()
            .expect("bind groups are created before rendering");

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        // Finish and set the render pass's binding groups and data
        pass.set_pipeline(&pipeline.render_pipeline);
        pass.set_bind_group(0, &bind_groups.canvas, &[]);
        pass.set_bind_group(1, &bind_groups.blending, &[]);
        // One 4-vertex triangle strip per canvas tile; the shader derives the
        // quad corners and tile position from the vertex/instance indices.
        let canvas = self.buffers.canvas.data();
        pass.draw(0..4, 0..canvas.cols() * canvas.rows());
    }
}
