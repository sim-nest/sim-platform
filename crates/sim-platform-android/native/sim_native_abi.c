/* Android NDK membrane. AOT changes symbol resolution, not the ABI table. */
struct NativeLibAbiV1;
extern const struct NativeLibAbiV1 sim_android_platform_abi_v1;
const struct NativeLibAbiV1 *sim_native_abi_v1(void) {
    return &sim_android_platform_abi_v1;
}

/* The JNI nativeCall pump resolves this table and invokes its named call
 * member. Android objects never cross the byte-frame boundary. */
