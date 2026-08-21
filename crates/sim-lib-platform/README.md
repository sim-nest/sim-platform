# sim-lib-platform

The loadable, provider-neutral Small SIM API. It exports exactly thirteen
functions, from `platform/card` through `platform/activations`; the function
surface is closed while service identities remain open.

Calls resolve `platform/active-site` from the current lexical realization
environment and dispatch through its `EvalFabric`. Privileged calls check their
SIM capability before the provider can inspect or request an OS permission.
Every request is local-only, single-answer, deadline-bounded, and traced; effect
receipts and provider evidence remain data returned by the site.

Use `sim-platform-model` for deterministic host-free tests. The modeled and
Ubuntu recipes carry the same `platform/require` expression, including the
optional GPIO substitution.
