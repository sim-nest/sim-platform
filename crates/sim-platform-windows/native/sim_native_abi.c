/* The one thin package shell. It enters the stable ABI and bootstraps rind. */
#include <windows.h>
extern const void *sim_windows_platform_site(void);
extern int sim_bootstrap_rind(const void *site);
__declspec(dllexport) const void *sim_native_abi_v1(void) {
    const void *site = sim_windows_platform_site();
    (void)sim_bootstrap_rind(site);
    return site;
}
