//! Pretty-prints a `RenderWare` section tree.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rw_stream::{Section, SectionKind};
use vc_transport::{GameRoot, ImageArchive};

pub(crate) fn run(target: &str, max_depth: usize) -> Result<()> {
    let bytes = read_target(target)?;
    let root = Section::root(&bytes).context("not a RenderWare stream")?;

    print_section(&root, 0, max_depth);

    Ok(())
}

fn read_target(target: &str) -> Result<Vec<u8>> {
    if let Some(name) = target.strip_prefix("img:") {
        let root = GameRoot::from_env()?;
        let archive = ImageArchive::open(&root.image())?;
        let indices = archive.indices_of(name);
        let Some(&index) = indices.first() else {
            bail!("`{name}` is not in the archive");
        };

        return Ok(archive.data(index)?.to_vec());
    }

    let path = PathBuf::from(target);

    fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))
}

fn print_section(section: &Section<'_>, depth: usize, max_depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{indent}{:<20} size={:<8} stamp=0x{:08X}",
        kind_name(section.kind()),
        section.size(),
        section.stamp().0
    );

    if depth >= max_depth || !section.kind().is_container() {
        return;
    }

    let Ok(children) = section.children() else {
        return;
    };

    for child in children {
        match child {
            Ok(child) => print_section(&child, depth + 1, max_depth),
            Err(error) => println!("{indent}  <{error}>"),
        }
    }
}

const fn kind_name(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::STRUCT => "Struct",
        SectionKind::STRING => "String",
        SectionKind::EXTENSION => "Extension",
        SectionKind::CAMERA => "Camera",
        SectionKind::MATERIAL => "Material",
        SectionKind::MATERIAL_LIST => "MaterialList",
        SectionKind::FRAME_LIST => "FrameList",
        SectionKind::GEOMETRY => "Geometry",
        SectionKind::CLUMP => "Clump",
        SectionKind::ATOMIC => "Atomic",
        SectionKind::TEXTURE_NATIVE => "TextureNative",
        SectionKind::TEXTURE_DICTIONARY => "TextureDictionary",
        SectionKind::GEOMETRY_LIST => "GeometryList",
        SectionKind::SKIN => "Skin",
        SectionKind(0x0253_F2FE) => "FrameTag",
        SectionKind(0x50E) => "GeometryExtra",
        SectionKind(0x105) => "GeometryFlags",
        _ => "Unknown",
    }
}
