#![forbid(unsafe_code)]
//! Repository-local structural gates.

use std::path::Path;
use std::sync::Arc;

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
        Some("check" | "simdoc" | "crate-catalog" | "compute-acceptance")
    ) {
        eprintln!(
            "usage: cargo run -p xtask -- <check|simdoc|crate-catalog> [--check] | compute-acceptance <capture|verify|import> ..."
        );
        std::process::exit(2);
    }
    if command.as_deref() == Some("compute-acceptance") {
        if let Err(error) = compute_acceptance(std::env::args().collect()) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
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
        "sim-platform-construction",
        "sim-platform-linux",
        "sim-platform-ubuntu-pc",
        "sim-platform-ubuntu-rpi",
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

fn compute_acceptance(args: Vec<String>) -> Result<(), String> {
    let command_args = std::iter::once("compute".to_owned())
        .chain(std::iter::once("acceptance".to_owned()))
        .chain(args.into_iter().skip(2))
        .collect::<Vec<_>>();
    let command = sim_lib_compute_cli::parse_compute_args(&command_args)
        .map_err(|error| error.to_string())?;
    let (mut cx, seat) = sim_kernel::Cx::new_seated(
        Arc::new(sim_kernel::EagerPolicy),
        Arc::new(sim_kernel::DefaultFactory),
    );
    seat.grant(
        &mut cx,
        sim_lib_compute_cli::compute_acceptance_capability(),
    )
    .map_err(|error| error.to_string())?;
    let output = sim_lib_compute_cli::run_command(&mut cx, None, &command)
        .map_err(|error| error.to_string())?;
    print!("{output}");
    Ok(())
}
