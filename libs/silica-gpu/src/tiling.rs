use silica::ns_archive::Size;

#[derive(Debug, Clone, Copy)]
pub struct AtlasTextureTiling {
    pub cols: u32,
    pub rows: u32,
    pub layers: u32,
}

impl AtlasTextureTiling {
    /// Choose an atlas layout that holds `chunk_count` tiles with as little
    /// unused space as possible.
    ///
    /// Tiles are spread evenly over the fewest array layers that fit, and
    /// each layer is kept close to square, so at most about one row of tiles
    /// per layer goes unused.
    pub fn compute_atlas_size(chunk_count: u32, tile_size: u32, limits: &wgpu::Limits) -> Self {
        let chunk_count = chunk_count.max(1);
        let max_cols = limits.max_texture_dimension_2d / tile_size;
        let max_rows = limits.max_texture_dimension_2d / tile_size;

        let layers = chunk_count.div_ceil(max_cols * max_rows);
        assert!(layers <= limits.max_texture_array_layers);

        let per_layer = chunk_count.div_ceil(layers);
        let cols = per_layer.isqrt().max(1).min(max_cols);
        let cols = if cols * cols < per_layer { (cols + 1).min(max_cols) } else { cols };
        let rows = per_layer.div_ceil(cols);
        debug_assert!(rows <= max_rows);

        AtlasTextureTiling { cols, rows, layers }
    }

    pub fn index(&self, atlas_index: u32) -> (u32, u32, u32) {
        return (
            atlas_index % self.cols,
            atlas_index / self.cols % self.rows,
            atlas_index / (self.cols * self.rows),
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CanvasTiling {
    pub cols: u32,
    pub rows: u32,
    pub diff: Size<u32>,
    pub size: u32,
    pub atlas: AtlasTextureTiling,
}

impl CanvasTiling {
    pub fn tile_extent(&self, col: u32, row: u32) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: if col != self.cols - 1 {
                self.size
            } else {
                self.size - self.diff.width
            },
            height: if row != self.rows - 1 {
                self.size
            } else {
                self.size - self.diff.height
            },
            depth_or_array_layers: 1,
        }
    }

    pub fn atlas_origin(&self, index: u32) -> wgpu::Origin3d {
        let (x, y, z) = self.atlas.index(index);
        wgpu::Origin3d {
            x: x * self.size,
            y: y * self.size,
            z,
        }
    }
}
