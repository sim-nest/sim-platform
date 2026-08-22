#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM

mkdir -p "$scratch/crates"
cp -R "$repo_root/crates/sim-platform-core" "$scratch/crates/"
cp -R "$repo_root/crates/sim-platform-wasi" "$scratch/crates/"
rm -rf "$scratch/crates/sim-platform-wasi/target"
cp "$repo_root/Cargo.lock" "$scratch/Cargo.lock"
sed -n '/^\[workspace.package\]/,$p' "$repo_root/Cargo.toml" > "$scratch/workspace-tail.toml"
{
    echo '[workspace]'
    echo 'resolver = "3"'
    echo 'members = ["crates/sim-platform-core", "crates/sim-platform-wasi"]'
    cat "$scratch/workspace-tail.toml"
} > "$scratch/Cargo.toml"

cargo generate-lockfile --manifest-path "$scratch/Cargo.toml" --offline
cargo test --manifest-path "$scratch/Cargo.toml" -p sim-platform-wasi --locked
cargo clippy --manifest-path "$scratch/Cargo.toml" -p sim-platform-wasi --all-targets --locked -- -D warnings
