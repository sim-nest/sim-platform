# sim-platform-amazfit

This capsule is the bounded official-wire edge between a Zepp OS device app and
SIM's Android companion. The watch registers lifecycle callbacks and forwards
only typed event and sensor frames; the companion enforces consent, sessions,
deduplication, framing, and queue limits. Host adapters map those frames to the
existing WATCH_8 Device/Stream contracts and map `scene/glance` output back to
bounded display and haptic commands. No SIM evaluator or vendor policy runs on
the watch, and no second phone runtime exists.

Support is `model` / `cross-built`. Physical support requires a registered watch
resource and a device round-trip attestation.
