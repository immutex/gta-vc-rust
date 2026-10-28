# Contributing

## Requirements

| Need | Detail |
|---|---|
| Rust | 1.99.1, pinned by `rust-toolchain.toml` — install via [rustup](https://rustup.rs) |
| Build and test host | WSL (Linux). CI and tests run here |
| Run host | Windows (D3D12). The game is Windows-first |
| Game data | Your own Vice City 1.0 install, outside version control |
| Optional | `cargo install cargo-deny`; `cmake` + `clang` for the texture-encoder cook |

## Setup

Keep build output off the Windows mount — building through `/mnt/c` is several times slower:

```bash
export CARGO_TARGET_DIR="$HOME/.cache/vc-rs/target"
cargo xtask check
```

Point the runtime at your installation, either with `VC_GAME_ROOT` or by placing it at `./gta-vc`:

```powershell
$env:VC_GAME_ROOT = "C:\path\to\Grand Theft Auto Vice City"
cargo run --release
```

Enable the commit-message hook once per clone:

```bash
git config core.hooksPath .githooks
```

## Commands

| Task | Command |
|---|---|
| Full gate | `cargo xtask check` |
| Format | `cargo fmt --all` |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Test | `cargo test --workspace` |
| Licences and duplicates | `cargo deny check` |
| Install census | `cargo xtask census --verify` |

## Rust rules

### Errors

- Libraries return `Result<T, E>` with a `thiserror` enum; only binaries may use `anyhow`.
- No `unwrap`, `expect` or `panic!` outside tests. Use `?`, `let ... else`, or `unwrap_or_else`.
- Document failure modes with a `# Errors` section on every public fallible function.
- Use `unreachable!` only when the invariant is argued in a comment; never as error handling.

### Comments

- The comment ratio stays low. Code is expected to read without narration.
- `//` comments explain non-obvious *why*, and start with a tag: `SAFETY:`, `PERF:`, `WORKAROUND:`,
  `CONTEXT:`. Link a design reference where one exists.
- `///` documents every public item — purpose, invariants, usage examples — not the implementation.
- No commented-out code. A `TODO` must reference an issue: `// TODO(#12): ...`.
- If a comment explains what a block does, extract a named function instead.

### Ownership

- Borrow in parameters: `&str`, `&[T]`, `&T`. Take ownership only when you keep it.
- Derive `Copy` only for small plain-data types (≤ 24 bytes) that are not iterators.
- No cloning to appease the borrow checker. `.clone()` in a loop needs a reason.
- Use `_else` variants when the default is expensive: `ok_or_else`, `unwrap_or_else`.

### Lints

- Warnings are errors: `-D warnings` is part of `cargo xtask check`.
- Never `#[allow(...)]`. Use `#[expect(lint, reason = "...")]` so the expectation fails when stale.
- `unsafe` is denied workspace-wide.
- Do not silence a lint to land a change; fix the code or change the design.

### Tests

- One behaviour per test, named as a sentence: `rejects_directory_without_image`.
- `unwrap` is fine in tests; keep each test's action and assertion inline.
- A test that needs real game data reads `VC_GAME_ROOT` and is skipped when it is unset.
  Committed fixtures are synthetic and hand-written.
- Prefer `assert_eq!` and `assert_matches!` with a message that prints the actual state.

### Dependencies

- Add to `[workspace.dependencies]` with an explicit version; crates use `dep.workspace = true`.
- Permissive licences only — see `deny.toml`. `cargo deny check` is the gate.

## Commit convention

`<type>(<scope>): <subject>`

| Type | Use for |
|---|---|
| `feat` | new behaviour |
| `fix` | bug fix |
| `docs` | documentation only |
| `refactor` | behaviour-preserving change |
| `perf` | performance work |
| `test` | tests only |
| `build` | dependencies, toolchain, build config |
| `ci` | CI configuration |
| `chore` | anything else that is not source behaviour |

Scopes name a crate or area without the `vc_` prefix: `transport`, `txd`, `dff`, `col`, `world`,
`render`, `sim`, `ai`, `audio`, `ui`, `repo`, `plan`.

```
feat(transport): read IMG v1 directory entries
fix(col): step records by the declared size
docs(plan): record the M2 performance gate
chore(repo): bump the pinned toolchain
```

Imperative mood, subject ≤ 72 characters, one logical change per commit, body explains *why* when
the subject cannot. The `commit-msg` hook enforces the format; run `cargo xtask check` before
committing.

## Game data policy

- Never commit, attach or publish anything from `gta-vc/`, `vc-artifacts/`, or any cooked, converted
  or upscaled artifact derived from the game — in commits, pull requests, issues or release assets.
- Do not read `re3`/`reVC` sources, in any fork. They are decompiled from the original binaries and
  were taken down by Take-Two in 2021; the surviving fork says so itself and carries an MIT licence
  with a Proof-of-Usage condition aimed at automated systems. Building and *running* it as a
  black-box oracle is fine — reading, quoting or porting it is not (`PLAN.md` section 12).
- Do not read GPL implementations of the file formats (`rw-parser-rs`, OpenRW) either.
- Reference screenshots of the running original are fine locally; do not commit them.

## Architecture

`PLAN.md` is the plan of record: decisions, milestone gates, the fidelity ledger and the risk
register. Change the plan there rather than in discussion.

## Build directory

Cargo never removes superseded artifacts, and this workspace builds Bevy at
`opt-level = 3`, so the target directory grows by gigabytes per week of iteration.
`[profile.dev]` therefore uses `debug = "line-tables-only"` rather than full
debuginfo, which is the difference between roughly 40 GB and a few.

Two habits keep it in check:

| Command | Effect |
|---|---|
| `cargo xtask disk` | Drops every `incremental/` directory (pure cache, safe) and prints before and after sizes |
| `cargo xtask disk --all` | Drops all build artifacts; the next build is cold |

`cargo xtask check` prints a warning when the target directory passes 12 GiB, so the
growth is visible rather than discovered when the disk fills.
