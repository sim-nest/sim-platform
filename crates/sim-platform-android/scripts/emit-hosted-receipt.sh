#!/bin/sh
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
results="$root/shell/android/app/build/outputs/androidTest-results"
test -d "$results"
test -n "${GITHUB_SHA:-}"
test -n "${GITHUB_RUN_ID:-}"

results_sha256=$(
    find "$results" -type f -print0 \
        | LC_ALL=C sort -z \
        | xargs -0 sha256sum \
        | sha256sum \
        | cut -d' ' -f1
)
receipt="$root/target/android-hosted-receipt.json"
mkdir -p "$root/target"
printf '%s\n' \
    '{' \
    '  "schema": "sim.platform-hosted-ci-receipt/v1",' \
    '  "level": "hosted-ci",' \
    '  "profile": "android-api-35-x86_64",' \
    '  "checks": ["recreation", "permission-denial", "suspension", "activation", "resource-cleanup"],' \
    "  \"git_commit\": \"$GITHUB_SHA\"," \
    "  \"github_run_id\": \"$GITHUB_RUN_ID\"," \
    "  \"results_sha256\": \"$results_sha256\"" \
    '}' > "$receipt"
printf 'Hosted Android receipt: %s\n' "$receipt"
