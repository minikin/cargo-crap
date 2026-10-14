#!/usr/bin/env bash
# Tests for scripts/action/lib.sh. Run: bash scripts/action/test.sh
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

failures=0
check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: expected '$expected', got '$actual'"
        failures=$((failures + 1))
    fi
}

# The runner's OS and architecture pick the release archive's target.
check "linux x64" "x86_64-unknown-linux-gnu" "$(target_for Linux X64)"
check "linux arm64" "aarch64-unknown-linux-gnu" "$(target_for Linux ARM64)"
check "macos x64" "x86_64-apple-darwin" "$(target_for macOS X64)"
check "macos arm64" "aarch64-apple-darwin" "$(target_for macOS ARM64)"
check "windows x64" "x86_64-pc-windows-msvc" "$(target_for Windows X64)"
check "an unknown runner has no target" "" "$(target_for Windows ARM64 2>/dev/null)"
target_for Windows ARM64 >/dev/null 2>&1
check "an unknown runner fails" "1" "$?"

# The archive each target is released as.
check "a unix archive" "cargo-crap-x86_64-unknown-linux-gnu.tar.gz" \
    "$(archive_for x86_64-unknown-linux-gnu)"
check "a windows archive" "cargo-crap-x86_64-pc-windows-msvc.zip" \
    "$(archive_for x86_64-pc-windows-msvc)"

# Where a version is downloaded from.
check "a pinned version" \
    "https://github.com/minikin/cargo-crap/releases/download/v0.8.0/cargo-crap-aarch64-apple-darwin.tar.gz" \
    "$(download_url 0.8.0 cargo-crap-aarch64-apple-darwin.tar.gz)"
check "the latest version" \
    "https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-apple-darwin.tar.gz" \
    "$(download_url latest cargo-crap-aarch64-apple-darwin.tar.gz)"

# The shields badge's message counts the functions above the threshold.
check "none above" "0" "$(crappy_from_badge '{"message": "passing"}')"
check "some above" "12" "$(crappy_from_badge '{"label": "CRAP > 15", "message": "12 crappy"}')"

if [ "$failures" -gt 0 ]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
