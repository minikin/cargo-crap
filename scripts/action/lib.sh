#!/usr/bin/env bash
# Pure helpers the Action's steps share. Sourced, never run.

# The release target for a runner, from RUNNER_OS and RUNNER_ARCH. Fails for
# a runner no release is built for.
target_for() {
    case "$1/$2" in
        Linux/X64) echo x86_64-unknown-linux-gnu ;;
        Linux/ARM64) echo aarch64-unknown-linux-gnu ;;
        macOS/X64) echo x86_64-apple-darwin ;;
        macOS/ARM64) echo aarch64-apple-darwin ;;
        Windows/X64) echo x86_64-pc-windows-msvc ;;
        *)
            echo "cargo-crap has no release for a $1 $2 runner" >&2
            return 1
            ;;
    esac
}

# The archive a target is released as: a zip on Windows, a tarball elsewhere.
archive_for() {
    case "$1" in
        *-windows-*) echo "cargo-crap-$1.zip" ;;
        *) echo "cargo-crap-$1.tar.gz" ;;
    esac
}

# Where `archive` of `version` (or `latest`) is downloaded from.
download_url() {
    local base="https://github.com/minikin/cargo-crap/releases"
    if [ "$1" = latest ]; then
        echo "$base/latest/download/$2"
    else
        echo "$base/download/v$1/$2"
    fi
}

# The SHA-256 digest of file `$1`, in lowercase hex: `sha256sum` on Linux and
# Git Bash, `shasum` on macOS.
digest_of() {
    if command -v sha256sum > /dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# Whether archive `$1` has the digest the release's sums file `$2` lists for
# its name. Either failure names the archive.
verify_archive() {
    local name expected
    name="$(basename "$1")"
    expected="$(awk -v name="$name" '{ sub(/\r$/, "") } $2 == name || $2 == "*" name { print $1; exit }' "$2")"
    if [ -z "$expected" ]; then
        echo "::error::the release's SHA256SUMS lists no $name" >&2
        return 1
    fi
    if [ "$(digest_of "$1")" != "$expected" ]; then
        echo "::error::$name does not match the digest in the release's SHA256SUMS" >&2
        return 1
    fi
}

# How many functions a shields badge counts above the threshold: its message
# is "passing" for none and "N crappy" otherwise.
crappy_from_badge() {
    local message
    message="$(printf '%s' "$1" | sed -n 's/.*"message": *"\([^"]*\)".*/\1/p')"
    case "$message" in
        passing) echo 0 ;;
        *) echo "${message%% *}" ;;
    esac
}

# The shell words in `$1`, one per line: quotes and backslashes are honoured
# as the shell would, and nothing in them is executed or expanded.
split_words() {
    if [ -z "${1//[[:space:]]/}" ]; then
        return 0
    fi
    # A last word `.` is split with the input and dropped again: GNU xargs
    # drops an empty word ('') at the very end of its input, BSD xargs keeps
    # it. The pipeline's status is xargs', so a stray quote still fails.
    local words
    words="$(printf '%s .' "$1" | xargs printf '%s\n')" || return 1
    printf '%s' "${words%.}"
}

# The words of input `$1` (value `$2`), one per line. Input that cannot be
# split, such as a stray quote, prints none and fails naming the input: xargs
# may have printed the words before the quote, and a run without the rest
# would pass under the wrong settings.
words_of() {
    local words
    # The `.` keeps the trailing newlines `$( )` would strip, and with them a
    # trailing empty word.
    if ! words="$(split_words "$2" 2> /dev/null || exit 1; echo .)"; then
        echo "::error::$1 cannot be split into words: check its quotes" >&2
        return 1
    fi
    printf '%s' "${words%.}"
}

# `value` when it is `true` or `false`. Anything else is an error naming the
# input, so a misspelt switch never turns a gate off unnoticed.
boolean() {
    case "$2" in
        true | false) echo "$2" ;;
        *)
            echo "::error::$1 must be true or false, not '$2'" >&2
            return 1
            ;;
    esac
}

# Whether `$1` is a count: one or more digits.
is_count() {
    [[ "$1" =~ ^[0-9]+$ ]]
}

# Whether the path input `$1`, set to `$2`, is empty or names an existing
# file. A missing file is an error naming the input.
file_input() {
    if [ -n "$2" ] && [ ! -f "$2" ]; then
        echo "::error::$1: $2 is not a file (paths are relative to the working directory)" >&2
        return 1
    fi
}
