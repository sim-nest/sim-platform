#!/bin/sh
set -eu
mode=${1:---check}
case "$mode" in --capture|--check) ;; *) exit 64 ;; esac
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
crate="$root/crates/sim-platform-ios"
toolchain=stable-x86_64-unknown-linux-gnu
rustc=$(rustup which --toolchain "$toolchain" rustc)
targets="aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios"
actual=$(mktemp -d /tmp/sim-platform-ios-targets.XXXXXX)
trap 'rm -rf "$actual"' EXIT HUP INT TERM
test "$($rustc --version --verbose | sed -n 's/^release: //p')" = 1.97.1
for target in $targets; do
    RUSTC_BOOTSTRAP=1 "$rustc" -Z unstable-options --print target-spec-json --target "$target" > "$actual/$target.json"
    digest=$(sha256sum "$actual/$target.json" | cut -d' ' -f1)
    rg -q "\"$target\": \"$digest\"" "$crate/contract/ios-build-provenance.json"
done
workspace="$actual/workspace"
mkdir -p "$workspace"
cp -R "$crate" "$workspace/ios"
cp "$crate/Cargo.lock" "$workspace/Cargo.lock"
sed -i 's/crate-type = \["rlib", "staticlib", "cdylib"\]/crate-type = ["staticlib"]/' "$workspace/ios/Cargo.toml"
printf '%s\n' '[workspace]' 'resolver = "3"' 'members = ["ios"]' '' \
  '[workspace.package]' 'edition = "2024"' 'license = "MPL-2.0"' \
  'repository = "https://github.com/sim-nest/sim-platform"' '' \
  '[workspace.lints.rust]' 'unsafe_code = "forbid"' '' \
  '[workspace.lints.clippy]' 'pedantic = "warn"' > "$workspace/Cargo.toml"
# The owner proof builds the static archive through the isolated package check;
# Apple SDK linking and simulator execution are hosted-closeout evidence.
cargo=$(rustup which --toolchain "$toolchain" cargo)
RUSTUP_TOOLCHAIN="$toolchain" "$cargo" generate-lockfile --manifest-path "$workspace/Cargo.toml" --offline
for target in $targets; do
    RUSTUP_TOOLCHAIN="$toolchain" RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR="$workspace/target" RUSTFLAGS="--remap-path-prefix=$workspace=." \
      "$cargo" rustc --manifest-path "$workspace/Cargo.toml" -Z build-std=std,panic_abort \
      -p sim-platform-ios --locked --release --target "$target" --offline -- --crate-type staticlib
    archive="$workspace/target/$target/release/libsim_platform_ios.a"
    test -s "$archive"
    /usr/lib/llvm-20/bin/llvm-nm --defined-only --extern-only "$archive" | rg -q 'sim_native_abi_v1$'
    /usr/lib/llvm-20/bin/llvm-nm --defined-only --extern-only "$archive" | rg -q 'sim_ios_encode_input_json$'
    /usr/lib/llvm-20/bin/llvm-nm --undefined-only --extern-only --format=posix "$archive" | LC_ALL=C sort -u > "$actual/$target.undefined"
    if rg -i 'UIApplication|UIScene|AVAudioSession|UNUserNotificationCenter|startAccessingSecurityScopedResource' "$actual/$target.undefined"; then exit 1; fi
    spec_digest=$(sha256sum "$actual/$target.json" | cut -d' ' -f1)
    artifact_digest=$(sha256sum "$archive" | cut -d' ' -f1)
    imports_digest=$(sha256sum "$actual/$target.undefined" | cut -d' ' -f1)
    imports_count=$(wc -l < "$actual/$target.undefined" | tr -d ' ')
    printf '{"schema":"sim.platform-cross-build-attestation/v1","target":"%s","evidence":"cross-built","registered_host":null,"hosted_ci":false,"hosted_receipt":null,"target_spec_sha256":"%s","artifact_sha256":"%s","undefined_imports_sha256":"%s","undefined_imports_count":%s}\n' \
      "$target" "$spec_digest" "$artifact_digest" "$imports_digest" "$imports_count" > "$actual/$target.attestation"
    if test "$mode" = --capture; then
      cp "$actual/$target.attestation" "$crate/attestations/$target.json"
    else
      row="$crate/attestations/$target.json"
      rg -q '"evidence":"cross-built"' "$row"
      rg -q "\"target\":\"$target\"" "$row"
      rg -q "\"target_spec_sha256\":\"$spec_digest\"" "$row"
      rg -q '"hosted_ci":false' "$row"
    fi
done
printf 'iOS compiler targets and static ABI imports verified; Xcode cross-link remains hosted evidence\n'
