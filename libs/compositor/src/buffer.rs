use std::num::NonZeroU32;

use wgpu::util::DeviceExt;

use crate::{
    ChunkTile, CompositeLayer,
    canvas::{
        ChunkData, ChunkFlags, ChunkSilo, CompositorAtlasTiling, CompositorCanvasTiling, LayerData,
        LayerFlags,
    },
};

/// Associates the texture's actual dimensions and its buffer dimensions on the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferDimensions<const ALIGN: u32 = { wgpu::COPY_BYTES_PER_ROW_ALIGNMENT }> {
    width: u32,
    height: u32,
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
    extent: wgpu::Extent3d,
}

impl BufferDimensions {
    pub const RGBA_CHANNEL_COUNT: usize = 4;
    const BYTES_PER_PIXEL: u32 = (Self::RGBA_CHANNEL_COUNT * std::mem::size_of::<u8>()) as u32;
}

impl<const ALIGN: u32> BufferDimensions<ALIGN> {
    /// Computes the buffer dimensions between the texture's actual dimensions
    /// and its buffer dimensions on the GPU.
    pub const fn new(width: u32, height: u32) -> Self {
        Self::from_extent(wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        })
    }

    /// Computes the buffer dimensions from the GPU texture extent.
    pub const fn from_extent(extent: wgpu::Extent3d) -> Self {
        // It is a WebGPU requirement that
        // ImageCopyBuffer.layout.bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT == 0
        // So we calculate padded_bytes_per_row by rounding unpadded_bytes_per_row
        // up to the next multiple of wgpu::COPY_BYTES_PER_ROW_ALIGNMENT.
        // https://en.wikipedia.org/wiki/Data_structure_alignment#Computing_padding
        debug_assert!(extent.depth_or_array_layers == 1);
        let unpadded_bytes_per_row = extent.width * BufferDimensions::BYTES_PER_PIXEL;
        let padded_bytes_per_row_padding = (ALIGN - unpadded_bytes_per_row % ALIGN) % ALIGN;
        let padded_bytes_per_row = unpadded_bytes_per_row + padded_bytes_per_row_padding;
        Self {
            width: extent.width,
            height: extent.height,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
            extent,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn to_vec2(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn unpadded_bytes_per_row(&self) -> u32 {
        self.unpadded_bytes_per_row
    }

    pub fn padded_bytes_per_row(&self) -> u32 {
        self.padded_bytes_per_row
    }

    pub fn extent(&self) -> wgpu::Extent3d {
        self.extent
    }
}

/// Association between CPU buffer and GPU buffer.
#[derive(Clone)]
pub(crate) struct DataBuffer<T> {
    label: &'static str,
    data: T,
    buffer: wgpu::Buffer,
}

impl<T> DataBuffer<T> {
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Get CPU data.
    pub fn data_mut(&mut self) -> &mut T {
        &mut self.data
    }

    /// Get GPU data.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

impl<T> DataBuffer<Vec<T>>
where
    T: bytemuck::NoUninit,
{
    pub fn init_vec(
        device: &wgpu::Device,
        label: &'static str,
        data: Vec<T>,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data.as_slice()),
            usage,
        });
        Self {
            label,
            data,
            buffer,
        }
    }

    pub(super) fn data_len(&self) -> u64 {
        (self.data.len() * std::mem::size_of::<T>()) as u64
    }

    /// Load the GPU buffer with updated data, expanding it if needed.
    /// Returns `true` if the GPU buffer was reallocated, which invalidates
    /// any bind group referencing it.
    pub fn load_vec_buffer(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if self.buffer.size() < self.data_len() {
            self.buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(self.label),
                contents: bytemuck::cast_slice(self.data.as_slice()),
                usage: self.buffer.usage(),
            });
            true
        } else {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(self.data.as_slice()));
            false
        }
    }
}

impl<T> DataBuffer<T>
where
    T: bytemuck::NoUninit,
{
    pub fn init(
        device: &wgpu::Device,
        label: &'static str,
        data: T,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&data),
            usage,
        });
        Self {
            label,
            data,
            buffer,
        }
    }

    /// Load the GPU vertex buffer with updated data.
    pub fn load_buffer(&mut self, queue: &wgpu::Queue) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&self.data));
    }
}

pub(crate) struct CompositorBuffers {
    device: wgpu::Device,
    queue: wgpu::Queue,

    pub(crate) background: DataBuffer<[f32; 4]>,
    pub(crate) atlas: CompositorAtlasTiling,

    pub(crate) canvas: DataBuffer<CompositorCanvasTiling>,
    pub(crate) silos: DataBuffer<Vec<ChunkSilo>>,
    pub(crate) chunks: DataBuffer<Vec<ChunkData>>,
    pub(crate) layers: DataBuffer<Vec<LayerData>>,

    /// Every chunk of the document with its silo index, in draw order.
    /// The GPU chunk buffer is the subset of these that can contribute.
    source_chunks: Vec<(u32, ChunkData)>,
    /// Per-layer culling state the GPU chunk buffer was last built against.
    layer_cull: Vec<LayerCull>,
    /// The GPU chunk/silo buffers need to be rebuilt from `source_chunks`.
    chunks_dirty: bool,
    /// A storage buffer was reallocated since the last `take_reallocated`.
    reallocated: bool,
}

/// The parts of a layer that decide which of its chunks get uploaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LayerCull {
    /// Layer is visible and not fully transparent.
    drawn: bool,
    clipped: bool,
    mask_visible: bool,
}

impl CompositorBuffers {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        canvas_data: CompositorCanvasTiling,
        atlas_data: CompositorAtlasTiling,
    ) -> Self {
        Self {
            background: DataBuffer::init(
                device,
                "background",
                [0.0; 4],
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            atlas: atlas_data,
            layers: DataBuffer::init_vec(
                device,
                "layer_buffer",
                Vec::new(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            silos: DataBuffer::init_vec(
                device,
                "silo_buffer",
                Vec::new(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            chunks: DataBuffer::init_vec(
                device,
                "chunk_buffer",
                Vec::new(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            canvas: DataBuffer::init(
                device,
                "canvas_buffer",
                canvas_data,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            source_chunks: Vec::new(),
            layer_cull: Vec::new(),
            chunks_dirty: false,
            reallocated: false,
            device: device.clone(),
            queue: queue.clone(),
        }
    }

    /// Nothing to composite at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.source_chunks.is_empty() || self.layers.data().is_empty()
    }

    pub(super) fn load_layer_buffer(&mut self, composite_layers: &[CompositeLayer]) {
        let layers = self.layers.data_mut();
        layers.clear();
        let mut layer_cull = Vec::with_capacity(composite_layers.len());

        for layer in composite_layers.iter() {
            let mut flags = LayerFlags::empty();
            flags.set(LayerFlags::CLIPPED, layer.clipped);

            layers.push(LayerData {
                blend: layer.blend.to_u32(),
                opacity: layer.opacity,
                flags: flags.bits(),
            });
            layer_cull.push(LayerCull {
                drawn: !layer.hidden && layer.opacity > 0.0,
                clipped: layer.clipped,
                mask_visible: !layer.mask_hidden,
            });
        }

        self.reallocated |= self.layers.load_vec_buffer(&self.device, &self.queue);

        if layer_cull != self.layer_cull {
            self.layer_cull = layer_cull;
            self.chunks_dirty = true;
        }
    }

    pub(super) fn load_chunk_buffer(&mut self, chunks_data: &[ChunkTile]) {
        debug_assert!(chunks_data.is_sorted_by_key(|v| (v.col, v.row)));

        let num_cols = self.canvas.data().cols();
        let tile_size = self.canvas.data().tile_size();
        let atlas = self.atlas;
        let resolve = |index: Option<NonZeroU32>| index.map(|i| atlas.texel_ref(i.get(), tile_size));

        self.source_chunks.clear();
        self.source_chunks.extend(chunks_data.iter().map(|chunk| {
            let mask = resolve(chunk.mask_atlas_index);
            let clip = resolve(chunk.clip_atlas_index);
            let mut flags = ChunkFlags::empty();
            flags.set(ChunkFlags::HAS_MASK, mask.is_some());
            flags.set(ChunkFlags::HAS_CLIP, clip.is_some());
            let data = ChunkData {
                color: atlas.texel_ref(chunk.atlas_index.get(), tile_size),
                mask: mask.unwrap_or_default(),
                clip: clip.unwrap_or_default(),
                layer_index: chunk.layer_index,
                flags: flags.bits(),
            };
            (chunk.row * num_cols + chunk.col, data)
        }));

        self.chunks_dirty = true;
    }

    /// Rebuild the GPU chunk and silo buffers if the chunks or the layer
    /// culling state changed since the last upload.
    pub(crate) fn upload_chunks(&mut self) {
        if !std::mem::take(&mut self.chunks_dirty) {
            return;
        }

        let num_silos = (self.canvas.data().cols() * self.canvas.data().rows()) as usize;

        let silos = self.silos.data_mut();
        silos.clear();
        silos.resize(num_silos, ChunkSilo { start: 0, end: 0 });

        let chunks = self.chunks.data_mut();
        chunks.clear();

        for &(silo_index, mut chunk) in &self.source_chunks {
            let Some(layer) = self.layer_cull.get(chunk.layer_index as usize) else {
                continue;
            };
            if !layer.drawn {
                continue;
            }
            let mut flags = ChunkFlags::from_bits_retain(chunk.flags);
            // Clipped onto a base layer that has no pixels in this tile.
            if layer.clipped && !flags.contains(ChunkFlags::HAS_CLIP) {
                continue;
            }
            if !layer.mask_visible {
                flags.remove(ChunkFlags::HAS_MASK);
            }
            chunk.flags = flags.bits();

            // `source_chunks` is grouped by silo, so each silo's chunks are
            // contiguous.
            let silo = &mut silos[silo_index as usize];
            if silo.start == silo.end {
                silo.start = chunks.len() as u32;
            }
            chunks.push(chunk);
            silo.end = chunks.len() as u32;
        }

        // Storage bindings can't be empty. Every silo is empty in this case,
        // so the placeholder is never read.
        if chunks.is_empty() {
            chunks.push(ChunkData::default());
        }

        self.reallocated |= self.chunks.load_vec_buffer(&self.device, &self.queue);
        self.reallocated |= self.silos.load_vec_buffer(&self.device, &self.queue);
    }

    /// Whether any buffer bound in the blending bind group was reallocated
    /// since the last call.
    pub(crate) fn take_reallocated(&mut self) -> bool {
        std::mem::take(&mut self.reallocated)
    }
}
