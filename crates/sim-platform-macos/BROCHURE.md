# sim-platform-macos

In one line: macOS platform capsule and deterministic unsigned bundle.

## What it gives you

One explicit macOS provider composes filesystem, process, loading, transport, desktop, permission, audio, MIDI, and compute ports behind SIM's stable native ABI. It produces reproducible unsigned development bundles for Intel and Apple Silicon while keeping every Apple framework detail out of portable crates. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-platform-macos owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
