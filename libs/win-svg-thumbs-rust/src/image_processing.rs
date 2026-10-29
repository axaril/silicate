use image;
use windows::core::*;
use windows::Win32::{Foundation::*, Graphics::Gdi};
use zip;

use crate::guards::HBitmapGuard;
use crate::logging::log_message;

/// Calculate new dimensions that preserve aspect ratio while constraining
/// the maximum source dimension to the smallest target dimension
fn calculate_aspect_preserving_dimensions(
    src_width: u32,
    src_height: u32,
    target_width: u32,
    target_height: u32,
) -> (u32, u32) {
    // Find the smallest target dimension
    let min_target_dimension = target_width.min(target_height);

    // Calculate the scaling factor based on the largest source dimension
    let max_src_dimension = src_width.max(src_height);
    let scale_factor = min_target_dimension as f64 / max_src_dimension as f64;

    // Apply the scaling factor to both dimensions
    let new_width = (src_width as f64 * scale_factor).round() as u32;
    let new_height = (src_height as f64 * scale_factor).round() as u32;

    // Ensure we don't have zero dimensions
    let new_width = new_width.max(1);
    let new_height = new_height.max(1);

    (new_width, new_height)
}

pub fn extract_procreate_thumbnail_bytes(
    procreate_data: impl std::io::Read + std::io::Seek,
) -> Result<Vec<u8>> {
    // Open the ZIP archive
    let mut archive = zip::ZipArchive::new(procreate_data)
        .map_err(|e| Error::new(E_FAIL, &format!("Failed to open procreate archive: {}", e)))?;

    // Find and extract the QuickLook/Thumbnail.png file
    let mut thumbnail_file = archive.by_name("QuickLook/Thumbnail.png").map_err(|e| {
        Error::new(
            E_FAIL,
            &format!("Failed to find QuickLook/Thumbnail.png: {}", e),
        )
    })?;

    const MAX_SIZE: u64 = 512 * 1024 * 1024;
    pub const ERROR_FILE_TOO_LARGE: WIN32_ERROR = WIN32_ERROR(223u32);

    if thumbnail_file.size() > MAX_SIZE {
        log_message(&format!(
            "extract_procreate_thumbnail: Error - Thumbnail too large: {} bytes (max: {} bytes)",
            thumbnail_file.size(),
            MAX_SIZE
        ));
        return Err(Error::from(HRESULT::from_win32(ERROR_FILE_TOO_LARGE.0)));
    }

    // Read the PNG data
    let mut png_data = Vec::new();
    std::io::Read::read_to_end(&mut thumbnail_file, &mut png_data)
        .map_err(|e| Error::new(E_FAIL, &format!("Failed to read thumbnail PNG: {}", e)))?;
    Ok(png_data)
}

fn extract_procreate_thumbnail(
    png_data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    log_message(&format!(
        "extract_procreate_thumbnail: Extracting thumbnail to {}x{} pixels",
        width, height
    ));

    // Decode the PNG image
    let img = image::load_from_memory_with_format(&png_data, image::ImageFormat::Png)
        .map_err(|e| Error::new(E_FAIL, &format!("Failed to decode PNG: {}", e)))?;

    // Calculate dimensions to preserve aspect ratio
    // Constrain the max source dimension to the smallest target dimension
    let (new_width, new_height) =
        calculate_aspect_preserving_dimensions(img.width(), img.height(), width, height);

    // Resize the image preserving aspect ratio
    let resized_img = img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3);

    // Create a new image with the target dimensions and center the resized image
    let mut final_img = image::RgbaImage::new(width, height);

    // Fill with transparent background (or you could use a solid color)
    for pixel in final_img.pixels_mut() {
        *pixel = image::Rgba([0, 0, 0, 0]); // Transparent black
    }

    // Calculate position to center the resized image
    let x_offset = (width - new_width) / 2;
    let y_offset = (height - new_height) / 2;

    // Overlay the resized image onto the final image
    image::imageops::overlay(
        &mut final_img,
        &resized_img.to_rgba8(),
        x_offset.into(),
        y_offset.into(),
    );

    // Convert RGBA to BGRA for Windows compatibility
    let mut pixels = final_img.into_raw();
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2); // Swap R and B channels
    }

    log_message(&format!(
        "extract_procreate_thumbnail: Successfully extracted and resized {} bytes",
        pixels.len()
    ));
    Ok(pixels)
}

/// Renders .procreate file to HBITMAP by extracting the embedded thumbnail
pub fn render_procreate_to_hbitmap(
    pixel_data: &[u8],
    requested_width: u32,
    requested_height: u32,
) -> Result<Gdi::HBITMAP> {
    log_message(&format!(
        "render_procreate_to_hbitmap: Starting render for {}x{} size, {} bytes of data",
        requested_width,
        requested_height,
        pixel_data.len()
    ));

    let pixel_data = extract_procreate_thumbnail(pixel_data, requested_width, requested_height)?;

    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Gdi::HBITMAP> {
            log_message("render_procreate_to_hbitmap: Creating GDI HBITMAP from pixel data");

            // Create the final GDI HBITMAP
            let bmi = Gdi::BITMAPINFO {
                bmiHeader: Gdi::BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<Gdi::BITMAPINFOHEADER>() as u32,
                    biWidth: requested_width as i32,
                    biHeight: -(requested_height as i32), // Negative for top-down DIB
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: Gdi::BI_RGB.0 as u32,
                    ..Default::default()
                },
                ..Default::default()
            };

            let mut dib_data: *mut std::ffi::c_void = std::ptr::null_mut();
            let hbitmap_handle: Gdi::HBITMAP = unsafe {
                Gdi::CreateDIBSection(None, &bmi, Gdi::DIB_RGB_COLORS, &mut dib_data, None, 0)
            }?;
            let hbitmap_guard = HBitmapGuard::new(hbitmap_handle);

            // Copy pixels from extracted thumbnail to the GDI HBITMAP buffer
            if !dib_data.is_null() {
                let dest_data: &mut [u8] = unsafe {
                    std::slice::from_raw_parts_mut(
                        dib_data.cast::<u8>(),
                        (requested_width * requested_height * 4) as usize,
                    )
                };

                // Direct copy since both buffers have the same stride (width * 4 bytes per pixel)
                dest_data.copy_from_slice(&pixel_data);
            }

            log_message("render_procreate_to_hbitmap: Successfully created HBITMAP");
            Ok(hbitmap_guard.release())
        }));

    match result {
        Ok(res) => res,
        Err(_) => {
            log_message("render_procreate_to_hbitmap: Panic occurred during rendering");
            Err(Error::new(
                E_FAIL,
                "Panic occurred during procreate rendering",
            ))
        }
    }
}

/// Creates a simple fallback thumbnail for invalid files
pub fn create_fallback_thumbnail(size: u32) -> Result<Gdi::HBITMAP> {
    // Create a simple black square as fallback thumbnail
    let bmi = Gdi::BITMAPINFO {
        bmiHeader: Gdi::BITMAPINFOHEADER {
            biSize: std::mem::size_of::<Gdi::BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32), // Negative for top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: Gdi::BI_RGB.0 as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut dib_data: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbitmap_handle: Gdi::HBITMAP =
        unsafe { Gdi::CreateDIBSection(None, &bmi, Gdi::DIB_RGB_COLORS, &mut dib_data, None, 0) }?;
    let hbitmap_guard = HBitmapGuard::new(hbitmap_handle);

    if !dib_data.is_null() {
        // Fill with solid black (BGRA format: 0xFF000000)
        let pixel_count = (size * size) as usize;
        let buffer: &mut [u32] =
            unsafe { std::slice::from_raw_parts_mut(dib_data as *mut u32, pixel_count) };

        // Solid black with full alpha
        buffer.fill(0xFF000000);
    }

    log_message("create_fallback_thumbnail: Successfully created bitmap-based fallback");
    Ok(hbitmap_guard.release())
}
