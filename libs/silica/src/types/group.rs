use plist::{Dictionary, Value};

use crate::{
    ns_archive::{NsArchive, NsDecode, NsObjects, error::NsArchiveError},
    types::hierarchy::SilicaHierarchy,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SilicaGroup {
    pub name: Option<String>,
    pub hidden: bool,
    /// Group opacity. Older files that predate group opacity decode as `1.0`.
    pub opacity: f32,
    pub locked: bool,
    /// Whether the group is collapsed in Procreate's layer panel.
    pub collapsed: bool,
    pub children: Vec<SilicaHierarchy>,
}

impl<'a> NsDecode<'a> for SilicaGroup {
    fn decode(nka: &'a NsArchive, key: &'a str, val: &'a Value) -> Result<Self, NsArchiveError> {
        let refs = nka.bind(<&'_ Dictionary>::decode(nka, key, val)?);

        Ok(Self {
            hidden: refs.resolve::<bool>("isHidden")?,
            name: refs.resolve::<Option<String>>("name")?,
            opacity: refs.resolve_opt::<f32>("opacity").unwrap_or(1.0),
            locked: refs.resolve_opt::<bool>("isLocked").unwrap_or(false),
            collapsed: refs.resolve_opt::<bool>("isCollapsed").unwrap_or(false),
            children: refs
                .resolve::<NsObjects<SilicaHierarchy>>("children")?
                .objects,
        })
    }
}
