#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
scratch=$(mktemp -d /tmp/sim-platform-android-check.XXXXXX)
cp -R "$root/crates/sim-platform-android" "$scratch/android"
cp -R "$root/crates/sim-platform-core" "$scratch/core"
sed -i 's#path = "../sim-platform-core"#path = "../core"#' "$scratch/android/Cargo.toml"
printf '%s\n' \
  '[workspace]' \
  'resolver = "3"' \
  'members = ["android", "core"]' \
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
  test --manifest-path "$scratch/Cargo.toml" -p sim-platform-android --offline
test "$(find "$root/crates/sim-platform-android/attestations" -name '*.json' | wc -l)" -eq 4
test "$(grep -c 'NativeLibAbiV1::call' "$root/crates/sim-platform-android/src/lib.rs")" -eq 1
grep -q 'sim_native_abi_v1' "$root/crates/sim-platform-android/native/sim_native_abi.c"
grep -q 'System.loadLibrary' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
