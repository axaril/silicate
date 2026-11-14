use std::time::Duration;

use crate::{
    data::{ColorProfile, Flipped, Orientation, le_array},
    error::SilicaError,
    ns_archive::{NsArchive, NsObjects, Size},
    types::{hierarchy::SilicaHierarchy, layer::SilicaLayer},
};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcreateFile {
    pub author_name: Option<String>,
    pub background_hidden: bool,
    pub background_color: [f32; 4],
    //     backgroundColorHSBA: Data? (same color as backgroundColor, in HSBA)
    //     closedCleanlyKey:Bool?
    /// Color profile the pixel data is stored in.
    pub color_profile: Option<ColorProfile>,
    /// Canvas resolution in dots per inch (`SilicaDocumentArchiveDPIKey`).
    pub dpi: Option<f32>,
    //     animation: ValkyrieDocumentAnimation?
    // //  public var drawingguide
    //     faceBackgroundHidden:Bool?
    //     featureSet:Int?
    pub flipped: Flipped,
    //     mask:SilicaLayer? (document-level mask, LayerKind::DocumentMask)
    pub name: Option<String>,
    pub orientation: Orientation,
    //     primaryItem:Any?
    // //  skipping a bunch of reference window related stuff here
    //     selectedLayer:Any?
    //     selectedSamplerLayer:SilicaLayer?
    //     SilicaDocumentArchiveUnitKey:Int?
    /// Total time spent working on the document (`SilicaDocumentTrackedTimeKey`).
    pub tracked_time: Option<Duration>,
    //     SilicaDocumentVideoPurgedKey:Bool?
    //     SilicaDocumentVideoSegmentInfoKey:VideoSegmentInfo? // not finished
    //     solo: SilicaLayer?
    pub stroke_count: usize,
    //     userSavedSelectionMasks: [Any]?
    /// Document archive format version. Observed values: 1, 2.
    pub version: Option<u64>,
    //     videoEnabled: Bool? = true
    //     videoQualityKey: String?
    //     videoResolutionKey: String?
    //     videoDuration: String? = "Calculating..."
    pub tile_size: u32,

    pub size: Size<u32>,

    pub layers: Vec<SilicaHierarchy>,
    pub composite: Option<SilicaLayer>,
}

impl ProcreateFile {
    pub fn from_ns<'a>(nka: &'a NsArchive) -> Result<Self, SilicaError> {
        let refs = nka.bind(nka.root()?);

        let size = refs.resolve::<Size<u32>>("size")?;
        let tile_size = refs.resolve::<u32>("tileSize")?;

        let layers = refs
            .resolve::<NsObjects<SilicaHierarchy>>("unwrappedLayers")?
            .objects;

        Ok(Self {
            author_name: refs.resolve::<Option<String>>("authorName")?,
            background_hidden: refs.resolve::<bool>("backgroundHidden")?,
            stroke_count: refs.resolve::<usize>("strokeCount")?,
            background_color: le_array::<f32, 4, 4>(
                refs.resolve::<&[u8]>("backgroundColor")?,
                "backgroundColor",
            )?,
            color_profile: refs.resolve_opt::<ColorProfile>("colorProfile"),
            dpi: refs
                .resolve_opt::<f32>("SilicaDocumentArchiveDPIKey")
                .filter(|dpi| dpi.is_finite() && *dpi > 0.0),
            name: refs.resolve::<Option<String>>("name")?,
            orientation: refs.resolve::<Orientation>("orientation")?,
            flipped: Flipped {
                horizontally: refs.resolve::<bool>("flippedHorizontally")?,
                vertically: refs.resolve::<bool>("flippedVertically")?,
            },
            tracked_time: refs
                .resolve_opt::<f64>("SilicaDocumentTrackedTimeKey")
                .and_then(|secs| Duration::try_from_secs_f64(secs).ok()),
            version: refs.resolve_opt::<u64>("version"),
            tile_size,
            composite: Some(refs.resolve::<SilicaLayer>("composite")?),
            layers,
            size,
        })
    }
}
