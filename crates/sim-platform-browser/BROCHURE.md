# sim-platform-browser

In one line: Capability-honest browser Wasm capsule and byte-frame host membrane.

## What it gives you

Run the same capability-honest SIM platform model in windows and workers. The browser capsule offers only detected storage, transport, clipboard, notification, permission, and wake-lock handles while `sim-web` retains every presentation decision. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-platform-browser owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
