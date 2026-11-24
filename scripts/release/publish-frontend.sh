#!/usr/bin/env bash
set -euo pipefail
product="${1:?expected cargo-crap or crapts}"
version="${2:?expected release version}"
case "$product" in cargo-crap|crapts) ;; *) exit 2 ;; esac
for attempt in 1 2 3; do
    status=$(curl -sSL --max-time 30 -o /dev/null -w '%{http_code}' \
        -A 'cargo-crap release (https://github.com/minikin/cargo-crap)' \
        "https://crates.io/api/v1/crates/$product/$version")
    case "$status" in
        200) echo "$product $version already published"; exit 0 ;;
        404) ;;
        *) echo "$product $version: HTTP $status" >&2; exit 1 ;;
    esac
    if output=$(CARGO_TERM_COLOR=never cargo publish --locked -p "$product" 2>&1); then
        printf '%s\n' "$output"
        exit 0
    else
        code=$?
    fi
    printf '%s\n' "$output" >&2
    if ((attempt == 3)); then exit "$code"; fi
    case "$output" in
        *"no matching package named \`cargo-crap-core\` found"*|\
        *"searched package name: \`cargo-crap-core\`"*|\
        *"failed to select a version for the requirement \`cargo-crap-core = "*) ;;
        *) exit "$code" ;;
    esac
    sleep "$((5 * attempt))"
done
