//! Decodes one texture from the install to a PNG, for looking at it.
//!
//! Built because "the textures are sideways" cannot be settled from statistics: the only
//! way to know whether the decoder or the UVs are wrong is to look at the pixels, and a
//! texture whose orientation is known removes the guesswork.

use std::path::Path;

use anyhow::{Context, Result};
use vc_cook::block::decode;
use vc_stream::Streamer;
use vc_transport::{GameRoot, ImageArchive};

pub(crate) fn run(name: &str, out: &Path) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut streamer = Streamer::open(archive);
    let indexed = streamer.warm_textures();

    let texture = streamer
        .texture("", name)
        .with_context(|| format!("no dictionary holds {name} after indexing {indexed}"))?;
    let level = texture.levels.first().context("the texture has no level")?;
    let texels = decode(
        texture.format,
        &level.data,
        u32::from(texture.width),
        u32::from(texture.height),
    );

    let mut rgba = Vec::with_capacity(texels.len() * 4);
    for texel in &texels {
        rgba.extend_from_slice(texel);
    }

    let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::from(texture.width),
        u32::from(texture.height),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .context("writing the PNG header")?
        .write_image_data(&rgba)
        .context("writing the PNG pixels")?;

    println!(
        "{}: {}x{} {:?} -> {}",
        texture.name,
        texture.width,
        texture.height,
        texture.format,
        out.display()
    );

    Ok(())
}
