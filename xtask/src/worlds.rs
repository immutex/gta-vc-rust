//! Reading the map's text files into a [`World`].
//!
//! Five commands need this and previously each carried its own copy; the map is the one
//! input every diagnostic shares, so it lives here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use vc_world::World;

/// Reads every IDE and IPL under `directory`.
pub(crate) fn load(directory: &Path) -> Result<World> {
    Ok(load_with_effects(directory)?.0)
}

/// Reads every IDE and IPL, and the `2dfx` effects declared alongside the definitions.
pub(crate) fn load_with_effects(directory: &Path) -> Result<(World, Vec<vc_world::Effect>)> {
    let mut world = World::default();
    let mut effects = Vec::new();

    for path in files(directory)? {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);

        match path.extension().and_then(|found| found.to_str()) {
            Some("ide") => {
                world
                    .definitions
                    .extend(vc_world::parse_definitions(&text)?);
                effects.extend(vc_world::parse_effects(&text)?);
            }
            Some("ipl") => world.instances.extend(vc_world::parse_instances(&text)?),
            _ => {}
        }
    }

    Ok((world, effects))
}

fn files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    collect(directory, &mut found)?;

    Ok(found)
}

fn collect(directory: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("listing {}", directory.display()))?
    {
        let path = entry?.path();

        if path.is_dir() {
            collect(&path, out)?;
        } else if matches!(
            path.extension().and_then(|found| found.to_str()),
            Some("ide" | "ipl")
        ) {
            out.push(path);
        }
    }

    Ok(())
}
