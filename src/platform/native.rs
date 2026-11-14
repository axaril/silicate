use super::ExportedImage;
use silica_gpu::error::SilicaError;
use std::fs::OpenOptions;
use std::path::PathBuf;

/// Spawns async tasks on a multi-threaded tokio runtime.
pub struct Runtime {
    rt: tokio::runtime::Runtime,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            rt: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime creation successful"),
        }
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(future);
    }
}

/// Where a file to load comes from.
pub enum FileSource {
    /// A file on disk, memory-mapped while it decodes.
    Path(PathBuf),
}

/// The contents of an opened [`FileSource`].
pub type FileBytes = memmap2::Mmap;

impl FileSource {
    pub fn open(self) -> Result<FileBytes, SilicaError> {
        match self {
            FileSource::Path(path) => {
                let file = OpenOptions::new().read(true).write(false).open(path)?;
                // SAFETY: the mapping is only read. Another process modifying
                // the file while it decodes could produce garbage tiles, but
                // the decoder bounds-checks everything it reads.
                Ok(unsafe { memmap2::Mmap::map(&file)? })
            }
        }
    }
}

/// Whether the rayon pool can run decoding in the background.
pub fn decode_on_workers() -> bool {
    true
}

/// Ask the user for a file to open. Returns `Ok(None)` if they cancel.
pub async fn pick_file() -> Result<Option<FileSource>, String> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter("All Files", &["*"])
        .add_filter("Procreate Files", &["procreate"])
        .pick_file()
        .await;

    Ok(handle.map(|handle| FileSource::Path(handle.path().to_path_buf())))
}

/// Turn a file dropped onto the window into a [`FileSource`].
pub async fn read_dropped_file(file: egui::DroppedFileHandle) -> Result<FileSource, String> {
    Ok(FileSource::Path(file.path().to_path_buf()))
}

/// Where an exported image will be written.
pub struct SaveTarget {
    handle: rfd::FileHandle,
}

/// Ask the user where to save an image. Returns `None` if they cancel.
pub async fn pick_save_target() -> Option<SaveTarget> {
    rfd::AsyncFileDialog::new()
        .add_filter("png", image::ImageFormat::Png.extensions_str())
        .add_filter("jpeg", image::ImageFormat::Jpeg.extensions_str())
        .add_filter("tga", image::ImageFormat::Tga.extensions_str())
        .add_filter("tiff", image::ImageFormat::Tiff.extensions_str())
        .add_filter("webp", image::ImageFormat::WebP.extensions_str())
        .add_filter("bmp", image::ImageFormat::Bmp.extensions_str())
        .save_file()
        .await
        .map(|handle| SaveTarget { handle })
}

impl SaveTarget {
    /// A name for the target, for messages to the user.
    pub fn name(&self) -> String {
        self.handle.file_name()
    }

    /// Write the image, in the format implied by the chosen file extension.
    pub async fn save(self, image: ExportedImage) -> Result<(), String> {
        let path = self.handle.path().to_path_buf();
        log::info!("Saving the file to {}", path.display());
        tokio::task::spawn_blocking(move || image.save(path))
            .await
            .map_err(|err| err.to_string())?
            .map_err(|err| err.to_string())
    }
}
