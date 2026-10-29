#!/usr/bin/env bash
# Boots the runtime for a few frames and fails on a panic.
#
# `cargo xtask check` compiles and tests, which does not catch a system that panics the
# moment the app runs - three startup defects reached the player that way: a duplicate
# component, a camera pointed at open sea, and two queries that were not disjoint. This
# renders one frame offscreen and reads the log.
#
# Needs a Vulkan driver. In WSL, lavapipe is enough:
#   export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/vc-rs/target}"
export VC_GAME_ROOT="${VC_GAME_ROOT:-$root/gta-vc}"

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

cargo run -q --manifest-path "$root/Cargo.toml" -p vc_game --example shot -- \
    "$log.png" 1 1 >"$log" 2>&1 || true

if grep -qE "panicked|Validation Error|Quitting the application" "$log"; then
    echo "smoke: FAILED"
    grep -E "panicked|Validation Error|ERROR" "$log" | head -10
    exit 1
fi

if ! grep -q "wrote" "$log"; then
    echo "smoke: no frame was written"
    tail -10 "$log"
    exit 1
fi

echo "smoke: ok"
grep -E "bake:|image:|far plane|water:" "$log" | head -6
rm -f "$log.png"
