# sim-viture-ffi

In one line: A platform capsule keeps VITURE SDK transport behind a narrow physical device port.

## What it gives you

The crate gives SIM one narrow place to find and open the VITURE glasses SDK at
runtime. It checks explicit local configuration, ordinary dynamic-library lookup,
and Linux USB device hints, then reports a clear unavailable result when no SDK is
present.

It also gives stream providers a safe physical port. Device sessions can ask
for poses and device-control setup without carrying raw handles, linking vendor
objects at build time, or spreading unsafe code through normal provider logic.

## Why you will be glad

You can build and test the stream host on a machine with no glasses attached.
When a VITURE SDK is installed, the risky boundary stays small enough to review,
audit, and replace without disturbing stream routing, consent checks, storage, or
the public provider contract.

## Where it fits

This crate is owned by `sim-platform` and sits below the glasses provider in
`sim-stream-host`. The provider owns profiles and sample expressions; this crate
owns SDK discovery, dynamic loading, native lifecycle, bounded modeled queues,
and the safe wrapper around vendor entry points.
