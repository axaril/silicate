//! Color lookup table applied to the composited output.

/// A 3D color lookup table, mapping un-premultiplied RGB to **linear** sRGB.
///
/// Entry `(r, g, b)` is at index `(b * size + g) * size + r`, and maps the
/// input color `(r, g, b) / (size - 1)`. Values between entries are
/// interpolated trilinearly, then clipped to `0..=1` and encoded with the
/// sRGB transfer curve. Entries may lie outside `0..=1` (colors outside the
/// sRGB gamut); keeping them unclipped is what makes interpolation accurate
/// near the gamut edge. The alpha component of each entry is unused.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputLut {
    size: u32,
    data: Vec<[f32; 4]>,
}

impl OutputLut {
    /// Builds a table of `size³` entries. Returns `None` if `size < 2` or
    /// `data` doesn't hold exactly `size³` entries.
    pub fn new(size: u32, data: Vec<[f32; 4]>) -> Option<Self> {
        let expected = (size as usize).checked_pow(3)?;
        (size >= 2 && data.len() == expected).then_some(Self { size, data })
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn data(&self) -> &[[f32; 4]] {
        &self.data
    }

    pub(crate) fn create_texture(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        create_lut_texture(device, queue, self.size, bytemuck::cast_slice(&self.data))
    }
}

/// A 1×1×1 table, which the shader treats as "no lookup".
pub(crate) fn disabled_lut_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    create_lut_texture(device, queue, 1, bytemuck::cast_slice(&[[0f32; 4]]))
}

fn create_lut_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: u32,
    bytes: &[u8],
) -> wgpu::Texture {
    let extent = wgpu::Extent3d {
        width: size,
        height: size,
        depth_or_array_layers: size,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("output_lut"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        // Read with `textureLoad` and interpolated in the shader, so it
        // doesn't need to be filterable.
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 16),
            rows_per_image: Some(size),
        },
        extent,
    );
    texture
}
