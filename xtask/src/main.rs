#![forbid(unsafe_code)]
//! Repository-local structural gates.

use std::path::Path;

const REQUIRED: &[&str] = &[
    "README.md",
    "BROCHURE.md",
    "LICENSE",
    "features.toml",
    "docs/humans/README.md",
    "docs/agents/README.md",
    "docs/generated/README.md",
    "docs/rustdoc/README.md",
    ".github/workflows/ci.yml",
];

fn main() {
    let command = std::env::args().nth(1);
    if !matches!(
        command.as_deref(),
        Some("check" | "simdoc" | "crate-catalog")
    ) {
        eprintln!("usage: cargo run -p xtask -- <check|simdoc|crate-catalog> [--check]");
        std::process::exit(2);
    }
    let missing = REQUIRED
        .iter()
        .filter(|path| !Path::new(path).is_file())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        eprintln!("missing repository contract files: {missing:?}");
        std::process::exit(1);
    }
    for name in [
        "sim-platform-core",
        "sim-platform-model",
        "sim-lib-platform",
        "sim-platform-bootstrap",
    ] {
        for relative in [
            "Cargo.toml",
            "README.md",
            "BROCHURE.md",
            "recipes/book.toml",
        ] {
            let path = format!("crates/{name}/{relative}");
            if !Path::new(&path).is_file() {
                eprintln!("missing crate contract file: {path}");
                std::process::exit(1);
            }
        }
    }
}
