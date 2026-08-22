/* Private Darwin glue. The exported name is the unchanged SIM native ABI. */
#include <stddef.h>
extern const void *sim_macos_platform_site(void);
const void *sim_native_abi_v1(void) { return sim_macos_platform_site(); }
