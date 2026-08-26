# sim-platform-sqlite

In one line: Attesting SQLite relation capsule for SIM.

## What it gives you

Run the same checked relational plans against memory or durable preopened SQLite files without making SQL or native paths part of the application. Schema state is re-introspected and content-attested, schema evolution steps are atomic, attached sources are named and bounded, and provider failures become stable relation errors. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-platform-sqlite owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
