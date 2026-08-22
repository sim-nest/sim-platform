#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
scratch=$(mktemp -d /tmp/sim-platform-android-check.XXXXXX)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
cp -R "$root/crates/sim-platform-android" "$scratch/android"
cp "$root/crates/sim-platform-android/Cargo.lock" "$scratch/Cargo.lock"
printf '%s\n' \
  '[workspace]' \
  'resolver = "3"' \
  'members = ["android"]' \
  '' \
  '[workspace.package]' \
  'edition = "2024"' \
  'license = "MPL-2.0"' \
  'repository = "https://github.com/sim-nest/sim-platform"' \
  '' \
  '[workspace.lints.rust]' \
  'unsafe_code = "forbid"' \
  '' \
  '[workspace.lints.clippy]' \
  'pedantic = "warn"' > "$scratch/Cargo.toml"

toolchain=/home/bo/.rustup/toolchains/stable-x86_64-unknown-linux-gnu
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$toolchain/bin/cargo" \
  test --manifest-path "$scratch/Cargo.toml" --locked -p sim-platform-android --offline
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$toolchain/bin/cargo" \
  clippy --manifest-path "$scratch/Cargo.toml" --locked -p sim-platform-android \
  --all-targets --offline -- -D warnings
RUSTUP_TOOLCHAIN=stable-x86_64-unknown-linux-gnu "$toolchain/bin/cargo" \
  doc --manifest-path "$scratch/Cargo.toml" --locked -p sim-platform-android --no-deps --offline
test "$(find "$root/crates/sim-platform-android/attestations" -name '*.json' | wc -l)" -eq 4
grep -q 'NativeLibAbiV1' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'sim_native_abi_v1' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'Java_org_simnest_shell_SimActivity_nativeCall' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'System.loadLibrary' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
grep -q 'android-emulator-runner' "$root/.github/workflows/ci.yml"
