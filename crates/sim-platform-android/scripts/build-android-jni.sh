#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
ndk_root=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
api=${SIM_ANDROID_API:-28}
test -n "$ndk_root"
test -f "$ndk_root/source.properties"
grep -q '^Pkg.Revision = 27\.3\.13750724$' "$ndk_root/source.properties"

toolchain="$ndk_root/toolchains/llvm/prebuilt/linux-x86_64/bin"
test -x "$toolchain/llvm-readobj"
test -x "$toolchain/llvm-nm"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$toolchain/aarch64-linux-android${api}-clang"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$toolchain/armv7a-linux-androideabi${api}-clang"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$toolchain/x86_64-linux-android${api}-clang"
export CARGO_TARGET_I686_LINUX_ANDROID_LINKER="$toolchain/i686-linux-android${api}-clang"
export CARGO_TARGET_DIR="$root/target/android-rust"

output="$root/target/android-jniLibs"
workspace="$root/target/android-jni-workspace"
rm -rf "$output" "$workspace"
mkdir -p "$output" "$workspace"
cp -R "$root/crates/sim-platform-android" "$workspace/android"
cp "$root/crates/sim-platform-android/Cargo.lock" "$workspace/Cargo.lock"
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

for pair in \
    aarch64-linux-android:arm64-v8a \
    armv7-linux-androideabi:armeabi-v7a \
    x86_64-linux-android:x86_64 \
    i686-linux-android:x86
do
    target=${pair%%:*}
    abi=${pair#*:}
    cargo build --manifest-path "$workspace/Cargo.toml" --locked --release \
        -p sim-platform-android --target "$target"
    library="$CARGO_TARGET_DIR/$target/release/libsim_platform_android.so"
    test -s "$library"
    "$toolchain/llvm-nm" --defined-only --dynamic "$library" | rg -q 'sim_native_abi_v1$'
    "$toolchain/llvm-nm" --defined-only --dynamic "$library" | rg -q \
        'Java_org_simnest_shell_SimActivity_nativeCall$'
    "$toolchain/llvm-readobj" --needed-libs "$library" > "$CARGO_TARGET_DIR/$target.needed-libs.txt"
    if rg -i 'libandroid|libnativewindow|libcamera2ndk|libmediandk' \
        "$CARGO_TARGET_DIR/$target.needed-libs.txt"; then
        echo "Android capsule linked framework behavior outside its JNI shell" >&2
        exit 1
    fi
    mkdir -p "$output/$abi"
    cp "$library" "$output/$abi/libsim_platform_android.so"
done

printf 'Android JNI libraries built with NDK r27d/API %s under %s\n' "$api" "$output"
