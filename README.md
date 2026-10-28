<div align="center">

# vc-rs

*Vice City's assets, a 2026 engine.*

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.99.1-orange?style=flat-square)](rust-toolchain.toml)
[![Platform](https://img.shields.io/badge/platform-Windows%20(D3D12)%20%C2%B7%20Linux%20(Vulkan)-lightgrey?style=flat-square)](#requirements)
[![Status](https://img.shields.io/badge/status-pre--alpha-red?style=flat-square)](#status)

[Overview](#overview) · [Features](#features) · [Requirements](#requirements) · [Quick start](#quick-start) · [Development](#development) · [Status](#status)

</div>

## Overview

`vc-rs` reads the Grand Theft Auto: Vice City installation you already own and runs it on a modern
engine: Rust, Bevy and wgpu on D3D12 or Vulkan. The art, the map, the vehicle handling data and the
ped statistics are the originals — opened read-only and never modified. Everything around them is
new: a modern renderer, a fixed-step simulation, a streamed and GPU-culled world, and AI rebuilt on
the original's rules.

It is not a mod, not a decompilation and not an emulator. No game code is executed.

> [!IMPORTANT]
> `vc-rs` ships **no game assets**. You need your own copy of Vice City. Nothing from your
> installation is ever committed, bundled or redistributed.

## Features

- **Original assets, modern renderer** — the shipped DFF/TXD/COL/IPL data drawn with generated
  mip chains, sRGB BC textures, anisotropic filtering, a physically placed sun with four cascaded
  shadow maps and timecyc-driven ambient light.
- **Framerate independent** — a fixed 60 Hz simulation decoupled from rendering, so behaviour is
  identical at 30 fps or 240 fps.
- **The whole city, resident** — 1.9M triangles and 7,400 placements baked once into 71 merged
  chunks, so nothing pops in at a cell boundary and the far shore is visible past the original's
  2,000-unit fog wall.
- **A living city** — traffic, pedestrians and police driven by the shipped stats, groups and
  relationships. Missions are out of scope by design.
- **Read-only by construction** — the installation is opened, never patched.

## Requirements

| | |
|---|---|
| Game | Grand Theft Auto: Vice City, PC 1.0 — `models/gta3.img` must exist |
| OS | Windows 10/11 with a D3D12 GPU (primary); Linux with Vulkan |
| Toolchain | Rust **1.99.1** (pinned by `rust-toolchain.toml`) |
| GPU | 8 GB VRAM recommended for the 1440p performance target |

## Quick start

```bash
export VC_GAME_ROOT="/path/to/Grand Theft Auto Vice City"
cargo run -p vc_game --release
```

Point `VC_GAME_ROOT` at the folder containing `models/`, `data/` and `Audio/` — or drop the install at
`./gta-vc`. Nothing is written into the install: models and textures are read through a memory map and
cooked in memory.

Starts above the densest cell of the map and holds the whole city in memory. Pass a cell to start
somewhere else, and an optional hour: `cargo run -p vc_game -- 1 1 18`. The window reports frame
rate, draws, triangles and position:

| Input | Action |
|---|---|
| `F` | switch between flying and walking |
| `W` `A` `S` `D` | fly, or walk on foot |
| `Space` / `Ctrl` | rise and descend while flying; jump while walking |
| Hold left mouse | look around |
| `Shift` | fly four times faster, or move twice as fast on foot |

Startup prints what was cooked: placements kept, LOD and interior placements dropped, merged draws,
triangles and textures. `cargo run -p vc_game --example bake` runs the same bake without a window.

> [!NOTE]
> Pre-alpha: see [Status](#status) for what runs today.

## Project layout

| Path | Contents |
|---|---|
| `crates/vc_transport` | installation discovery and read-only archive access |
| `crates/vc_*` | one crate per format, plus world, render, sim, ai, audio and ui |
| `xtask` | repository automation, invoked as `cargo xtask` |
| `PLAN.md` | architecture, decisions, milestones, risks — the plan of record |

## Development

Build and test in WSL; run the game on Windows. Keep `target/` on the Linux filesystem, because
building across the Windows mount is several times slower:

```bash
export CARGO_TARGET_DIR="$HOME/.cache/vc-rs/target"
cargo xtask check
```

Rules for Rust code, commit messages and game data: see [CONTRIBUTING.md](CONTRIBUTING.md).

## Status

Pre-alpha, milestone **M0** — asset reconnaissance and toolchain. Nothing renders yet.

| Milestone | Goal |
|---|---|
| M0 | install census and RenderWare readers |
| M1 – M2 | first pixels, then the whole city |
| M3 – M4 | on foot, then vehicles |
| M5 – M7 | pedestrians, traffic, police, combat |
| M8 – M10 | audio, UI, optimisation, polish |

## Legal

Unofficial and unaffiliated with Rockstar Games or Take-Two Interactive. "Grand Theft Auto" and
"Vice City" are trademarks of their respective owners. This project contains no game assets, and
does not support their redistribution.

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.