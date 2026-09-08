# sim-platform-ubuntu-pc

In one line: Ubuntu PC reference platform capsule.

## What it gives you

One bounded, physically evidenced membrane for Ubuntu desktop and headless PCs. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

The capsule's `LocalCheckAdapter` connects exact runtime `CommandSpec` entries to the durable M5 operation lifecycle. It supports trusted `ProcessPort` mechanics and fully confined `BwrapLauncher` execution, while the released checker-facing port exposes no native paths or command construction. Each run binds an owned checkout, explicit inputs, outputs and scratch roots, a sealed environment, time and output limits, descendant cleanup, and an independent postcondition observation.

`BwrapLauncher::confinement_status` separately reports whether the exact
boot-resolved bubblewrap and limit mechanics are live. That record deliberately
sets `purity_qualified` to false. Anonymous roots, namespace isolation, and
bounded resources constrain effects; mounted inputs, `/proc`, `/dev`, and host
process observations mean they do not prove a native function depends only on
declared semantic inputs.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.
- Real bubblewrap conformance proves an anonymous root, absent networking, literal command bytes, output observation, and zero retained scratch entries.
- Timeout and cancellation kill the owned process group, escalate from `TERM` to `KILL` when required, and refuse completion if descendants remain.
- Provider admission can require the typed live membrane record without
  confusing confinement with source-level purity qualification.

## Where it fits

Within SIM, sim-platform-ubuntu-pc owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
