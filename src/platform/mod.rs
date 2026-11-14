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
