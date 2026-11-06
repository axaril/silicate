mod blend;
pub mod compositor;
pub mod instance;

use compositor::CompositorApp;
use eframe::egui_wgpu::wgpu;
use egui_tiles::TileId;
use instance::{Instance, InstanceKey};
use silica_gpu::{ProcreateFile, ProcreateFileAtlas, error::SilicaError};
use silicate_compositor::{
    Compositor,
    buffer::BufferDimensions,
    canvas::{CompositorAtlasTiling, CompositorCanvasTiling},
    pipeline::Pipeline,
    tex::TextureExt,
};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicUsize, mpsc::Sender},
    time::Duration,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::OpenOptions, path::PathBuf};

/// Where a file to load comes from.
pub enum FileSource {
    /// A file on disk, memory-mapped while it decodes.
    #[cfg(not(target_arch = "wasm32"))]
    Path(PathBuf),
    /// File contents already in memory, such as from a browser file picker.
    /// Native frontends currently always have a path.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Bytes(Arc<[u8]>),
}

/// The contents of an opened [`FileSource`].
enum FileBytes {
    #[cfg(not(target_arch = "wasm32"))]
    Mapped(memmap2::Mmap),
    Owned(Arc<[u8]>),
}

impl std::ops::Deref for FileBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            FileBytes::Mapped(mapping) => mapping,
            FileBytes::Owned(bytes) => bytes,
        }
    }
}

impl FileSource {
    fn open(self) -> Result<FileBytes, SilicaError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            FileSource::Path(path) => {
                let file = OpenOptions::new().read(true).write(false).open(path)?;
                // SAFETY: the mapping is only read. Another process modifying
                // the file while it decodes could produce garbage tiles, but
                // the decoder bounds-checks everything it reads.
                let mapping = unsafe { memmap2::Mmap::map(&file)? };
                Ok(FileBytes::Mapped(mapping))
            }
            FileSource::Bytes(bytes) => Ok(FileBytes::Owned(bytes)),
        }
    }
}

pub type LoadResult = Result<(ProcreateFile, ProcreateFileAtlas), SilicaError>;

pub enum AppEvent {
    NewInstance(InstanceKey, Instance, CompositorApp),
    NewView(Option<TileId>, InstanceKey),
    RebindTexture(InstanceKey),
    RebindPreviews(InstanceKey),
    RemoveInstance(InstanceKey),
    LoadFile {
        source: FileSource,
        tabs: Option<TileId>,
    },
    /// A file finished decoding off the UI thread.
    FileDecoded {
        result: LoadResult,
        tabs: Option<TileId>,
    },
    LoadDialog(Option<TileId>),
    SaveDialog(wgpu::Texture),
    Toast(egui_notify::Toast),
    SetTheme(egui::ThemePreference),
    #[cfg(target_arch = "wasm32")]
    LoadDemoFile,
}

impl std::fmt::Debug for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppEvent::NewInstance(arg0, arg1, _) => f
                .debug_tuple("NewInstance")
                .field(arg0)
                .field(arg1)
                .finish(),
            AppEvent::NewView(tabs, instance_key) => f
                .debug_tuple("NewView")
                .field(tabs)
                .field(instance_key)
                .finish(),
            AppEvent::RebindTexture(arg0) => f.debug_tuple("RebindTexture").field(arg0).finish(),
            AppEvent::RebindPreviews(arg0) => f.debug_tuple("RebindPreviews").field(arg0).finish(),
            AppEvent::RemoveInstance(arg0) => f.debug_tuple("RemoveInstance").field(arg0).finish(),
            AppEvent::Toast(_) => f.debug_tuple("Toast").field(&"...").finish(),
            AppEvent::LoadFile { .. } => f.debug_tuple("LoadFile").field(&"...").finish(),
            AppEvent::FileDecoded { .. } => f.debug_tuple("FileDecoded").field(&"...").finish(),
            AppEvent::LoadDialog(_) => f.debug_tuple("LoadDialog").field(&"...").finish(),
            AppEvent::SaveDialog(_) => f.debug_tuple("SaveDialog").field(&"...").finish(),
            AppEvent::SetTheme(theme) => f.debug_tuple("SetTheme").field(theme).finish(),
            #[cfg(target_arch = "wasm32")]
            AppEvent::LoadDemoFile => f.debug_tuple("LoadDemoFile").finish(),
        }
    }
}

pub struct App {
    device: wgpu::Device,
    queue: wgpu::Queue,
    event_sender: Sender<AppEvent>,
    pipeline: Pipeline,
    curr_id: AtomicUsize,
}

impl App {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, event_sender: Sender<AppEvent>) -> Self {
        Self {
            pipeline: Pipeline::new(&device),
            device,
            queue,
            event_sender,
            curr_id: AtomicUsize::new(0),
        }
    }

    /// Load a file without blocking the UI thread.
    ///
    /// Tiles are decompressed on the rayon pool and uploaded by whichever
    /// task awaits this future, as they arrive. Uploading stays off the
    /// workers because WebGPU objects can't be used from a Web Worker, and
    /// the bounded queue keeps decoded tiles from piling up in memory.
    /// Pass the result to [`App::create_instance`].
    pub fn load(&self, source: FileSource) -> impl Future<Output = LoadResult> + 'static {
        let device = self.device.clone();
        let queue = self.queue.clone();
        async move {
            log::info!("Loading file");

            let bytes = source.open()?;
            let (decoder, atlas) = ProcreateFile::prepare(&bytes, &device)?;

            if !Self::decode_on_workers() {
                return Self::decode_inline(decoder, &bytes, atlas, &queue);
            }

            // Workers block once this many decoded tiles are waiting. This
            // matters most on wasm, where memory never shrinks, so any
            // backlog counts towards the page's memory for its lifetime.
            const MAX_QUEUED_TILES: usize = 64;

            let (tile_tx, mut tile_rx) = tokio::sync::mpsc::channel(MAX_QUEUED_TILES);
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            rayon::spawn(move || {
                let result = decoder.decode(&bytes, &|tile| {
                    // Rayon workers may block, including Web Workers.
                    let _ = tile_tx.blocking_send(tile);
                });
                let _ = done_tx.send(result);
                // `tile_tx` drops here, ending the upload loop below.
            });

            while let Some(tile) = tile_rx.recv().await {
                atlas.upload(&queue, &tile);
            }

            let file = done_rx.await.expect("decoder dropped without a result")?;
            Ok((file, atlas))
        }
    }

    /// Whether the rayon pool can run decoding in the background.
    fn decode_on_workers() -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return true;
        #[cfg(target_arch = "wasm32")]
        return crate::web::threads_enabled();
    }

    /// Decode on the current thread, for wasm builds without a thread pool.
    fn decode_inline(
        decoder: silica_gpu::ProcreateFileDecoder,
        bytes: &[u8],
        atlas: ProcreateFileAtlas,
        queue: &wgpu::Queue,
    ) -> LoadResult {
        // Without atomics, wgpu objects are `Sync`, so tiles can go straight
        // to the queue as they're decoded.
        #[cfg(not(all(target_arch = "wasm32", target_feature = "atomics")))]
        let file = decoder.decode(bytes, &|tile| atlas.upload(queue, &tile))?;

        // With atomics but no pool (it failed to start), wgpu objects can't
        // be captured by the decoder, so every tile is held until the end.
        #[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
        let file = {
            let tiles = std::sync::Mutex::new(Vec::new());
            let file = decoder.decode(bytes, &|tile| tiles.lock().unwrap().push(tile))?;
            for tile in tiles.into_inner().unwrap() {
                atlas.upload(queue, &tile);
            }
            file
        };

        Ok((file, atlas))
    }

    pub fn create_instance(
        &self,
        file: ProcreateFile,
        metadata: ProcreateFileAtlas,
    ) -> InstanceKey {
        let id = InstanceKey::new(
            self.curr_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );

        log::info!(
            "{id} Loaded Procreate document \"{}\" with {} layers",
            file.name.as_deref().unwrap_or("Untitled Artwork"),
            file.layer_count(true)
        );

        let ProcreateFileAtlas {
            atlas_texture,
            canvas_tiling,
            ..
        } = metadata;

        let canvas = CompositorCanvasTiling::new(
            (file.size.width, file.size.height),
            (canvas_tiling.cols, canvas_tiling.rows),
            canvas_tiling.size,
        );
        let composite_target = Compositor::new(
            &self.device,
            &self.queue,
            canvas,
            CompositorAtlasTiling::new(canvas_tiling.atlas.cols, canvas_tiling.atlas.rows),
            atlas_texture.clone(),
        );

        let output_texture = wgpu::Texture::empty(
            &self.device,
            file.size.width,
            file.size.height,
            wgpu::Texture::OUTPUT_USAGE,
        );

        let rotation = match file.orientation {
            silica_gpu::Orientation::NoRotation => 0.0,
            silica_gpu::Orientation::Clockwise180 => 180.0,
            silica_gpu::Orientation::Clockwise270 => 270.0,
            silica_gpu::Orientation::Clockwise90 => 90.0,
            _ => 0f32,
        }
        .to_radians();

        let initial_compositor_file = Arc::new(file.clone());
        let (compositor, handle) = CompositorApp::new(
            id,
            self.pipeline.clone(),
            initial_compositor_file.clone(),
            composite_target,
        );

        let mut instance = Instance {
            id,
            file: file.clone(),
            output_texture: output_texture.clone(),
            preview_textures: None,
            compositor: handle,
            rotation,
            previews: HashMap::new(),
            canvas: None,
        };

        log::debug!(
            "{id} Generating previews for Procreate document \"{}\"",
            file.name.as_deref().unwrap_or("Untitled Artwork")
        );

        instance.generate_previews(
            Compositor::new(
                &self.device,
                &self.queue,
                canvas,
                CompositorAtlasTiling::new(canvas_tiling.atlas.cols, canvas_tiling.atlas.rows),
                atlas_texture,
            ),
            &self.device,
            &self.pipeline,
        );

        log::info!(
            "{id} Instance created for Procreate document \"{}\"",
            file.name.as_deref().unwrap_or("Untitled Artwork")
        );

        self.event_sender
            .send(AppEvent::NewInstance(id, instance, compositor))
            .unwrap();
        id
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    /// Export the texture to the given path.
    pub async fn export(
        texture: &wgpu::Texture,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dim: BufferDimensions,
    ) -> image::ImageResult<image::ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
        let output_buffer = texture.export_buffer(device, queue, dim);

        let buffer_slice = output_buffer.slice(..);

        // NOTE: We have to create the mapping THEN device.poll() before await
        // the future. Otherwise the application will freeze.
        let (tx, rx) = tokio::sync::oneshot::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .unwrap();
        rx.await.unwrap().expect("Buffer mapping failed");

        let data = buffer_slice.get_mapped_range().expect("Failed to get mapped range").to_vec();
        output_buffer.unmap();

        log::debug!("Loading data to CPU");
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
            dim.padded_bytes_per_row() / 4,
            dim.height(),
            data,
        )
        .unwrap();

        Ok(image::imageops::crop_imm(&buffer, 0, 0, dim.width(), dim.height()).to_image())
    }

    pub fn rebind_texture(&self, id: InstanceKey) {
        self.event_sender.send(AppEvent::RebindTexture(id)).unwrap();
    }
}
