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

    pub fn set_flipped(&mut self, horizontally: bool, vertically: bool) {
        self.flipped = u32::from(horizontally) << 1 | u32::from(vertically);
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
        const HIDDEN = 1 << 1;
        const MASK_HIDDEN = 1 << 2;
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct ChunkSilo {
    pub start: u32,
    pub end: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable, Default)]
pub(crate) struct ChunkData {
    pub atlas_index: u32,
    pub clip_atlas_index: u32,
    pub mask_atlas_index: u32,
    pub layer_index: u32,
}
