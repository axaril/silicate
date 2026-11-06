#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositorAtlasTiling {
    cols: u32,
    rows: u32,
}

impl CompositorAtlasTiling {
    pub fn new(cols: u32, rows: u32) -> Self {
        Self { cols, rows }
    }

    /// Resolve an atlas index into its texel origin and array layer, so the
    /// shader doesn't have to do integer division per sample.
    pub(crate) fn texel_ref(&self, index: u32, tile_size: u32) -> AtlasRef {
        let x = index % self.cols * tile_size;
        let y = index / self.cols % self.rows * tile_size;
        debug_assert!(x <= u16::MAX as u32 && y <= u16::MAX as u32);
        AtlasRef {
            origin: x | (y << 16),
            layer: index / (self.cols * self.rows),
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositorCanvasTiling {
    height: u32,
    width: u32,
    cols: u32,
    rows: u32,
    tile_size: u32,
    flipped: u32,
}

impl CompositorCanvasTiling {
    pub fn new((width, height): (u32, u32), (cols, rows): (u32, u32), tile_size: u32) -> Self {
        Self {
            height,
            width,
            cols,
            rows,
            tile_size,
            flipped: 0,
        }
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// Set the flip state. Returns `true` if it changed.
    pub fn set_flipped(&mut self, horizontally: bool, vertically: bool) -> bool {
        let flipped = u32::from(horizontally) << 1 | u32::from(vertically);
        let changed = self.flipped != flipped;
        self.flipped = flipped;
        changed
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct LayerData {
    pub opacity: f32,
    pub blend: u32,
    pub flags: u32,
}

bitflags::bitflags! {
    #[repr(transparent)]
    pub(crate) struct LayerFlags: u32 {
        const CLIPPED = 1;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct ChunkSilo {
    pub start: u32,
    pub end: u32,
}

/// Location of a tile in the atlas texture.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct AtlasRef {
    /// Texel origin, packed as `x | y << 16`.
    pub origin: u32,
    /// Array layer of the atlas texture.
    pub layer: u32,
}

bitflags::bitflags! {
    #[repr(transparent)]
    pub(crate) struct ChunkFlags: u32 {
        const HAS_MASK = 1;
        const HAS_CLIP = 1 << 1;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct ChunkData {
    pub color: AtlasRef,
    pub mask: AtlasRef,
    pub clip: AtlasRef,
    pub layer_index: u32,
    pub flags: u32,
}
