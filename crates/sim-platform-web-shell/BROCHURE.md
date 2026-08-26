# sim-platform-web-shell

In one line: Native Linux process capsule for the host-neutral SIM web shell.

## What it gives you

This capsule is the Linux process rind for the host-neutral `sim-web-shell` library. It realizes socket and DNS transport, explicit mounted-file reads, monotonic time, entropy, and external URL opening. Reversible Surface, Scene, Intent, session, and layout behavior remains replaceable in `sim-web`. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-platform-web-shell owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
