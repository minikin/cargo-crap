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

# Shell words from an input: quotes honoured, nothing executed.
words() {
    local out=() w
    while IFS= read -r w; do out+=("[$w]"); done < <(split_words "$1")
    echo "${out[*]-}"
}
check "plain words" "[--workspace] [--exclude] [benches/**]" "$(words "--workspace --exclude benches/**")"
check "quoted words" "[--exclude] [a b/**]" "$(words "--exclude 'a b/**'")"
check "double quotes" "[--exclude] [a b]" "$(words '--exclude "a b"')"
check "no words" "" "$(words "")"
check "only spaces" "" "$(words "   ")"
marker="$(mktemp -d)/ran"
# shellcheck disable=SC2016 # the literal `$(` is the point: it must survive unrun
check "nothing is executed" '[$(touch] ['"$marker"')]' "$(words "\$(touch $marker)")"
check "the command did not run" "absent" "$( [ -e "$marker" ] && echo present || echo absent)"

# A true/false input, and nothing else.
check "true" "true" "$(boolean fail-above true)"
check "false" "false" "$(boolean fail-above false)"
check "another spelling is refused" "" "$(boolean fail-above yes 2>/dev/null)"
boolean fail-above yes >/dev/null 2>&1
check "another spelling fails" "1" "$?"

# A count, and nothing else.
is_count 12
check "a count" "0" "$?"
is_count twelve
check "a word is not a count" "1" "$?"
is_count ""
check "nothing is not a count" "1" "$?"

# A path input names an existing file, or nothing.
present="$(mktemp)"
check "an existing file" "" "$(file_input lcov "$present" 2>&1)"
file_input lcov "$present"
check "an existing file passes" "0" "$?"
check "no value passes" "" "$(file_input lcov "" 2>&1)"
check "a missing file names its input" \
    "::error::lcov: no-such.info is not a file (paths are relative to the working directory)" \
    "$(file_input lcov no-such.info 2>&1)"
file_input lcov no-such.info >/dev/null 2>&1
check "a missing file fails" "1" "$?"

if [ "$failures" -gt 0 ]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
