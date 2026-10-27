//! Repository automation, run as `cargo xtask <command>`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod census;
mod cook;
mod dump;

use std::path::PathBuf;
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
        Cmd::Dump { target, depth } => dump::run(&target, depth),
    }
}

fn check() -> Result<()> {
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
