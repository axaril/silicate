use crate::data::{BlendingMode, LayerKind, Rect};
use crate::ns_archive::{NsArchive, NsDecode, NsRefDictionary, error::NsArchiveError};
use plist::{Dictionary, Value};

impl NsDecode<'_> for BlendingMode {
    fn decode(nka: &NsArchive, key: &str, val: &Value) -> Result<Self, NsArchiveError> {
        let mode = u32::decode(nka, key, val)?;
        BlendingMode::from_u32(mode)
            .ok_or_else(|| NsArchiveError::BadValue(key.to_string(), mode.to_string()))
    }
}

/// Blend mode keys, newest first.
///
/// Procreate adds a new key when it introduces blend modes that older
/// readers can't represent (`blend` only goes up to 19, `extendedBlend`
/// adds 20-26, `extendedBlend2` appears in newer files). The older keys are
/// still written, so a mode this library doesn't recognize under a newer
/// key can fall back to the closest mode under an older one.
const BLEND_KEYS: [&str; 3] = ["extendedBlend2", "extendedBlend", "blend"];

fn resolve_blend(refs: &NsRefDictionary<'_>) -> Result<BlendingMode, NsArchiveError> {
    let mut first_err = None;
    for key in BLEND_KEYS {
        match refs.resolve::<Option<BlendingMode>>(key) {
            Ok(Some(mode)) => return Ok(mode),
            Ok(None) => {}
            Err(err) => {
                first_err.get_or_insert(err);
            }
        }
    }
    Err(first_err.unwrap_or_else(|| NsArchiveError::MissingKey("blend".to_string())))
}

#[derive(Debug, Clone, PartialEq)]
pub struct SilicaLayer {
    // animationHeldLength:Int?
    pub blend: BlendingMode,
    // bundledImagePath:String?
    // bundledMaskPath:String?
    // bundledVideoPath:String?
    pub clipped: bool,
    /// Bounding box of the layer's painted pixels, when Procreate has
    /// computed one (`contentsRectValid`).
    pub contents_rect: Option<Rect>,
    // document:SilicaDocument?
    pub hidden: bool,
    /// Role of this layer object. `None` if the file has no `type` key.
    pub kind: Option<LayerKind>,
    pub locked: bool,
    pub mask: Option<Box<SilicaLayer>>,
    pub name: Option<String>,
    pub opacity: f32,
    // perspectiveAssisted:Bool?
    /// Alpha lock: painting only affects existing pixels (`preserve`).
    pub alpha_locked: bool,
    // private:Bool?
    // sizeWidth/sizeHeight: Int? (matches canvas size in all observed files)
    // text:ValkyrieText?
    // textPDF:Data?
    // textureSet:Any?
    // transform:Data? (4x4 f64 matrix, identity in all observed files)
    pub uuid: String,
    pub version: u64,
    // videoTime:NSDictionary?
}

impl<'a> NsDecode<'a> for SilicaLayer {
    fn decode(nka: &'a NsArchive, key: &'a str, val: &'a Value) -> Result<Self, NsArchiveError> {
        let refs = nka.bind(<&'_ Dictionary>::decode(nka, key, val)?);

        let contents_rect = refs
            .resolve_opt::<bool>("contentsRectValid")
            .unwrap_or(false)
            .then(|| refs.resolve_opt::<Rect>("contentsRect"))
            .flatten();

        Ok(Self {
            blend: resolve_blend(&refs)?,
            clipped: refs.resolve::<bool>("clipped")?,
            contents_rect,
            hidden: refs.resolve::<bool>("hidden")?,
            kind: refs.resolve_opt::<LayerKind>("type"),
            locked: refs.resolve_opt::<bool>("locked").unwrap_or(false),
            mask: refs.resolve::<Option<Box<SilicaLayer>>>("mask")?,
            name: refs.resolve::<Option<String>>("name")?,
            opacity: refs.resolve::<f32>("opacity")?,
            alpha_locked: refs.resolve_opt::<bool>("preserve").unwrap_or(false),
            uuid: refs.resolve::<String>("UUID")?,
            version: refs.resolve::<u64>("version")?,
        })
    }
}
