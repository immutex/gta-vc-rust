//! Repository automation, run as `cargo xtask <command>`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

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
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Check => check(),
        Cmd::Census { verify } => census(verify),
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

fn census(verify: bool) -> Result<()> {
    let _ = verify;

    bail!("census is not implemented yet; it lands in M0 step 3 (PLAN.md section 19)")
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
