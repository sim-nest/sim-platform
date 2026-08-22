# sim-platform-android

The Android capsule is AOT Rust behind the unchanged `sim_native_abi_v1` byte-frame table. A thin Kotlin Activity forwards typed lifecycle, activation, permission, content-URI, notification, audio-device, and background-execution inputs. It never passes ambient filesystem paths and contains no parallel runtime.

The declared Android ABIs are `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`, and `i686-linux-android`. See `contract/android-build-provenance.json` for the pinned toolchain policy.

