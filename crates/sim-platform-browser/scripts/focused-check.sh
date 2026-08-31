#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
mkdir -p "$root/target"
scratch=$(mktemp -d "$root/target/sim-platform-browser-check.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
cp -R "$root/crates/sim-platform-browser" "$scratch/browser"
printf '%s\n' '[workspace]' 'resolver = "3"' 'members = ["browser"]' '' \
  '[workspace.package]' 'edition = "2024"' 'license = "MPL-2.0"' \
  'repository = "https://github.com/sim-nest/sim-platform"' '' \
  '[workspace.lints.rust]' 'unsafe_code = "forbid"' '' \
  '[workspace.lints.clippy]' 'pedantic = "warn"' > "$scratch/Cargo.toml"
toolchain=stable-x86_64-unknown-linux-gnu
cargo=$(rustup which --toolchain "$toolchain" cargo)
RUSTUP_TOOLCHAIN="$toolchain" "$cargo" generate-lockfile --manifest-path "$scratch/Cargo.toml" --offline
RUSTUP_TOOLCHAIN="$toolchain" "$cargo" test --manifest-path "$scratch/Cargo.toml" -p sim-platform-browser --locked --offline
RUSTUP_TOOLCHAIN="$toolchain" "$cargo" clippy --manifest-path "$scratch/Cargo.toml" -p sim-platform-browser --all-targets --locked --offline -- -D warnings
RUSTUP_TOOLCHAIN="$toolchain" "$cargo" doc --manifest-path "$scratch/Cargo.toml" -p sim-platform-browser --no-deps --locked --offline
node "$root/shell/browser/browser-capsule.test.mjs"
test "$(jq -r '.semantic_exports | join(" ")' "$root/crates/sim-platform-browser/contract/wasm-imports.json")" = sim_browser_named_call
test "$(jq '.imports | length' "$root/crates/sim-platform-browser/contract/wasm-imports.json")" -eq 0
rg -q 'presentation_owner.*sim-web' "$root/crates/sim-platform-browser/contract/wasm-imports.json"
rg -q '"retained": true' "$root/crates/sim-platform-browser/attestations/browser-offline.json"
