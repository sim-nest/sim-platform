#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)
crate_root="$repo_root/crates/sim-platform-halo"
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
test "$(tr -d '\n' < "$crate_root/fixtures/button-select.hex")" = "5348010110203040000000010007000100070173656c656374"
grep -q '"official_emulator": true' "$crate_root/emulator/profile.json"
grep -q '"physical": false' "$crate_root/attestations/cross-built.json"
grep -q 'GLASSES_8' "$crate_root/README.md"
grep -q 'Surface' "$crate_root/README.md"
if grep -REn 'loadstring|dofile|require[[:space:]]*\(|eval[[:space:]]*\(' "$repo_root/shell/halo"; then
  echo "Halo shell contains dynamic behavior loading" >&2
  exit 1
fi
