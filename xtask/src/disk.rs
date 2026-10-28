//! Keeps the build directory from growing without bound.
//!
//! Cargo never deletes superseded artifacts, so a directory that starts at two
//! gigabytes reaches fifty after a week of iteration: every change to every crate
//! leaves the previous objects behind. Incremental state is the worst offender, since
//! it is pure cache and costs only a slower next build to discard.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Above this the directory is reported as needing a prune.
pub(crate) const BUDGET_BYTES: u64 = 12 * 1024 * 1024 * 1024;

/// Removes the parts of the target directory that are pure rebuildable cache.
pub(crate) fn run(all: bool) -> Result<()> {
    let target = crate::target_dir();

    if !target.is_dir() {
        println!("no target directory at {}", target.display());

        return Ok(());
    }

    println!("target directory: {}", target.display());
    println!("  size before   {}", human(size_of(&target)?));

    if all {
        measure(&target, "removed", prune_all(&target)?)?;
    } else {
        let mut freed = 0;

        for profile in ["debug", "release"] {
            freed += prune_incremental(&target.join(profile))?;
        }

        measure(&target, "released", freed)?;
    }

    Ok(())
}

/// Warns when the target directory has outgrown its budget, without failing the gate.
pub(crate) fn warn_if_bloated(target: &Path) {
    let Ok(size) = size_of(target) else {
        return;
    };

    if size > BUDGET_BYTES {
        println!(
            "warning: {} is {} (budget {}); run `cargo xtask disk` to prune it",
            target.display(),
            human(size),
            human(BUDGET_BYTES)
        );
    }
}

fn measure(target: &Path, verb: &str, freed: u64) -> Result<()> {
    println!("  {verb}        {}", human(freed));
    println!("  size after    {}", human(size_of(target)?));

    Ok(())
}

fn prune_incremental(profile: &Path) -> Result<u64> {
    let incremental = profile.join("incremental");

    if !incremental.is_dir() {
        return Ok(0);
    }

    let freed = size_of(&incremental)?;
    fs::remove_dir_all(&incremental)
        .with_context(|| format!("failed to remove {}", incremental.display()))?;

    Ok(freed)
}

/// Drops every build artifact, keeping only the directory itself.
fn prune_all(target: &Path) -> Result<u64> {
    let freed = size_of(target)?;

    for entry in fs::read_dir(target)? {
        let path = entry?.path();

        if path.is_dir() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    Ok(freed)
}

fn size_of(path: &Path) -> Result<u64> {
    let mut total = 0;

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;

        total += if meta.is_dir() {
            size_of(&entry.path())?
        } else {
            meta.len()
        };
    }

    Ok(total)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    format!("{value:.1} {}", UNITS[unit])
}
