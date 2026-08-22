# WASI, without ambient authority

Run portable components with an import list that says exactly which existing
SIM services they may reach. Named Table/Dir preopens and explicit service
bindings make the component reproducible, inspectable, and fail-closed.
