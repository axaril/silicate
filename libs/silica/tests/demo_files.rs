//! Decodes the demo documents in `demo_files/` and checks the decoded
//! metadata against values inspected directly from the archives.

use std::{fs::File, io::Cursor, io::Read, path::PathBuf};

use silica::{BlendingMode, LayerKind, ProcreateFile, SilicaHierarchy, ns_archive::NsArchive};

fn load(name: &str) -> ProcreateFile {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo_files").join(name);
    let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
    let mut bytes = Vec::new();
    zip.by_name("Document.archive")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let nka = NsArchive::from_reader(Cursor::new(bytes)).unwrap();
    ProcreateFile::from_ns(&nka).unwrap()
}

const ALL: [&str; 5] = [
    "Mask_Test_File.procreate",
    "reference.procreate",
    "Reference_Blend_File.procreate",
    "Reference_Blend_File 2.procreate",
    "Reference_Blend_File 3.procreate",
];

#[test]
fn all_demo_files_decode_with_metadata() {
    for name in ALL {
        let file = load(name);
        assert_eq!(file.dpi, Some(300.0), "{name}");
        assert!(file.tracked_time.is_some_and(|t| t.as_secs() > 0), "{name}");
        let profile = file.color_profile.as_ref().expect(name);
        assert!(profile.name.is_some(), "{name}");
        assert!(profile.icc_data.as_ref().is_some_and(|d| d.len() > 128), "{name}");
        assert_eq!(file.composite.as_ref().unwrap().kind, Some(LayerKind::Composite), "{name}");
    }
}

#[test]
fn layer_masks_and_contents_rect() {
    let file = load("Mask_Test_File.procreate");
    assert_eq!(file.version, Some(2));
    let SilicaHierarchy::Layer(layer) = &file.layers[0] else { panic!() };
    assert_eq!(layer.kind, Some(LayerKind::Layer));
    assert_eq!(layer.mask.as_ref().unwrap().kind, Some(LayerKind::LayerMask));
    let rect = layer.contents_rect.unwrap();
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (626.0, 373.0, 835.0, 998.0));
    assert_eq!(file.composite.as_ref().unwrap().contents_rect, None);
}

fn walk<'a>(items: &'a [SilicaHierarchy], out: &mut Vec<&'a SilicaHierarchy>) {
    for item in items {
        out.push(item);
        if let SilicaHierarchy::Group(group) = item {
            walk(&group.children, out);
        }
    }
}

#[test]
fn groups_and_blend_modes() {
    // Older file without extendedBlend2 and newer file with it must agree.
    for name in ["Reference_Blend_File.procreate", "reference.procreate"] {
        let file = load(name);
        let mut all = Vec::new();
        walk(&file.layers, &mut all);
        let groups: Vec<_> = all
            .iter()
            .filter_map(|h| match h { SilicaHierarchy::Group(g) => Some(g), _ => None })
            .collect();
        assert!(!groups.is_empty(), "{name}");
        assert!(groups.iter().all(|g| g.opacity == 1.0), "{name}");
        assert!(groups.iter().any(|g| g.collapsed), "{name}");

        let blends: Vec<_> = all
            .iter()
            .filter_map(|h| match h { SilicaHierarchy::Layer(l) => Some(l.blend), _ => None })
            .collect();
        assert!(blends.contains(&BlendingMode::Divide), "{name}");
        assert!(blends.contains(&BlendingMode::HardMix), "{name}");
    }
}
