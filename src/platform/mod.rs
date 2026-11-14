//! Platform-specific code.
//!
//! [`native`] and [`web`] expose the same API, so the rest of the crate can
//! use `crate::platform::*` without `cfg` attributes:
//!
//! - [`Runtime`]: spawns async tasks (tokio on native, the browser's event
//!   loop on web).
//! - [`FileSource`]: where a file to load comes from, and
//!   [`FileSource::open`] to get its bytes.
//! - [`decode_on_workers`]: whether the rayon pool can decode in the
//!   background.
//! - [`pick_file`] and [`read_dropped_file`]: get a [`FileSource`] from the
//!   user.
//! - [`pick_save_target`] and [`SaveTarget::save`]: write an exported image.
//! - [`pick_bytes_target`] and [`SaveTarget::save_bytes`]: write an
//!   already-encoded file, such as a zip archive or a PSD.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;

/// An exported image, ready to be written by [`SaveTarget::save`].
pub type ExportedImage = image::RgbaImage;

/// Formats of already-encoded files written by [`SaveTarget::save_bytes`].
#[derive(Clone, Copy, Debug)]
pub enum BytesFormat {
    Zip,
    Psd,
}

impl BytesFormat {
    pub fn extension(self) -> &'static str {
        match self {
            BytesFormat::Zip => "zip",
            BytesFormat::Psd => "psd",
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn mime_type(self) -> &'static str {
        match self {
            BytesFormat::Zip => "application/zip",
            BytesFormat::Psd => "image/vnd.adobe.photoshop",
        }
    }
}
