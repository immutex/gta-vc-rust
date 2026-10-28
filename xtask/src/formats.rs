//! Reports the texture format families the install actually uses.

use std::collections::BTreeMap;

use anyhow::Result;
use vc_transport::{GameRoot, ImageArchive};
use vc_txd::Dictionary;

pub(crate) fn run() -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut families: BTreeMap<(u32, u32, u32), u32> = BTreeMap::new();
    let mut alpha_head: BTreeMap<(u8, u8), u32> = BTreeMap::new();
    let mut block_textures = 0;
    let mut cutouts = 0;
    let mut total = 0;

    for entry in archive.entries() {
        if !entry.has_extension("txd") {
            continue;
        }

        let Some(&index) = archive.indices_of(entry.name()).first() else {
            continue;
        };
        let Ok(dictionary) = Dictionary::parse(archive.data(index)?) else {
            continue;
        };

        for texture in &dictionary.textures {
            let key = (
                texture.platform_id,
                texture.raster_format,
                texture.block_bytes().unwrap_or(0),
            );
            *families.entry(key).or_default() += 1;

            total += 1;
            if vc_cook::cook_texture(texture).is_ok_and(|cooked| cooked.has_alpha) {
                cutouts += 1;
            }

            if texture.block_bytes() == Some(16) {
                block_textures += 1;
                let head = texture.level_data.get(..2).unwrap_or(&[0, 0]);
                *alpha_head.entry((head[0], head[1])).or_default() += 1;
            }
        }
    }

    println!("texture families (platform, raster, block bytes) -> count");
    for (key, count) in &families {
        println!("  {key:?}: {count}");
    }

    println!("\ntextures needing alpha cutout: {cutouts} of {total}");
    println!("16-byte-block textures: {block_textures}");
    println!("first two bytes of level 0 (alpha block head), most common:");
    let mut heads: Vec<_> = alpha_head.into_iter().collect();
    heads.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

    for ((a, b), count) in heads.into_iter().take(8) {
        println!("  {a:02x} {b:02x}: {count}");
    }

    Ok(())
}
