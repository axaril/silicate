//! A minimal writer for layered Photoshop (PSD) files.
//!
//! Writes 8-bit RGB documents with transparency, layer groups, blending
//! modes, clipping, layer masks and Unicode layer names. Channel data is
//! compressed with PackBits as soon as a layer is added, so only the
//! compressed layers stay in memory.
//!
//! See Adobe's "Photoshop File Formats Specification".

use silica_gpu::BlendingMode;

/// The largest width or height a PSD (as opposed to a PSB) can hold.
pub const MAX_DIMENSION: u32 = 30_000;

/// A rectangle in canvas pixels, as `top, left, bottom, right`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Rect {
    top: u32,
    left: u32,
    bottom: u32,
    right: u32,
}

impl Rect {
    fn width(&self) -> u32 {
        self.right - self.left
    }

    fn height(&self) -> u32 {
        self.bottom - self.top
    }

    /// The smallest rectangle holding every pixel where `keep` is true, or
    /// an empty rectangle if there are none.
    fn bounding(width: u32, height: u32, keep: impl Fn(u32, u32) -> bool) -> Self {
        let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..height {
            for x in 0..width {
                if keep(x, y) {
                    left = left.min(x);
                    right = right.max(x + 1);
                    top = top.min(y);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        if left == u32::MAX {
            Rect::default()
        } else {
            Rect {
                top,
                left,
                bottom,
                right,
            }
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        for v in [self.top, self.left, self.bottom, self.right] {
            out.extend_from_slice(&(v as i32).to_be_bytes());
        }
    }
}

/// Channel ids in a layer record.
const CHANNEL_ALPHA: i16 = -1;
const CHANNEL_USER_MASK: i16 = -2;

struct Channel {
    id: i16,
    /// Compression method followed by the image data.
    data: Vec<u8>,
}

/// The `lsct` section divider type of a group record.
#[derive(Clone, Copy)]
enum Section {
    OpenFolder = 1,
    ClosedFolder = 2,
    /// The hidden record marking the bottom of a group.
    Divider = 3,
}

struct MaskInfo {
    rect: Rect,
    /// The mask's value outside of `rect`.
    default_color: u8,
    disabled: bool,
}

/// One layer record. Groups take two records: one for the group itself and
/// a divider below its children.
pub struct Record {
    name: String,
    rect: Rect,
    channels: Vec<Channel>,
    mask: Option<MaskInfo>,
    blend: [u8; 4],
    opacity: u8,
    clipped: bool,
    hidden: bool,
    alpha_locked: bool,
    section: Option<Section>,
}

/// Properties shared by layers and groups.
pub struct Properties<'a> {
    pub name: &'a str,
    pub opacity: f32,
    pub hidden: bool,
}

/// A layer mask: 0 hides the layer, 255 shows it.
pub struct Mask<'a> {
    pub image: &'a image::GrayImage,
    pub disabled: bool,
}

impl Record {
    /// A pixel layer from a canvas-sized, straight (not premultiplied)
    /// image. Fully transparent borders are cropped away.
    pub fn layer(
        props: Properties,
        image: &image::RgbaImage,
        blend: BlendingMode,
        clipped: bool,
        alpha_locked: bool,
        mask: Option<Mask>,
    ) -> Self {
        let (width, height) = image.dimensions();
        let rect = Rect::bounding(width, height, |x, y| image.get_pixel(x, y)[3] != 0);

        let mut channels: Vec<Channel> = [CHANNEL_ALPHA, 0, 1, 2]
            .into_iter()
            .map(|id| {
                let index = if id == CHANNEL_ALPHA { 3 } else { id as usize };
                Channel {
                    id,
                    data: encode_channel(rect, |x, y| image.get_pixel(x, y)[index]),
                }
            })
            .collect();

        let mask = mask.map(|mask| {
            let (width, height) = mask.image.dimensions();
            let rect = Rect::bounding(width, height, |x, y| mask.image.get_pixel(x, y)[0] != 0);
            channels.push(Channel {
                id: CHANNEL_USER_MASK,
                data: encode_channel(rect, |x, y| mask.image.get_pixel(x, y)[0]),
            });
            MaskInfo {
                rect,
                default_color: 0,
                disabled: mask.disabled,
            }
        });

        Self {
            name: props.name.to_owned(),
            rect,
            channels,
            mask,
            blend: blend_key(blend),
            opacity: opacity_byte(props.opacity),
            clipped,
            hidden: props.hidden,
            alpha_locked,
            section: None,
        }
    }

    /// The record at the top of a group, above its children.
    pub fn group(props: Properties, collapsed: bool) -> Self {
        let section = if collapsed {
            Section::ClosedFolder
        } else {
            Section::OpenFolder
        };
        Self {
            opacity: opacity_byte(props.opacity),
            hidden: props.hidden,
            ..Self::empty(props.name, *b"pass", section)
        }
    }

    /// The record at the bottom of a group, below its children.
    pub fn group_end() -> Self {
        Self::empty("</Layer group>", *b"norm", Section::Divider)
    }

    fn empty(name: &str, blend: [u8; 4], section: Section) -> Self {
        Self {
            name: name.to_owned(),
            rect: Rect::default(),
            channels: [CHANNEL_ALPHA, 0, 1, 2]
                .into_iter()
                .map(|id| Channel {
                    id,
                    data: encode_channel(Rect::default(), |_, _| 0),
                })
                .collect(),
            mask: None,
            blend,
            opacity: 255,
            clipped: false,
            hidden: false,
            alpha_locked: false,
            section: Some(section),
        }
    }

    fn write(&self, out: &mut Vec<u8>) -> Result<(), String> {
        self.rect.write(out);
        out.extend_from_slice(&(self.channels.len() as u16).to_be_bytes());
        for channel in &self.channels {
            out.extend_from_slice(&channel.id.to_be_bytes());
            out.extend_from_slice(&len_u32(channel.data.len())?.to_be_bytes());
        }
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(&self.blend);
        out.push(self.opacity);
        out.push(u8::from(self.clipped));
        let mut flags = 0;
        if self.alpha_locked {
            flags |= 0x01;
        }
        if self.hidden {
            flags |= 0x02;
        }
        if self.section.is_some() {
            // Bit 4 is meaningful: the pixel data doesn't affect appearance.
            flags |= 0x08 | 0x10;
        }
        out.push(flags);
        out.push(0);

        let mut extra = Vec::new();
        match &self.mask {
            Some(mask) => {
                extra.extend_from_slice(&20u32.to_be_bytes());
                mask.rect.write(&mut extra);
                extra.push(mask.default_color);
                extra.push(if mask.disabled { 0x02 } else { 0 });
                extra.extend_from_slice(&[0, 0]);
            }
            None => extra.extend_from_slice(&0u32.to_be_bytes()),
        }
        // No blending ranges.
        extra.extend_from_slice(&0u32.to_be_bytes());
        write_pascal_name(&self.name, &mut extra);
        write_unicode_name(&self.name, &mut extra);
        if let Some(section) = self.section {
            extra.extend_from_slice(b"8BIMlsct");
            match section {
                Section::Divider => {
                    extra.extend_from_slice(&4u32.to_be_bytes());
                    extra.extend_from_slice(&(section as u32).to_be_bytes());
                }
                _ => {
                    extra.extend_from_slice(&12u32.to_be_bytes());
                    extra.extend_from_slice(&(section as u32).to_be_bytes());
                    extra.extend_from_slice(b"8BIM");
                    extra.extend_from_slice(&self.blend);
                }
            }
        }

        out.extend_from_slice(&len_u32(extra.len())?.to_be_bytes());
        out.extend_from_slice(&extra);
        Ok(())
    }
}

/// Write a PSD document.
///
/// `records` are ordered bottom to top, as Photoshop stores them. `merged`
/// is the flattened image shown by readers that don't understand layers,
/// straight (not premultiplied).
pub fn write(
    width: u32,
    height: u32,
    records: &[Record],
    merged: &image::RgbaImage,
) -> Result<Vec<u8>, String> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(format!(
            "PSD files can't be larger than {MAX_DIMENSION}×{MAX_DIMENSION} pixels"
        ));
    }

    let mut out = Vec::new();

    // Header: RGB with an alpha channel for the merged image.
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&4u16.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes());
    out.extend_from_slice(&3u16.to_be_bytes());

    // Color mode data and image resources: none.
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());

    // Layer and mask information.
    let mut layer_info = Vec::new();
    let count = i16::try_from(records.len()).map_err(|_| "Too many layers for a PSD file")?;
    // A negative count says the merged image's alpha channel is its
    // transparency.
    layer_info.extend_from_slice(&(-count).to_be_bytes());
    for record in records {
        record.write(&mut layer_info)?;
    }
    for record in records {
        for channel in &record.channels {
            layer_info.extend_from_slice(&channel.data);
        }
    }
    if layer_info.len() % 2 != 0 {
        layer_info.push(0);
    }

    let section_len = 4 + layer_info.len() + 4;
    out.extend_from_slice(&len_u32(section_len)?.to_be_bytes());
    out.extend_from_slice(&len_u32(layer_info.len())?.to_be_bytes());
    out.extend_from_slice(&layer_info);
    // No global layer mask.
    out.extend_from_slice(&0u32.to_be_bytes());

    write_merged(merged, &mut out);
    Ok(out)
}

/// The merged image: RLE compressed, with the byte counts of every row of
/// every channel up front, then each channel in turn.
///
/// Photoshop expects the color of a transparent merged image to be matted
/// against white.
fn write_merged(image: &image::RgbaImage, out: &mut Vec<u8>) {
    let (width, height) = image.dimensions();
    let mut counts = Vec::with_capacity(4 * height as usize);
    let mut data = Vec::new();
    let mut row = Vec::with_capacity(width as usize);
    for channel in 0..4 {
        for y in 0..height {
            row.clear();
            row.extend((0..width).map(|x| {
                let pixel = image.get_pixel(x, y);
                if channel == 3 {
                    pixel[3]
                } else {
                    let alpha = u32::from(pixel[3]);
                    let color = u32::from(pixel[channel]);
                    ((color * alpha + 255 * (255 - alpha) + 127) / 255) as u8
                }
            }));
            let start = data.len();
            packbits(&row, &mut data);
            counts.push((data.len() - start) as u16);
        }
    }
    out.extend_from_slice(&1u16.to_be_bytes());
    for count in counts {
        out.extend_from_slice(&count.to_be_bytes());
    }
    out.extend_from_slice(&data);
}

/// Compress the part of a channel inside `rect`, prefixed by its compression
/// method.
fn encode_channel(rect: Rect, value: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let (width, height) = (rect.width(), rect.height());
    if width == 0 || height == 0 {
        // Raw, with no data.
        return 0u16.to_be_bytes().to_vec();
    }

    let mut counts = Vec::with_capacity(height as usize);
    let mut data = Vec::new();
    let mut row = Vec::with_capacity(width as usize);
    for y in rect.top..rect.bottom {
        row.clear();
        row.extend((rect.left..rect.right).map(|x| value(x, y)));
        let start = data.len();
        packbits(&row, &mut data);
        counts.push((data.len() - start) as u16);
    }

    let mut out = Vec::with_capacity(2 + 2 * counts.len() + data.len());
    out.extend_from_slice(&1u16.to_be_bytes());
    for count in counts {
        out.extend_from_slice(&count.to_be_bytes());
    }
    out.extend_from_slice(&data);
    out
}

/// PackBits run-length encoding of one row.
fn packbits(row: &[u8], out: &mut Vec<u8>) {
    let n = row.len();
    let mut i = 0;
    while i < n {
        let mut run = 1;
        while i + run < n && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as u8);
            out.push(row[i]);
            i += run;
        } else {
            // Copy literally until the next run of three or more.
            let start = i;
            i += 1;
            while i < n && i - start < 128 {
                if i + 2 < n && row[i] == row[i + 1] && row[i] == row[i + 2] {
                    break;
                }
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&row[start..i]);
        }
    }
}

/// The legacy name: a Pascal string padded to a multiple of 4 bytes. Readers
/// prefer the Unicode name, so characters outside ASCII become `?`.
fn write_pascal_name(name: &str, out: &mut Vec<u8>) {
    let bytes: Vec<u8> = name
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .take(255)
        .collect();
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    let padding = (4 - (1 + bytes.len()) % 4) % 4;
    out.extend(std::iter::repeat_n(0, padding));
}

/// The `luni` additional layer information: the name in UTF-16.
fn write_unicode_name(name: &str, out: &mut Vec<u8>) {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut data = Vec::with_capacity(4 + 2 * units.len());
    data.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for unit in units {
        data.extend_from_slice(&unit.to_be_bytes());
    }
    while data.len() % 4 != 0 {
        data.push(0);
    }
    out.extend_from_slice(b"8BIMluni");
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&data);
}

fn opacity_byte(opacity: f32) -> u8 {
    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn len_u32(len: usize) -> Result<u32, String> {
    u32::try_from(len).map_err(|_| "The document is too large for a PSD file".to_owned())
}

fn blend_key(blend: BlendingMode) -> [u8; 4] {
    *match blend {
        BlendingMode::Normal => b"norm",
        BlendingMode::Multiply => b"mul ",
        BlendingMode::Screen => b"scrn",
        BlendingMode::Add => b"lddg",
        BlendingMode::Lighten => b"lite",
        BlendingMode::Exclusion => b"smud",
        BlendingMode::Difference => b"diff",
        BlendingMode::Subtract => b"fsub",
        BlendingMode::LinearBurn => b"lbrn",
        BlendingMode::ColorDodge => b"div ",
        BlendingMode::ColorBurn => b"idiv",
        BlendingMode::Overlay => b"over",
        BlendingMode::HardLight => b"hLit",
        BlendingMode::Color => b"colr",
        BlendingMode::Luminosity => b"lum ",
        BlendingMode::Hue => b"hue ",
        BlendingMode::Saturation => b"sat ",
        BlendingMode::SoftLight => b"sLit",
        BlendingMode::Darken => b"dark",
        BlendingMode::HardMix => b"hMix",
        BlendingMode::VividLight => b"vLit",
        BlendingMode::LinearLight => b"lLit",
        BlendingMode::PinLight => b"pLit",
        BlendingMode::LighterColor => b"lgCl",
        BlendingMode::DarkerColor => b"dkCl",
        BlendingMode::Divide => b"fdiv",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpackbits(mut data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        while let [header, rest @ ..] = data {
            let header = *header as i8;
            if header >= 0 {
                let len = header as usize + 1;
                out.extend_from_slice(&rest[..len]);
                data = &rest[len..];
            } else {
                let len = (1 - header as i32) as usize;
                out.extend(std::iter::repeat_n(rest[0], len));
                data = &rest[1..];
            }
        }
        out
    }

    #[test]
    fn packbits_round_trips() {
        let rows: [Vec<u8>; 6] = [
            vec![],
            vec![7],
            vec![1, 2, 3, 4, 5],
            vec![0; 1000],
            (0..=255).chain(std::iter::repeat_n(9, 300)).chain(0..10).collect(),
            vec![1, 1, 2, 2, 3, 3, 3, 4, 5, 5],
        ];
        for row in rows {
            let mut packed = Vec::new();
            packbits(&row, &mut packed);
            assert_eq!(unpackbits(&packed), row);
        }
    }

    #[test]
    fn pascal_names_are_padded() {
        for name in ["", "a", "ab", "abc", "abcd", "Ünïcode"] {
            let mut out = Vec::new();
            write_pascal_name(name, &mut out);
            assert_eq!(out.len() % 4, 0, "{name:?}");
            assert_eq!(out[0] as usize, name.chars().count());
        }
    }

    /// Write a sample document to `$PSD_SAMPLE` for checking with other
    /// readers.
    #[test]
    fn write_sample() {
        let (w, h) = (64, 48);
        let red = image::RgbaImage::from_fn(w, h, |x, y| {
            if (8..40).contains(&x) && (4..20).contains(&y) {
                image::Rgba([255, 0, 0, 128])
            } else {
                image::Rgba([0; 4])
            }
        });
        let blue = image::RgbaImage::from_fn(w, h, |x, _| {
            image::Rgba([0, 0, 255, if x < 32 { 255 } else { 0 }])
        });
        let mask = image::GrayImage::from_fn(w, h, |_, y| image::Luma([if y < 24 { 255 } else { 0 }]));
        let props = |name, hidden| Properties {
            name,
            opacity: 0.5,
            hidden,
        };
        // Top to bottom, then reversed.
        let mut records = vec![
            Record::group(props("Grüppe ✨", false), false),
            Record::layer(props("Red", false), &red, BlendingMode::Multiply, true, false, None),
            Record::layer(
                props("Blue", false),
                &blue,
                BlendingMode::Normal,
                false,
                true,
                Some(Mask {
                    image: &mask,
                    disabled: false,
                }),
            ),
            Record::group_end(),
            Record::layer(props("Hidden", true), &red, BlendingMode::Screen, false, false, None),
        ];
        records.reverse();
        let bytes = write(w, h, &records, &blue).unwrap();
        if let Ok(path) = std::env::var("PSD_SAMPLE") {
            std::fs::write(path, bytes).unwrap();
        }
    }

    #[test]
    fn bounding_crops_to_content() {
        let rect = Rect::bounding(10, 10, |x, y| (2..5).contains(&x) && y == 7);
        assert_eq!(
            rect,
            Rect {
                top: 7,
                left: 2,
                bottom: 8,
                right: 5
            }
        );
        assert_eq!(Rect::bounding(3, 3, |_, _| false), Rect::default());
    }
}
