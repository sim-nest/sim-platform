#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
crate_root="$repo_root/crates/sim-platform-amazfit"
check_root=$(mktemp -d)
trap 'rm -rf "$check_root"' EXIT HUP INT TERM

cp "$crate_root/Cargo.toml" "$check_root/Cargo.toml"
cp -R "$crate_root/src" "$check_root/src"
sed -i \
  -e 's/edition.workspace = true/edition = "2024"/' \
  -e 's/license.workspace = true/license = "MPL-2.0"/' \
  -e '/repository.workspace = true/d' \
  -e '/\[lints\]/,$d' \
  "$check_root/Cargo.toml"

CARGO_NET_OFFLINE=true cargo test --manifest-path "$check_root/Cargo.toml"
CARGO_NET_OFFLINE=true cargo clippy --manifest-path "$check_root/Cargo.toml" --all-targets -- -D warnings

grep -q '"evidence": "cross-built"' "$crate_root/attestations/cross-built.json"
grep -q '"remote_recheck": "terminal-closeout-required"' "$crate_root/contract/zepp-provenance.json"
if grep -REn 'eval[[:space:]]*\(|new[[:space:]]+Function|Function[[:space:]]*\(' "$repo_root/shell/amazfit"; then
  echo "Amazfit shell contains dynamic evaluation" >&2
  exit 1
fi
