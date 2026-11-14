use crate::ns_archive::{NsArchive, error::NsArchiveError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendingMode {
    Normal = 0,
    Multiply = 1,
    Screen = 2,
    Add = 3,
    Lighten = 4,
    Exclusion = 5,
    Difference = 6,
    Subtract = 7,
    LinearBurn = 8,
    ColorDodge = 9,
    ColorBurn = 10,
    Overlay = 11,
    HardLight = 12,
    Color = 13,
    Luminosity = 14,
    Hue = 15,
    Saturation = 16,
    SoftLight = 17,
    Darken = 19,
    HardMix = 20,
    VividLight = 21,
    LinearLight = 22,
    PinLight = 23,
    LighterColor = 24,
    DarkerColor = 25,
    Divide = 26,
}

impl std::fmt::Display for BlendingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BlendingMode {
    pub fn all() -> &'static [BlendingMode] {
        use BlendingMode::*;
        &[
            Multiply,
            Darken,
            // Shade, unknown which values map to this, if any
            ColorBurn,
            LinearBurn,
            DarkerColor,
            Normal,
            Lighten,
            Screen,
            ColorDodge,
            Add,
            LighterColor,
            Overlay,
            SoftLight,
            HardLight,
            VividLight,
            LinearLight,
            PinLight,
            HardMix,
            Difference,
            Exclusion,
            Subtract,
            Divide,
            Hue,
            Saturation,
            Color,
            Luminosity,
        ]
    }

    pub fn as_short_str(&self) -> &'static str {
        match self {
            Self::Normal => "N",
            Self::Multiply => "M",
            Self::Screen => "S",
            Self::Add => "A",
            Self::Lighten => "Li",
            Self::Exclusion => "E",
            Self::Difference => "D",
            Self::Subtract => "Su",
            Self::LinearBurn => "Lb",
            Self::ColorDodge => "Cd",
            Self::ColorBurn => "Cb",
            Self::Overlay => "O",
            Self::HardLight => "Hl",
            Self::Color => "C",
            Self::Luminosity => "L",
            Self::Hue => "H",
            Self::Saturation => "Sa",
            Self::SoftLight => "Sl",
            Self::Darken => "Da",
            Self::HardMix => "Hm",
            Self::VividLight => "Vl",
            Self::LinearLight => "Ll",
            Self::PinLight => "Pl",
            Self::LighterColor => "Lc",
            Self::DarkerColor => "Dc",
            Self::Divide => "Di",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Add => "Add",
            Self::Lighten => "Lighten",
            Self::Exclusion => "Exclusion",
            Self::Difference => "Difference",
            Self::Subtract => "Subtract",
            Self::LinearBurn => "Linear Burn",
            Self::ColorDodge => "Color Dodge",
            Self::ColorBurn => "Color Burn",
            Self::Overlay => "Overlay",
            Self::HardLight => "Hard Light",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::SoftLight => "Soft Light",
            Self::Darken => "Darken",
            Self::HardMix => "Hard Mix",
            Self::VividLight => "Vivid Light",
            Self::LinearLight => "Linear Light",
            Self::PinLight => "Pin Light",
            Self::LighterColor => "Lighter Color",
            Self::DarkerColor => "Darker Color",
            Self::Divide => "Divide",
        }
    }

    pub fn from_u32(blend: u32) -> Option<Self> {
        Some(match blend {
            0 => Self::Normal,
            1 => Self::Multiply,
            2 => Self::Screen,
            3 => Self::Add,
            4 => Self::Lighten,
            5 => Self::Exclusion,
            6 => Self::Difference,
            7 => Self::Subtract,
            8 => Self::LinearBurn,
            9 => Self::ColorDodge,
            10 => Self::ColorBurn,
            11 => Self::Overlay,
            12 => Self::HardLight,
            13 => Self::Color,
            14 => Self::Luminosity,
            15 => Self::Hue,
            16 => Self::Saturation,
            17 => Self::SoftLight,
            19 => Self::Darken,
            20 => Self::HardMix,
            21 => Self::VividLight,
            22 => Self::LinearLight,
            23 => Self::PinLight,
            24 => Self::LighterColor,
            25 => Self::DarkerColor,
            26 => Self::Divide,
            _ => return None,
        })
    }

    pub fn to_u32(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flipped {
    pub horizontally: bool,
    pub vertically: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    NoRotation,
    Clockwise180,
    Clockwise270,
    Clockwise90,
    Unknown,
}

impl crate::ns_archive::NsDecode<'_> for Orientation {
    fn decode(nka: &NsArchive, key: &str, val: &plist::Value) -> Result<Self, NsArchiveError> {
        Ok(match u64::decode(nka, key, val)? {
            1 => Self::NoRotation,
            2 => Self::Clockwise180,
            3 => Self::Clockwise270,
            4 => Self::Clockwise90,
            // Orientation only affects how the canvas is presented, so an
            // unrecognized value should not make the document unreadable.
            _ => Self::Unknown,
        })
    }
}

/// The role of a `SilicaLayer` object, from its `type` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// A regular content layer.
    Layer,
    /// The flattened document composite.
    Composite,
    /// The document-level mask (believed to be the selection mask).
    DocumentMask,
    /// A layer mask attached to a regular layer.
    LayerMask,
    /// A value not known to this version of the library.
    Other(u64),
}

impl crate::ns_archive::NsDecode<'_> for LayerKind {
    fn decode(nka: &NsArchive, key: &str, val: &plist::Value) -> Result<Self, NsArchiveError> {
        Ok(match u64::decode(nka, key, val)? {
            0 => Self::Layer,
            1 => Self::Composite,
            2 => Self::DocumentMask,
            3 => Self::LayerMask,
            v => Self::Other(v),
        })
    }
}

/// A rectangle in canvas pixels, using the same axes as the tile grid
/// (x increases with tile column, y increases with tile row).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl crate::ns_archive::NsDecode<'_> for Rect {
    /// Decodes a CGRect archived as four little-endian `f64`s.
    fn decode(nka: &NsArchive, key: &str, val: &plist::Value) -> Result<Self, NsArchiveError> {
        let bytes = <&[u8]>::decode(nka, key, val)?;
        let [x, y, width, height] = le_array::<f64, 4, 8>(bytes, key)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// The ICC color profile that the document's pixel data is stored in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorProfile {
    /// Human-readable profile name, e.g. `"Display P3"`.
    pub name: Option<String>,
    /// Raw ICC profile bytes, suitable for embedding in exported images.
    pub icc_data: Option<Vec<u8>>,
}

impl<'a> crate::ns_archive::NsDecode<'a> for ColorProfile {
    fn decode(nka: &'a NsArchive, key: &'a str, val: &'a plist::Value) -> Result<Self, NsArchiveError> {
        let refs = nka.bind(<&plist::Dictionary>::decode(nka, key, val)?);
        Ok(Self {
            name: refs.resolve_opt::<String>("SiColorProfileArchiveICCNameKey"),
            icc_data: refs
                .resolve_opt::<&[u8]>("SiColorProfileArchiveICCDataKey")
                .map(<[u8]>::to_vec),
        })
    }
}

/// Splits `bytes` into exactly `N` little-endian values of `SIZE` bytes each.
pub(crate) fn le_array<T: FromLeBytes<SIZE>, const N: usize, const SIZE: usize>(
    bytes: &[u8],
    key: &str,
) -> Result<[T; N], NsArchiveError> {
    if bytes.len() != N * SIZE {
        return Err(NsArchiveError::TypeMismatch(key.to_string()));
    }
    Ok(std::array::from_fn(|i| {
        T::from_le_bytes(bytes[i * SIZE..(i + 1) * SIZE].try_into().unwrap())
    }))
}

pub(crate) trait FromLeBytes<const SIZE: usize> {
    fn from_le_bytes(bytes: [u8; SIZE]) -> Self;
}

impl FromLeBytes<4> for f32 {
    fn from_le_bytes(bytes: [u8; 4]) -> Self {
        f32::from_le_bytes(bytes)
    }
}

impl FromLeBytes<8> for f64 {
    fn from_le_bytes(bytes: [u8; 8]) -> Self {
        f64::from_le_bytes(bytes)
    }
}
