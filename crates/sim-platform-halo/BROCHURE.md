# sim-platform-halo

In one line: Official-contract bounded Halo proxy capsule.

## What it gives you

`sim-platform-halo` connects Brilliant Labs Halo through its official host SDK model while keeping behavior in the companion. Android, iOS, and browser hosts share one bounded codec; apps keep using SIM's ordinary glasses Device/Stream profile and Surface protocol. Deterministic model and emulator fixtures exercise the path without pretending that unregistered physical glasses were tested. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-platform-halo owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
