//! Color management for documents with an embedded ICC profile.
//!
//! Layers are composited in the document's own color space, as Procreate
//! does, so blending gives the same results. The composited output is then:
//!
//! - converted to sRGB for display, through an [`OutputLut`] baked from the
//!   document's profile, since the window surface is treated as sRGB; and
//! - left in the document's color space for export, with the profile
//!   embedded, so wide-gamut colors survive.

use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};
use silicate_compositor::lut::OutputLut;

/// Entries per axis of the display lookup table.
const LUT_SIZE: u32 = 33;

/// Largest difference from the identity, per channel, for a profile to be
/// treated as sRGB and left unconverted.
const SRGB_TOLERANCE: f32 = 0.5 / 255.0;

/// Parse an ICC profile that describes RGB pixel data. Profiles for other
/// color spaces (Procreate offers CMYK canvases) don't describe how the
/// stored RGB values should be read, so they are rejected.
fn parse_rgb_profile(icc: &[u8]) -> Result<ColorProfile, String> {
    let profile = ColorProfile::new_from_slice(icc).map_err(|err| format!("{err:?}"))?;
    if profile.color_space != DataColorSpace::Rgb {
        return Err(format!("not an RGB profile ({:?})", profile.color_space));
    }
    Ok(profile)
}

/// The ICC profile to embed in exported images, if the document has one
/// that is valid for RGB images.
pub fn embeddable_icc(icc: Option<&[u8]>) -> Option<Vec<u8>> {
    let icc = icc?;
    match parse_rgb_profile(icc) {
        Ok(_) => Some(icc.to_vec()),
        Err(err) => {
            log::warn!("Not embedding the document's color profile in exports: {err}");
            None
        }
    }
}

/// Build a lookup table converting the document's colors to sRGB for
/// display. Returns `None` when no conversion is needed or possible: no
/// profile (assumed sRGB), an sRGB profile, or a profile that can't be used.
pub fn display_lut(icc: Option<&[u8]>) -> Option<OutputLut> {
    let icc = icc?;
    match build_display_lut(icc) {
        Ok(lut) => lut,
        Err(err) => {
            log::warn!("Displaying the document without color conversion: {err}");
            None
        }
    }
}

/// The table maps encoded document RGB to **linear, unclipped** sRGB; the
/// shader interpolates, then clips and applies the sRGB transfer curve.
///
/// Clipping before interpolating would be wrong near the sRGB gamut edge:
/// there a channel crosses zero, where the sRGB curve is steepest, and
/// interpolating clipped, encoded entries is off by over ten 8-bit steps.
/// Linear values change smoothly, so interpolating them stays accurate.
///
/// moxcms clips its output to the destination gamut, so the conversion goes
/// to linear ACEScg first, whose gamut contains Display P3 (and nearly all
/// visible colors), and then to linear sRGB by matrix, which doesn't clip.
fn build_display_lut(icc: &[u8]) -> Result<Option<OutputLut>, String> {
    let source = parse_rgb_profile(icc)?;
    let wide = ColorProfile::new_aces_cg_linear();
    let transform = source
        .create_transform_f32(
            Layout::Rgb,
            &wide,
            Layout::Rgb,
            TransformOptions {
                rendering_intent: RenderingIntent::RelativeColorimetric,
                // Built once per document, so precision matters more than speed.
                prefer_fixed_point: false,
                ..TransformOptions::default()
            },
        )
        .map_err(|err| format!("{err:?}"))?;
    let to_srgb = wide.transform_matrix(&ColorProfile::new_srgb()).v;

    let input = lut_grid(LUT_SIZE);
    let mut wide_linear = vec![0.0; input.len()];
    transform
        .transform(&input, &mut wide_linear)
        .map_err(|err| format!("{err:?}"))?;

    let data: Vec<[f32; 4]> = wide_linear
        .chunks_exact(3)
        .map(|rgb| {
            let [r, g, b] = to_srgb.map(|row| {
                (row[0] * rgb[0] as f64 + row[1] * rgb[1] as f64 + row[2] * rgb[2] as f64) as f32
            });
            [r, g, b, 1.0]
        })
        .collect();

    let max_deviation = input
        .chunks_exact(3)
        .zip(&data)
        .flat_map(|(input, linear)| (0..3).map(move |c| (input[c] - srgb_encode(linear[c])).abs()))
        .fold(0.0, f32::max);
    if max_deviation < SRGB_TOLERANCE {
        return Ok(None);
    }

    Ok(OutputLut::new(LUT_SIZE, data))
}

/// Clip a linear value and apply the sRGB transfer curve. Must match
/// `srgb_encode` in the compositor shader.
fn srgb_encode(linear: f32) -> f32 {
    let x = linear.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

/// Every lookup table input, as packed RGB in [`OutputLut`] order (red
/// varies fastest).
fn lut_grid(size: u32) -> Vec<f32> {
    let step = 1.0 / (size - 1) as f32;
    let mut grid = Vec::with_capacity(size.pow(3) as usize * 3);
    for b in 0..size {
        for g in 0..size {
            for r in 0..size {
                grid.extend([r as f32 * step, g as f32 * step, b as f32 * step]);
            }
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icc_bytes(profile: &ColorProfile) -> Vec<u8> {
        profile.encode().expect("profile encodes")
    }

    /// The same lookup as `apply_output_lut` in the compositor shader.
    fn lookup(lut: &OutputLut, rgb: [f32; 3]) -> [f32; 3] {
        let size = lut.size() as usize;
        let entry = |r: usize, g: usize, b: usize| lut.data()[(b * size + g) * size + r];
        let pos = rgb.map(|c| c.clamp(0.0, 1.0) * (size - 1) as f32);
        let i = pos.map(|p| (p as usize).min(size - 2));
        let t = [0, 1, 2].map(|k| pos[k] - i[k] as f32);
        let mut linear = [0.0; 3];
        for corner in 0..8 {
            let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let weight: f32 = (0..3)
                .map(|k| if d[k] == 1 { t[k] } else { 1.0 - t[k] })
                .product();
            let e = entry(i[0] + d[0], i[1] + d[1], i[2] + d[2]);
            for c in 0..3 {
                linear[c] += weight * e[c];
            }
        }
        linear.map(srgb_encode)
    }

    #[test]
    fn srgb_needs_no_conversion() {
        let icc = icc_bytes(&ColorProfile::new_srgb());
        assert_eq!(display_lut(Some(&icc)), None);
        assert!(embeddable_icc(Some(&icc)).is_some());
    }

    #[test]
    fn missing_or_invalid_profile_is_ignored() {
        assert_eq!(display_lut(None), None);
        assert_eq!(display_lut(Some(b"not an icc profile")), None);
        assert_eq!(embeddable_icc(Some(b"not an icc profile")), None);
    }

    /// Compare the interpolated table against moxcms converting each color
    /// directly to sRGB, across the whole cube including out-of-gamut colors.
    #[test]
    fn display_p3_lut_matches_direct_conversion() {
        let p3 = ColorProfile::new_display_p3();
        let lut = display_lut(Some(&icc_bytes(&p3))).expect("P3 needs conversion");
        let direct = p3
            .create_transform_f32(
                Layout::Rgb,
                &ColorProfile::new_srgb(),
                Layout::Rgb,
                TransformOptions {
                    rendering_intent: RenderingIntent::RelativeColorimetric,
                    // Built once per document, so precision matters more than speed.
                    prefer_fixed_point: false,
                    ..TransformOptions::default()
                },
            )
            .unwrap();

        let mut worst = (0.0f32, [0; 3]);
        for r in (0..=255).step_by(3) {
            for g in (0..=255).step_by(5) {
                for b in (0..=255).step_by(7) {
                    let rgb = [r, g, b].map(|v| v as f32 / 255.0);
                    let mut exact = [0.0; 3];
                    direct.transform(&rgb, &mut exact).unwrap();
                    let approx = lookup(&lut, rgb);
                    for c in 0..3 {
                        let err = (exact[c].clamp(0.0, 1.0) - approx[c]).abs();
                        if err > worst.0 {
                            worst = (err, [r, g, b]);
                        }
                    }
                }
            }
        }
        assert!(
            worst.0 * 255.0 < 1.0,
            "max error {:.3} 8-bit steps at P3 {:?}",
            worst.0 * 255.0,
            worst.1
        );
    }

    #[test]
    fn display_p3_is_actually_converted() {
        let lut = display_lut(Some(&icc_bytes(&ColorProfile::new_display_p3()))).unwrap();
        // Pure P3 red lies outside sRGB and clips to sRGB red.
        let red = lookup(&lut, [1.0, 0.0, 0.0]);
        assert!(red[0] > 0.99 && red[1] < 0.01 && red[2] < 0.01, "{red:?}");
        // Grays are unchanged.
        let gray = lookup(&lut, [0.5, 0.5, 0.5]);
        assert!(gray.iter().all(|c| (c - 0.5).abs() < 1.0 / 255.0), "{gray:?}");
    }
}
