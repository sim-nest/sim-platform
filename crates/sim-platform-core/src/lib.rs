#![forbid(unsafe_code)]
//! Pure, fail-closed manifest contracts for the sole SIM platform owner.

mod validation;
pub use validation::*;

mod records;
pub use records::*;

#[cfg(test)]
mod tests;
