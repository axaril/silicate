//! Exporting every layer of a document as a zip of PNGs.

use eframe::wgpu;
use silica_gpu::{ProcreateFile, SilicaHierarchy, SilicaLayer};
use silicate_compositor::{
    CompositeLayer, Compositor,
    blend::BlendingMode,
    buffer::BufferDimensions,
    canvas::{CompositorAtlasTiling, CompositorCanvasTiling},
    pipeline::Pipeline,
    tex::TextureExt,
};
use std::io::{Cursor, Write as _};

use super::{App, compositor::CompositorApp};

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

    let (width, height) = (file.size.width, file.size.height);
    let mut target = Compositor::new(
        device,
        queue,
        source.canvas,
        source.atlas,
        source.atlas_texture,
    );
    target.set_background(None);
    target.set_flipped(file.flipped.horizontally, file.flipped.vertically);
    let texture = wgpu::Texture::empty(device, width, height, wgpu::Texture::OUTPUT_USAGE);
    let dim = BufferDimensions::from_extent(texture.size());

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    // PNGs are already compressed.
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut chunks = Vec::new();

    for LayerEntry { path, layer } in entries {
        let hierarchy = [SilicaHierarchy::Layer(layer.clone())];
        CompositorApp::flatten_chunks(&mut chunks, &hierarchy, true);

        let image = if chunks.is_empty() {
            // The compositor skips empty layers, leaving the last render in
            // the texture.
            image::RgbaImage::new(width, height)
        } else {
            chunks.sort_by_key(|v| (v.col, v.row));
            target.load_chunk_buffer(&chunks);
            target.load_layer_buffer(&[CompositeLayer {
                opacity: layer.opacity,
                blend: BlendingMode::Normal,
                clipped: false,
                hidden: false,
                mask_hidden: layer.mask.as_ref().is_none_or(|mask| mask.hidden),
            }]);
            target.render(pipeline, texture.create_default_view());
            App::export(&texture, device, queue, dim)
                .await
                .map_err(|err| err.to_string())?
        };

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
