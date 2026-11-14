//! Exporting every layer of a document, as a zip of PNGs or as a PSD.

use eframe::wgpu;
use silica_gpu::{ProcreateFile, SilicaGroup, SilicaHierarchy, SilicaLayer};
use silicate_compositor::{
    ChunkTile, CompositeLayer, Compositor,
    blend::BlendingMode,
    buffer::BufferDimensions,
    canvas::{CompositorAtlasTiling, CompositorCanvasTiling},
    pipeline::Pipeline,
    tex::TextureExt,
};
use std::io::{Cursor, Write as _};

use super::{App, compositor::CompositorApp, psd};

/// What's needed to render a document's layers outside its compositor.
#[derive(Clone)]
pub struct LayerSource {
    pub canvas: CompositorCanvasTiling,
    pub atlas: CompositorAtlasTiling,
    pub atlas_texture: wgpu::Texture,
}

/// A layer to export, with its path inside the archive.
struct LayerEntry<'a> {
    path: String,
    layer: &'a SilicaLayer,
}

/// Turn a layer or group name into a file name, which can't hold path
/// separators or characters that Windows rejects.
fn sanitize(name: Option<&str>, fallback: &str) -> String {
    let cleaned: String = name
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// List every layer, top to bottom, with groups as folders. Names are
/// prefixed with their position so archives sort in layer order, which also
/// keeps layers that share a name apart.
fn collect_layers<'a>(layers: &'a [SilicaHierarchy], prefix: &str, out: &mut Vec<LayerEntry<'a>>) {
    let width = layers.len().to_string().len();
    for (index, item) in layers.iter().enumerate() {
        let position = format!("{:0width$}", index + 1);
        match item {
            SilicaHierarchy::Group(group) => {
                let name = sanitize(group.name.as_deref(), "Group");
                let folder = format!("{prefix}{position} {name}/");
                collect_layers(&group.children, &folder, out);
            }
            SilicaHierarchy::Layer(layer) => {
                let name = sanitize(layer.name.as_deref(), "Layer");
                out.push(LayerEntry {
                    path: format!("{prefix}{position} {name} ({}).png", layer.blend.as_str()),
                    layer,
                });
            }
        }
    }
}

/// Renders layers on their own, at full canvas size, into images.
struct LayerRenderer<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    pipeline: &'a Pipeline,
    target: Compositor,
    texture: wgpu::Texture,
    dim: BufferDimensions,
    chunks: Vec<ChunkTile>,
    tile_size: u32,
    flipped: (bool, bool),
}

impl<'a> LayerRenderer<'a> {
    fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        pipeline: &'a Pipeline,
        source: LayerSource,
        file: &ProcreateFile,
    ) -> Self {
        let tile_size = source.canvas.tile_size();
        let mut target = Compositor::new(
            device,
            queue,
            source.canvas,
            source.atlas,
            source.atlas_texture,
        );
        target.set_background(None);
        target.set_flipped(file.flipped.horizontally, file.flipped.vertically);
        let texture = wgpu::Texture::empty(
            device,
            file.size.width,
            file.size.height,
            wgpu::Texture::OUTPUT_USAGE,
        );
        let dim = BufferDimensions::from_extent(texture.size());
        Self {
            device,
            queue,
            pipeline,
            target,
            texture,
            dim,
            chunks: Vec::new(),
            tile_size,
            flipped: (file.flipped.horizontally, file.flipped.vertically),
        }
    }

    fn blank(&self) -> image::RgbaImage {
        image::RgbaImage::new(self.dim.width(), self.dim.height())
    }

    async fn read_back(&self) -> Result<image::RgbaImage, String> {
        App::export(&self.texture, self.device, self.queue, self.dim)
            .await
            .map_err(|err| err.to_string())
    }

    /// Render one layer with normal blending and without clipping. The mask
    /// is applied if `apply_mask` and it isn't hidden. The result is
    /// premultiplied.
    async fn render_layer(
        &mut self,
        layer: &SilicaLayer,
        opacity: f32,
        apply_mask: bool,
    ) -> Result<image::RgbaImage, String> {
        let hierarchy = [SilicaHierarchy::Layer(layer.clone())];
        CompositorApp::flatten_chunks(&mut self.chunks, &hierarchy, apply_mask);
        if self.chunks.is_empty() {
            // The compositor skips empty layers, leaving the last render in
            // the texture.
            return Ok(self.blank());
        }
        self.chunks.sort_by_key(|v| (v.col, v.row));
        self.target.load_chunk_buffer(&self.chunks);
        self.target.load_layer_buffer(&[CompositeLayer {
            opacity,
            blend: BlendingMode::Normal,
            clipped: false,
            hidden: false,
            mask_hidden: !apply_mask || layer.mask.as_ref().is_none_or(|mask| mask.hidden),
        }]);
        self.target
            .render(self.pipeline, self.texture.create_default_view());
        self.read_back().await
    }

    /// Render a layer's mask as the compositor applies it: the mask's alpha
    /// where it has tiles, and fully visible where the layer has tiles but
    /// the mask doesn't.
    async fn render_mask(
        &mut self,
        layer: &SilicaLayer,
        mask: &SilicaLayer,
    ) -> Result<image::GrayImage, String> {
        let rendered = self.render_layer(mask, 1.0, false).await?;
        let (width, height) = rendered.dimensions();
        let mut out = image::GrayImage::from_fn(width, height, |x, y| {
            image::Luma([rendered.get_pixel(x, y)[3]])
        });

        let ts = self.tile_size;
        let (flip_h, flip_v) = self.flipped;
        for chunk in &layer.image.chunks {
            let covered = mask
                .image
                .chunks
                .iter()
                .any(|m| m.col == chunk.col && m.row == chunk.row);
            if covered {
                continue;
            }
            // Tile rows count up from the bottom of the canvas unless it is
            // flipped vertically.
            let x0 = (chunk.col * ts).min(width);
            let x1 = ((chunk.col + 1) * ts).min(width);
            let y0 = (chunk.row * ts).min(height);
            let y1 = ((chunk.row + 1) * ts).min(height);
            let (x0, x1) = if flip_h { (width - x1, width - x0) } else { (x0, x1) };
            let (y0, y1) = if flip_v { (y0, y1) } else { (height - y1, height - y0) };
            for y in y0..y1 {
                for x in x0..x1 {
                    out.put_pixel(x, y, image::Luma([255]));
                }
            }
        }
        Ok(out)
    }

    /// Render the whole document as the viewer shows it. The result is
    /// premultiplied.
    async fn render_document(
        &mut self,
        file: &ProcreateFile,
        background: Option<[f32; 4]>,
    ) -> Result<image::RgbaImage, String> {
        let mut layers = Vec::new();
        CompositorApp::flatten_layers(&mut layers, &file.layers);
        CompositorApp::flatten_chunks(&mut self.chunks, &file.layers, true);
        if self.chunks.is_empty() {
            let mut image = self.blank();
            if let Some(color) = background {
                let pixel = image::Rgba(color.map(color_byte));
                image.pixels_mut().for_each(|p| *p = pixel);
            }
            return Ok(image);
        }
        self.chunks.sort_by_key(|v| (v.col, v.row));
        self.target.load_chunk_buffer(&self.chunks);
        self.target.load_layer_buffer(&layers);
        self.target.set_background(background);
        self.target
            .render(self.pipeline, self.texture.create_default_view());
        self.target.set_background(None);
        self.read_back().await
    }
}

fn color_byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Convert a premultiplied image to straight alpha in place.
fn unpremultiply(image: &mut image::RgbaImage) {
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel.0 = [0; 4];
        } else if alpha < 255 {
            for c in &mut pixel.0[..3] {
                *c = ((u32::from(*c) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
}

/// Render each layer on its own at full canvas size and pack them as PNGs
/// into a zip archive.
///
/// Layers are exported even when hidden, with their opacity and mask
/// applied but without blending or clipping, since those depend on the
/// layers below. The blending mode is kept in the file name instead.
pub async fn export_layers(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &Pipeline,
    source: LayerSource,
    file: &ProcreateFile,
) -> Result<Vec<u8>, String> {
    let mut entries = Vec::new();
    collect_layers(&file.layers, "", &mut entries);

    let mut renderer = LayerRenderer::new(device, queue, pipeline, source, file);

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // PNGs are already compressed.
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for LayerEntry { path, layer } in entries {
        let image = renderer.render_layer(layer, layer.opacity, true).await?;

        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|err| err.to_string())?;

        zip.start_file(path, options)
            .map_err(|err| err.to_string())?;
        zip.write_all(png.get_ref())
            .map_err(|err| err.to_string())?;
    }

    zip.finish()
        .map(Cursor::into_inner)
        .map_err(|err| err.to_string())
}

/// An item of the layer tree, top to bottom.
enum PsdItem<'a> {
    GroupStart(&'a SilicaGroup),
    Layer { layer: &'a SilicaLayer, clipped: bool },
    GroupEnd,
}

/// List the layer tree top to bottom, with each group's children between
/// its start and end.
fn collect_psd_items<'a>(layers: &'a [SilicaHierarchy], out: &mut Vec<PsdItem<'a>>) {
    // Procreate only clips onto a layer below in the same group, never onto
    // a group, and draws a clipped layer without a base normally. Work out
    // which clipped layers have a base, the way the compositor does.
    let mut clipped = vec![false; layers.len()];
    let mut has_base = false;
    for (index, item) in layers.iter().enumerate().rev() {
        match item {
            SilicaHierarchy::Group(_) => has_base = false,
            SilicaHierarchy::Layer(layer) => {
                clipped[index] = layer.clipped && has_base;
                if !layer.clipped {
                    has_base = true;
                }
            }
        }
    }

    for (item, clipped) in layers.iter().zip(clipped) {
        match item {
            SilicaHierarchy::Group(group) => {
                out.push(PsdItem::GroupStart(group));
                collect_psd_items(&group.children, out);
                out.push(PsdItem::GroupEnd);
            }
            SilicaHierarchy::Layer(layer) => out.push(PsdItem::Layer { layer, clipped }),
        }
    }
}

/// Export the document as a layered PSD.
///
/// Groups, visibility, opacity, blending modes, clipping, alpha lock and
/// layer masks are kept as layer properties, so each layer's pixels are
/// rendered without them. The background color becomes a layer at the
/// bottom, hidden if it is hidden in Procreate.
pub async fn export_psd(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &Pipeline,
    source: LayerSource,
    file: &ProcreateFile,
) -> Result<Vec<u8>, String> {
    let (width, height) = (file.size.width, file.size.height);
    if width > psd::MAX_DIMENSION || height > psd::MAX_DIMENSION {
        return Err(format!(
            "PSD files can't be larger than {0}×{0} pixels",
            psd::MAX_DIMENSION
        ));
    }

    let mut items = Vec::new();
    collect_psd_items(&file.layers, &mut items);

    let mut renderer = LayerRenderer::new(device, queue, pipeline, source, file);
    // Top to bottom; PSDs store them bottom to top.
    let mut records = Vec::with_capacity(items.len() + 1);

    for item in items {
        match item {
            PsdItem::GroupStart(group) => records.push(psd::Record::group(
                psd::Properties {
                    name: group.name.as_deref().unwrap_or("Group"),
                    opacity: group.opacity,
                    hidden: group.hidden,
                },
                group.collapsed,
            )),
            PsdItem::GroupEnd => records.push(psd::Record::group_end()),
            PsdItem::Layer { layer, clipped } => {
                let mut image = renderer.render_layer(layer, 1.0, false).await?;
                unpremultiply(&mut image);
                let mask = match &layer.mask {
                    Some(mask) => Some((renderer.render_mask(layer, mask).await?, mask.hidden)),
                    None => None,
                };
                records.push(psd::Record::layer(
                    psd::Properties {
                        name: layer.name.as_deref().unwrap_or("Layer"),
                        opacity: layer.opacity,
                        hidden: layer.hidden,
                    },
                    &image,
                    layer.blend,
                    clipped,
                    layer.alpha_locked,
                    mask.as_ref().map(|(image, disabled)| psd::Mask {
                        image,
                        disabled: *disabled,
                    }),
                ));
            }
        }
    }

    let [r, g, b, _] = file.background_color.map(color_byte);
    let background = image::RgbaImage::from_pixel(width, height, image::Rgba([r, g, b, 255]));
    records.push(psd::Record::layer(
        psd::Properties {
            name: "Background color",
            opacity: 1.0,
            hidden: file.background_hidden,
        },
        &background,
        silica_gpu::BlendingMode::Normal,
        false,
        false,
        None,
    ));
    drop(background);

    let visible_background = (!file.background_hidden).then_some(file.background_color);
    let mut merged = renderer.render_document(file, visible_background).await?;
    unpremultiply(&mut merged);

    records.reverse();
    psd::write(width, height, &records, &merged)
}
