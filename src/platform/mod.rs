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
//! - [`pick_archive_target`] and [`SaveTarget::save_bytes`]: write an
//!   exported zip archive.

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

/// Encode an image in `format`, embedding the `icc` color profile where the
/// format supports one (PNG, JPEG, TIFF and WebP). Other formats are written
/// without it.
pub fn encode_image(
    image: &ExportedImage,
    format: image::ImageFormat,
    icc: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    use image::codecs::{jpeg::JpegEncoder, png::PngEncoder, tiff::TiffEncoder, webp::WebPEncoder};
    use std::io::Cursor;

    fn write(
        image: &ExportedImage,
        mut encoder: impl image::ImageEncoder,
        icc: Option<&[u8]>,
    ) -> image::ImageResult<()> {
        if let Some(icc) = icc
            && let Err(err) = encoder.set_icc_profile(icc.to_vec())
        {
            log::warn!("Exporting without a color profile: {err}");
        }
        image.write_with_encoder(encoder)
    }

    let mut out = Cursor::new(Vec::new());
    match format {
        image::ImageFormat::Png => write(image, PngEncoder::new(&mut out), icc),
        image::ImageFormat::Jpeg => write(image, JpegEncoder::new(&mut out), icc),
        image::ImageFormat::Tiff => write(image, TiffEncoder::new(&mut out), icc),
        image::ImageFormat::WebP => write(image, WebPEncoder::new_lossless(&mut out), icc),
        format => image.write_to(&mut out, format),
    }
    .map_err(|err| err.to_string())?;
    Ok(out.into_inner())
}
