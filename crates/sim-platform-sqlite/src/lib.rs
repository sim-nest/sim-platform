// conformance: SQLite realization remains bounded, attested, and locator-safe.

//! Attesting `SQLite` realization behind the provider-neutral relation site.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod driver;

pub use driver::*;
