#!/usr/bin/env bash
set -euo pipefail
product="${1:?expected cargo-crap or crapts}"
version="${2:?expected release version}"
case "$product" in cargo-crap|crapts) ;; *) exit 2 ;; esac
status=$(curl -sSL --max-time 30 -o /dev/null -w '%{http_code}' \
    -A 'cargo-crap release (https://github.com/minikin/cargo-crap)' \
    "https://crates.io/api/v1/crates/$product/$version")
case "$status" in
    200) echo "$product $version already published" ;;
    404) cargo publish --locked -p "$product" ;;
    *) echo "$product $version: HTTP $status" >&2; exit 1 ;;
esac
