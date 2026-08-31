#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
mode=${1:-all}
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
  'pedantic = "warn"' \
  '' \
  '[patch.crates-io]' \
  "sim-kernel = { path = \"$root/../sim-kernel\" }" \
  "sim-codec-binary = { path = \"$root/../sim-codecs/crates/sim-codec-binary\" }" \
  "sim-shape = { path = \"$root/../sim-shape\" }" \
  "sim-cookbook-build = { path = \"$root/../sim-foundation/crates/sim-cookbook-build\" }" \
  > "$scratch/Cargo.toml"

toolchain=stable-x86_64-unknown-linux-gnu
cargo=$(rustup which --toolchain "$toolchain" cargo)
case "$mode" in
  all|test)
    RUSTUP_TOOLCHAIN="$toolchain" "$cargo" \
      test --manifest-path "$scratch/Cargo.toml" -p sim-platform-android --offline
    ;;
esac
case "$mode" in
  all|compile)
    RUSTUP_TOOLCHAIN="$toolchain" "$cargo" \
      clippy --manifest-path "$scratch/Cargo.toml" --locked -p sim-platform-android \
      --all-targets --offline -- -D warnings
    ;;
esac
case "$mode" in
  all|doc)
    RUSTUP_TOOLCHAIN="$toolchain" "$cargo" \
      doc --manifest-path "$scratch/Cargo.toml" -p sim-platform-android --no-deps --offline
    ;;
esac
case "$mode" in
  all|test|compile|doc) ;;
  *) printf 'unknown focused-check mode: %s\n' "$mode" >&2; exit 2 ;;
esac
test "$(find "$root/crates/sim-platform-android/attestations" -name '*.json' | wc -l)" -eq 4
grep -q 'NativeLibAbiV1' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'sim_native_abi_v1' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'Java_org_simnest_shell_SimActivity_nativeCall' "$root/crates/sim-platform-android/src/ffi.rs"
grep -q 'System.loadLibrary' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
grep -q 'isOnDeviceRecognitionAvailable' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
grep -q 'createOnDeviceSpeechRecognizer' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
grep -q 'isNetworkConnectionRequired' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"
if grep -q 'createSpeechRecognizer' "$root/shell/android/app/src/main/java/org/simnest/shell/SimActivity.kt"; then
    echo 'remote-capable Android recognizer construction is forbidden' >&2
    exit 1
fi
grep -q 'android-emulator-runner' "$root/.github/workflows/ci.yml"
