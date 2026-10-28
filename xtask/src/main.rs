//! Repository automation, run as `cargo xtask <command>`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod census;
mod cook;
mod disk;
mod dump;
mod facing;
mod formats;
mod models;
mod physics;
mod stream;
mod verify;
mod worlds;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Repository automation for vc-rs")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Format, lint, test and check licences.
    Check,
    /// Summarise the game installation, and optionally verify it.
    Census {
        /// Fail when the install does not match the totals in PLAN.md.
        #[arg(long)]
        verify: bool,
        /// Print per-file detail for the text formats.
        #[arg(long)]
        verbose: bool,
    },
    /// List the models a cell places, with cutout and collision status.
    Models {
        /// Cell x index.
        x: i32,
        /// Cell y index.
        y: i32,
    },
    /// Decide the instance rotation convention from lamp arms and the road graph.
    Facing,
    /// Cross-check a cell's placements against the map's collision data.
    Verify {
        /// Restrict to one cell, as x y.
        #[arg(long, num_args = 2)]
        cell: Option<Vec<i32>>,
    },
    /// Report the texture format families the install uses.
    Formats {
        /// Only print textures whose name contains this string.
        #[arg(long)]
        name: Option<String>,
    },
    /// Build collision for a region and probe the ground.
    Physics {
        /// Cells to load in each direction.
        #[arg(long, default_value_t = 1)]
        radius: i32,
    },
    /// Cook the region around the busiest cell and report what resolved.
    Stream {
        /// Cells to load in each direction.
        #[arg(long, default_value_t = 1)]
        radius: i32,
    },
    /// Cook one model and its textures into a versioned artifact.
    Cook {
        /// Model name in the archive, for example `landstal.dff`.
        model: String,
        /// Texture dictionaries to cook alongside it.
        #[arg(long = "txd")]
        txds: Vec<String>,
        /// Output path; defaults to `vc-artifacts/cooked/<model>.vcck`.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Report and prune build artifacts that have outgrown their budget.
    Disk {
        /// Remove every artifact, not just the incremental cache.
        #[arg(long)]
        all: bool,
    },
    /// Print the `RenderWare` section tree of a file or archive entry.
    Dump {
        /// A file path, or `img:<name>` to read an entry from `models/gta3.img`.
        target: String,
        /// Deepest level to print.
        #[arg(long, default_value_t = 4)]
        depth: usize,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Check => check(),
        Cmd::Census { verify, verbose } => census::run(verify, verbose),
        Cmd::Cook { model, txds, out } => cook::run(&model, &txds, out.as_deref()),
        Cmd::Stream { radius } => stream::run(radius),
        Cmd::Physics { radius } => physics::run(radius),
        Cmd::Formats { name } => formats::run(name.as_deref()),
        Cmd::Models { x, y } => models::run(x, y),
        Cmd::Verify { cell } => verify::run(cell.and_then(|values| match values.as_slice() {
            [x, y] => Some((*x, *y)),
            _ => None,
        })),
        Cmd::Facing => facing::run(),
        Cmd::Disk { all } => disk::run(all),
        Cmd::Dump { target, depth } => dump::run(&target, depth),
    }
}

fn check() -> Result<()> {
    disk::warn_if_bloated(&target_dir());
    run("cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run("cargo", &["test", "--workspace"])?;
    run("cargo", &["deny", "check"])?;

    Ok(())
}

/// Where cargo writes build output, honouring the environment override.
pub(crate) fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map_or_else(|| PathBuf::from("target"), |root| root.join("target"))
        },
        PathBuf::from,
    )
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn `{program}`"))?;

    if !status.success() {
        bail!("`{program} {}` failed: {status}", args.join(" "));
    }

    Ok(())
}
