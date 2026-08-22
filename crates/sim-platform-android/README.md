# sim-platform-android

The Android capsule is AOT Rust behind the unchanged `sim_native_abi_v1` byte-frame table. A thin Kotlin Activity forwards typed lifecycle, activation, permission, content-URI, notification, audio-device, and background-execution inputs. It never passes ambient filesystem paths and contains no parallel runtime.

The declared Android ABIs are `aarch64-linux-android`,
`armv7-linux-androideabi`, `x86_64-linux-android`, and
`i686-linux-android`. `scripts/cross-build.sh --check` rebuilds their static
archives from the installed matching `rust-src`, checks the exported ABI/JNI
symbols and native-import policy, and verifies the committed artifact digests.
The capsule-local `Cargo.lock` pins that isolated build closure. See
`contract/android-build-provenance.json` for the pinned toolchain policy.

The repository's Android emulator workflow builds NDK r27d shared libraries and
exercises lifecycle recreation, denial, suspension, activation, and cleanup.
Its `hosted-ci` receipt is generated only by that workflow and remains pending
until SUP20 terminal closeout is authorized to retain the exact remote result.
