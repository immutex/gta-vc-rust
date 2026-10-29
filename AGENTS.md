# Agent Instructions

## Environment
- Build and test in WSL; the game runs on Windows (D3D12). WSL has no GPU rendering path.
- Keep build output off the Windows mount: `export CARGO_TARGET_DIR="$HOME/.cache/vc-rs/target"`.
- The toolchain is pinned by `rust-toolchain.toml`; bump it deliberately, never casually.
- The game install lives at `./gta-vc` or at `$VC_GAME_ROOT`. It is never committed.

## Commands
| Task | Command |
|------|---------|
| Full gate (fmt, clippy, test, deny) | `cargo xtask check` |
| Lint only | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| Test one crate | `cargo test -p vc_transport` |
| Test one case | `cargo test -p vc_transport rejects_directory_without_image` |
| Format | `cargo fmt --all` |
| Licence and duplicate check | `cargo deny check` |
| Install census (needs game data) | `cargo xtask census --verify` |
| Boot the runtime and fail on a panic | `./scripts/smoke.sh` |

## External References
| Need | File |
|------|------|
| Plan of record: decisions, milestones, risks | `PLAN.md` |
| Rust rules, commit format, setup | `CONTRIBUTING.md` |
| Project overview | `README.md` |

## Key Conventions
- Never commit or attach anything under `gta-vc/` or `vc-artifacts/`, in commits, issues or releases.
- Commits: `<type>(<scope>): <subject>`; scopes are crate or area names, e.g. `feat(transport): ...`.
- No `unwrap`, `expect` or `panic!` outside tests.
- Comments explain non-obvious *why* only; public items carry `///`. Do not narrate code.
- Use `#[expect(lint, reason = "...")]`, never `#[allow]`.
- New dependencies go in `[workspace.dependencies]` with an explicit version; permissive licences only.
- Never read `re3`/`reVC` sources (any fork) or GPL implementations. `reVC` may be *run* as a black-box oracle only; see `PLAN.md` section 12.
- Record plan or decision changes in `PLAN.md`, not in chat.
