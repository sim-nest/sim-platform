#!/bin/sh
set -eu

mode=${1:---check}
case "$mode" in
    --capture|--check) ;;
    *) echo "usage: $0 [--capture|--check]" >&2; exit 64 ;;
esac

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
crate="$root/crates/sim-platform-android"
state_root=${SIM_ANDROID_CROSS_ROOT:-${XDG_STATE_HOME:-$HOME/.local/state}/sim/platform-android-cross}
workspace="$state_root/workspace"
target_dir="$state_root/target"
actual_dir="$state_root/actual-attestations"
rustc=${RUSTC:-rustc}
cargo=${CARGO:-cargo}
llvm_nm=${LLVM_NM:-/usr/lib/llvm-20/bin/llvm-nm}
targets=${SIM_ANDROID_TARGETS:-"aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android"}
rust_library="$($rustc --print sysroot)/lib/rustlib/src/rust/library"

case "$state_root" in
    ""|/|"$HOME") echo "refusing unsafe Android cross-build state root: $state_root" >&2; exit 64 ;;
esac
test -x "$llvm_nm"
test -n "${CARGO_HOME:-}"
test -d "$rust_library"

rm -rf "$workspace" "$actual_dir"
mkdir -p "$workspace" "$actual_dir" "$target_dir"
cp -R "$crate" "$workspace/android"
cp "$crate/Cargo.lock" "$workspace/Cargo.lock"
sed -i 's/crate-type = \["rlib", "staticlib", "cdylib"\]/crate-type = ["staticlib"]/' "$workspace/android/Cargo.toml"
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
    'pedantic = "warn"' > "$workspace/Cargo.toml"

rust_release=$($rustc --version --verbose | sed -n 's/^release: //p')
rust_commit=$($rustc --version --verbose | sed -n 's/^commit-hash: //p')
test "$rust_release" = "1.97.1"
test "$rust_commit" = "8bab26f4f68e0e26f0bb7960be334d5b520ea452"
rust_pin=$(printf '%s' \
    'https://doc.rust-lang.org/rustc/platform-support/android.html|1.97.1|8bab26f4f68e0e26f0bb7960be334d5b520ea452|aarch64-linux-android,armv7-linux-androideabi,x86_64-linux-android,i686-linux-android' \
    | sha256sum | cut -d' ' -f1)
ndk_pin=$(printf '%s' \
    'https://developer.android.com/ndk/downloads/|r27d|27.3.13750724|28' \
    | sha256sum | cut -d' ' -f1)
rg -q "\"pin_sha256\": \"$rust_pin\"" "$crate/contract/android-build-provenance.json"
rg -q "\"pin_sha256\": \"$ndk_pin\"" "$crate/contract/android-build-provenance.json"

for target in $targets; do
    target_spec="$actual_dir/$target.target-spec.json"
    RUSTC_BOOTSTRAP=1 "$rustc" -Z unstable-options --print target-spec-json \
        --target "$target" > "$target_spec"
    rg -q '"tier": 2' "$target_spec"
    rg -q '"std": true' "$target_spec"
    rg -q '"host_tools": false' "$target_spec"
    target_spec_sha256=$(sha256sum "$target_spec" | cut -d' ' -f1)
    expected_target_spec_sha256=$(sed -n \
        "s/.*\"$target\": \"\([0-9a-f][0-9a-f]*\)\".*/\1/p" \
        "$crate/contract/android-build-provenance.json")
    test "$target_spec_sha256" = "$expected_target_spec_sha256"

    RUSTC_BOOTSTRAP=1 RUSTFLAGS="--remap-path-prefix=$workspace=." \
        CARGO_TARGET_DIR="$target_dir" "$cargo" rustc \
        --manifest-path "$workspace/Cargo.toml" \
        -Z build-std=std,panic_abort \
        -p sim-platform-android --locked --release --target "$target" --offline \
        -- --crate-type staticlib

    artifact="$target_dir/$target/release/libsim_platform_android.a"
    test -s "$artifact"
    "$llvm_nm" --defined-only --extern-only "$artifact" | rg -q 'sim_native_abi_v1$'
    "$llvm_nm" --defined-only --extern-only "$artifact" | rg -q \
        'Java_org_simnest_shell_SimActivity_nativeCall$'
    "$llvm_nm" --undefined-only --extern-only --format=posix "$artifact" \
        | LC_ALL=C sort -u > "$actual_dir/$target.undefined-imports.txt"
    if rg -i 'ANativeActivity|AAssetManager|AInputQueue|ALooper|android_app|eglSwapBuffers' \
        "$actual_dir/$target.undefined-imports.txt"; then
        echo "Android capsule imports framework behavior outside the Kotlin/JNI shell" >&2
        exit 1
    fi

    artifact_sha256=$(sha256sum "$artifact" | cut -d' ' -f1)
    imports_sha256=$(sha256sum "$actual_dir/$target.undefined-imports.txt" | cut -d' ' -f1)
    imports_count=$(wc -l < "$actual_dir/$target.undefined-imports.txt" | tr -d ' ')
    printf '%s\n' \
        '{' \
        '  "schema": "sim.platform-cross-build-attestation/v1",' \
        "  \"target\": \"$target\"," \
        '  "evidence": "cross-built",' \
        '  "registered_host": null,' \
        '  "hosted_ci": false,' \
        '  "hosted_receipt": null,' \
        "  \"rust_release\": \"$rust_release\"," \
        "  \"rust_commit\": \"$rust_commit\"," \
        "  \"target_spec_sha256\": \"$target_spec_sha256\"," \
        "  \"artifact_sha256\": \"$artifact_sha256\"," \
        "  \"undefined_imports_sha256\": \"$imports_sha256\"," \
        "  \"undefined_imports_count\": $imports_count" \
        '}' > "$actual_dir/$target.json"

    if test "$mode" = --check; then
        cmp "$actual_dir/$target.json" "$crate/attestations/$target.json"
    fi
done

printf 'Android static AOT cross-build proof passed for: %s\n' "$targets"
printf 'Captured proof: %s\n' "$actual_dir"
