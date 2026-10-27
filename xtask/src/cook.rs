//! Cooks one model and its textures into a versioned artifact.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use vc_transport::{GameRoot, ImageArchive};

pub(crate) fn run(model: &str, txds: &[String], out: Option<&Path>) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;

    let geometries = vc_dff::decode_clump(fetch(&archive, model)?)
        .with_context(|| format!("failed to decode `{model}`"))?;

    let stem = Path::new(model).file_stem().map_or_else(
        || model.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );

    let mut scene = vc_cook::Scene::from_geometries(&stem, &geometries);

    for txd in txds {
        let dictionary = vc_txd::Dictionary::parse(fetch(&archive, txd)?)
            .with_context(|| format!("failed to parse `{txd}`"))?;

        for texture in &dictionary.textures {
            scene.textures.push(
                vc_cook::cook_texture(texture)
                    .with_context(|| format!("failed to cook `{}` from `{txd}`", texture.name))?,
            );
        }
    }

    let encoded = scene.encode()?;
    let path = out.map_or_else(
        || PathBuf::from(format!("vc-artifacts/cooked/{stem}.vcck")),
        Path::to_path_buf,
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create `{}`", parent.display()))?;
    }

    fs::write(&path, &encoded).with_context(|| format!("failed to write `{}`", path.display()))?;

    // Read it back rather than trusting the write.
    let read =
        fs::read(&path).with_context(|| format!("failed to re-read `{}`", path.display()))?;
    let decoded = vc_cook::Scene::decode(&read)?;

    ensure!(
        decoded == scene,
        "`{}` did not read back as it was written",
        path.display()
    );

    if decoded.meshes.is_empty() {
        bail!("`{model}` produced no meshes");
    }

    let texture_bytes: usize = decoded
        .textures
        .iter()
        .map(vc_cook::CookedTexture::byte_len)
        .sum();

    println!("cooked       {}", path.display());
    println!("  meshes     {}", decoded.meshes.len());
    println!("  vertices   {}", decoded.vertex_count());
    println!("  triangles  {}", decoded.triangle_count());
    println!("  textures   {}", decoded.textures.len());
    println!("  tex bytes  {texture_bytes}");
    println!("  file bytes {}", encoded.len());
    println!("  round trip verified");

    Ok(())
}

fn fetch<'a>(archive: &'a ImageArchive, name: &str) -> Result<&'a [u8]> {
    let indices = archive.indices_of(name);
    let Some(&index) = indices.first() else {
        bail!("`{name}` is not in the archive");
    };

    Ok(archive.data(index)?)
}
