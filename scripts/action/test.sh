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

# The checksums file sits beside the archives of the same release.
check "a pinned version's sums" \
    "https://github.com/minikin/cargo-crap/releases/download/v0.8.0/SHA256SUMS" \
    "$(download_url 0.8.0 SHA256SUMS)"

# An archive is installed only when its digest is the one the release lists.
sums_dir="$(mktemp -d)"
printf 'a release archive' > "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz"
printf 'another archive' > "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz"
(cd "$sums_dir" && digest_of cargo-crap-x86_64-unknown-linux-gnu.tar.gz) > "$sums_dir/digest"
check "a digest is sha256" "fd6e6ba7ed5cb5c7f77871e3d91e5ad51d7936c45e9c5a460d8a975601f70ea1" "$(cat "$sums_dir/digest")"
{
    echo "$(cat "$sums_dir/digest")  cargo-crap-x86_64-unknown-linux-gnu.tar.gz"
    echo "0000000000000000000000000000000000000000000000000000000000000000  cargo-crap-aarch64-apple-darwin.tar.gz"
} > "$sums_dir/SHA256SUMS"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/SHA256SUMS"
check "a listed digest passes" "0" "$?"
sed 's/$/\r/' "$sums_dir/SHA256SUMS" > "$sums_dir/crlf"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/crlf"
check "a sums file with CRLF line ends passes" "0" "$?"
{ cat "$sums_dir/SHA256SUMS"; cat "$sums_dir/SHA256SUMS"; } > "$sums_dir/twice"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/twice"
check "a name listed twice passes" "0" "$?"
check "an archive that does not match the release's sums is refused" \
    "::error::cargo-crap-aarch64-apple-darwin.tar.gz does not match the digest in the release's SHA256SUMS" \
    "$(verify_archive "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" 2>&1)"
verify_archive "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" >/dev/null 2>&1
check "a mismatch fails" "1" "$?"
printf 'unlisted' > "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz"
check "an unlisted archive names itself" \
    "::error::the release's SHA256SUMS lists no cargo-crap-x86_64-apple-darwin.tar.gz" \
    "$(verify_archive "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" 2>&1)"
verify_archive "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" >/dev/null 2>&1
check "an unlisted archive fails" "1" "$?"

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
check "a backslash in quotes is kept" "[--exclude] [src\**]" "$(words "--exclude 'src\**'")"

# An input's words, or an error naming the input when they cannot be split.
check "an input's words" "--exclude
a b" "$(words_of args "--exclude 'a b'")"
check "no words of an empty input" "" "$(words_of args "")"
check "a stray quote prints no words" "" "$(words_of args "--threshold 20'" 2>/dev/null)"
check "a stray quote names its input" \
    "::error::args cannot be split into words: check its quotes" \
    "$(words_of args "--threshold 20'" 2>&1 >/dev/null)"
words_of args "--threshold 20'" >/dev/null 2>&1
check "a stray quote fails" "1" "$?"
words_file="$(mktemp)"
words_of args "--exclude ''" > "$words_file"
check "a trailing empty word survives" "2" "$(wc -l < "$words_file" | tr -d ' ')"
check "and it is empty" "" "$(sed -n 2p "$words_file")"

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
