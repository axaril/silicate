use crate::ZipArchiveMmap;
use crate::error::SilicaError;
use crate::tiling::{AtlasTextureTiling, CanvasTiling};
use crate::types::{hierarchy::SilicaHierarchy, layer::SilicaLayer};
use rayon::{iter::ParallelDrainRange, prelude::ParallelIterator};
use silica::ns_archive::NsArchive;
use silica::ns_archive::Size;
use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use zip::read::ZipArchive;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcreateFile {
    info: silica::ProcreateFile,
    pub composite: Option<SilicaLayer>,
    pub layers: Vec<SilicaHierarchy>,
}

impl std::ops::Deref for ProcreateFile {
    type Target = silica::ProcreateFile;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

impl std::ops::DerefMut for ProcreateFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.info
    }
}

pub struct ProcreateFileAtlas {
    pub atlas_texture: wgpu::Texture,
    pub canvas_tiling: CanvasTiling,
    /// Bytes written with `write_texture` since the last queue submit.
    pending_upload_bytes: AtomicU64,
}

/// wgpu holds every `write_texture` in staging memory until the next queue
/// submit. Submitting after this many bytes stops a large file from staging
/// a second copy of its whole atlas before the first render.
const UPLOAD_FLUSH_BYTES: u64 = 64 * 1024 * 1024;

/// A decoded tile of pixel data waiting to be written into the atlas.
pub struct TileUpload {
    pub origin: wgpu::Origin3d,
    pub size: wgpu::Extent3d,
    /// Tightly packed RGBA8 pixels.
    pub data: Vec<u8>,
}

/// The CPU-only half of loading a Procreate file.
///
/// Holds no GPU resources, so it can be moved to a worker thread
/// (including Web Workers, where WebGPU objects are unavailable).
pub struct ProcreateFileDecoder {
    info: silica::ProcreateFile,
    tiling: CanvasTiling,
}

impl ProcreateFile {
    /// Load a Procreate file, uploading tiles to the GPU as they are decoded.
    ///
    /// Unavailable on threaded wasm, where WebGPU objects can't be shared
    /// with the workers doing the decoding.
    #[cfg(not(all(target_arch = "wasm32", target_feature = "atomics")))]
    pub fn open(
        bytes: &[u8],
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<(Self, ProcreateFileAtlas), SilicaError> {
        let (decoder, atlas) = Self::prepare(bytes, device)?;
        let file = decoder.decode(bytes, &|tile| atlas.upload(queue, &tile))?;
        Ok((file, atlas))
    }

    /// Parse the document metadata and allocate the atlas texture.
    ///
    /// Tile decoding is deferred to [`ProcreateFileDecoder::decode`].
    pub fn prepare(
        bytes: &[u8],
        device: &wgpu::Device,
    ) -> Result<(ProcreateFileDecoder, ProcreateFileAtlas), SilicaError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;

        let nka: NsArchive = {
            let mut document = archive.by_name("Document.archive")?;

            let mut buf = Vec::with_capacity(document.size() as usize);
            document.read_to_end(&mut buf)?;

            NsArchive::from_reader(Cursor::new(buf))?
        };

        let info = silica::ProcreateFile::from_ns(&nka)?;

        // Only layer, mask and composite tiles go into the atlas. The archive
        // also holds thumbnails and timelapse video, which aren't loaded.
        let chunk_count = {
            let mut uuids = HashSet::new();
            collect_layer_uuids(&info.layers, &mut uuids);
            if let Some(composite) = &info.composite {
                uuids.insert(composite.uuid.as_str());
            }
            let mut tiles = 0;
            for name in archive.file_names() {
                if name?
                    .split_once('/')
                    .is_some_and(|(dir, _)| uuids.contains(dir))
                {
                    tiles += 1;
                }
            }
            // Atlas slot 0 is reserved as an empty tile.
            tiles + 1
        };

        let size = info.size;
        let tile_size = info.tile_size;

        let (cols, rows) = (
            size.width.div_ceil(tile_size),
            size.height.div_ceil(tile_size),
        );

        let tiling = CanvasTiling {
            cols,
            rows,
            diff: Size {
                width: cols * tile_size - size.width,
                height: rows * tile_size - size.height,
            },
            size: tile_size,
            atlas: AtlasTextureTiling::compute_atlas_size(chunk_count, tile_size, &device.limits()),
        };

        let atlas_texture = Self::empty_layers(
            device,
            tiling.size * tiling.atlas.cols,
            tiling.size * tiling.atlas.rows,
            tiling.atlas.layers, // Make it an array
        );

        Ok((
            ProcreateFileDecoder { info, tiling },
            ProcreateFileAtlas {
                atlas_texture,
                canvas_tiling: tiling,
                pending_upload_bytes: AtomicU64::new(0),
            },
        ))
    }

    pub fn layer_count(&self, include_groups: bool) -> u32 {
        self.layers
            .iter()
            .map(|layer| layer.layer_count(include_groups))
            .sum()
    }

    pub fn empty_layers(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        layers: u32,
    ) -> wgpu::Texture {
        const TEX_DIM: wgpu::TextureDimension = wgpu::TextureDimension::D2;
        const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TEX_DIM,
            format: TEX_FORMAT,
            view_formats: &[
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ],
            usage: wgpu::TextureUsages::COPY_DST
                .union(wgpu::TextureUsages::COPY_SRC)
                .union(wgpu::TextureUsages::TEXTURE_BINDING),
            label: None,
        })
    }
}

impl ProcreateFileDecoder {
    /// Decompress every tile in the archive in parallel on the rayon pool.
    ///
    /// `upload` is called once per tile, from whichever rayon thread
    /// decoded it. `bytes` must be the same archive given to
    /// [`ProcreateFile::prepare`].
    pub fn decode(
        self,
        bytes: &[u8],
        upload: &(dyn Fn(TileUpload) + Sync),
    ) -> Result<ProcreateFile, SilicaError> {
        let Self { mut info, tiling } = self;

        let archive: ZipArchiveMmap<'_> = ZipArchive::new(Cursor::new(bytes))?;
        let file_names = archive.file_names().collect::<Result<Vec<_>, _>>()?;

        let params = crate::params::LoadParams {
            archive: &archive,
            file_names,
            tiling,
            upload,
            chunk_id_counter: AtomicU32::new(1),
            layer_id_counter: AtomicU32::new(0),
        };

        Ok(ProcreateFile {
            composite: info
                .composite
                .take()
                .and_then(|composite| SilicaLayer::load(composite, &params, false).ok()),
            layers: info
                .layers
                .par_drain(..)
                .map(|ir| SilicaHierarchy::load(ir, &params))
                .collect::<Result<_, _>>()?,
            info,
        })
    }
}

impl ProcreateFileAtlas {
    /// Replace a section of the atlas with a decoded tile.
    ///
    /// ### Note
    /// The tile's origin and size should strictly fit within the texture boundaries.
    pub fn upload(&self, queue: &wgpu::Queue, tile: &TileUpload) {
        let texture = &self.atlas_texture;
        let layers = texture.size().depth_or_array_layers;
        assert!(
            tile.origin.z < layers,
            "index {} must be less than {}",
            tile.origin.z,
            layers
        );
        queue.write_texture(
            // Tells wgpu where to copy the pixel data
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: tile.origin,
                aspect: wgpu::TextureAspect::All,
            },
            // The actual pixel data
            &tile.data,
            // The layout of the texture
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * tile.size.width),
                rows_per_image: Some(tile.size.height),
            },
            tile.size,
        );

        let size = tile.data.len() as u64;
        let pending = self.pending_upload_bytes.fetch_add(size, Ordering::Relaxed) + size;
        if pending >= UPLOAD_FLUSH_BYTES
            && self
                .pending_upload_bytes
                .compare_exchange(pending, 0, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            // An empty submit flushes the queued writes and lets wgpu free
            // their staging memory once the GPU has copied them.
            queue.submit([]);
        }
    }
}

/// Collect the UUIDs of every layer and mask, whose tiles live in the
/// archive directory of the same name.
fn collect_layer_uuids<'a>(layers: &'a [silica::SilicaHierarchy], uuids: &mut HashSet<&'a str>) {
    for layer in layers {
        match layer {
            silica::SilicaHierarchy::Layer(layer) => {
                uuids.insert(layer.uuid.as_str());
                if let Some(mask) = &layer.mask {
                    uuids.insert(mask.uuid.as_str());
                }
            }
            silica::SilicaHierarchy::Group(group) => collect_layer_uuids(&group.children, uuids),
        }
    }
}
