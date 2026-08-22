#!/bin/sh
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
mkdir -p "$root/target"
scratch=$(mktemp -d "$root/target/sim-platform-ios-check.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
cp -R "$root/crates/sim-platform-ios" "$scratch/ios"
cp "$root/crates/sim-platform-ios/Cargo.lock" "$scratch/Cargo.lock"
printf '%s\n' '[workspace]' 'resolver = "3"' 'members = ["ios"]' '' \
  '[workspace.package]' 'edition = "2024"' 'license = "MPL-2.0"' \
  'repository = "https://github.com/sim-nest/sim-platform"' '' \
  '[workspace.lints.rust]' 'unsafe_code = "forbid"' '' \
  '[workspace.lints.clippy]' 'pedantic = "warn"' > "$scratch/Cargo.toml"
cargo=/home/bo/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$cargo" generate-lockfile --manifest-path "$scratch/Cargo.toml" --offline
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$cargo" test --manifest-path "$scratch/Cargo.toml" -p sim-platform-ios --locked --offline
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu CARGO="$cargo" /home/bo/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo-clippy clippy --manifest-path "$scratch/Cargo.toml" -p sim-platform-ios --all-targets --locked --offline -- -D warnings
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$cargo" doc --manifest-path "$scratch/Cargo.toml" -p sim-platform-ios --no-deps --locked --offline
"$root/crates/sim-platform-ios/scripts/cross-build.sh"
test "$(find "$root/crates/sim-platform-ios/attestations" -name '*.json' | wc -l)" -eq 3
rg -q 'sim_native_abi_v1' "$root/shell/ios/Sources/CSimPlatformIOS/include/sim_platform_ios.h"
rg -q 'startAccessingSecurityScopedResource' "$root/shell/ios/Sources/SimIOSShell/SimCapsule.swift"
rg -q 'sceneWillResignActive' "$root/shell/ios/Sources/SimIOSShell/SceneDelegate.swift"
rg -q 'terminal-closeout-required' "$root/crates/sim-platform-ios/contract/ios-build-provenance.json"
/home/bo/projects/sim-tooling/target/debug/xtask repo-contract --repo "$root" --check
