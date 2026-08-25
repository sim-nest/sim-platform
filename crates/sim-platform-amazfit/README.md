# sim-platform-amazfit

This crate models the bounded state machine intended for a Zepp OS device-app
edge. It does not establish a route between an Amazfit watch and SIM's Android
companion. The delivered manifest registers no app service, the unregistered
side-service source acknowledges and discards frames, and the Android capsule
has no adapter into `AmazfitCapsule`.

Support is model-only. The exact missing edges and the prerequisites for a
physical, network-disabled round trip are recorded in
`attestations/offline-route.json`. Amazfit therefore earns no watch-provider,
continuity, or worn-view role and never gates the Android product.
