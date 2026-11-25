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

# Run a command, a shell function included, with every provider key unset.
# cargo-crap reads a missing key as no key and sends no triage request, and
# project code never sees one. Triage is all or nothing: with a group left
# to ask and no key, the run reports no verdict at all, which none of the
# unpaid runs needs.
keyless() {
    (
        local name
        shopt -s nocasematch
        while IFS= read -r name; do
            case "$name" in
                TYPESAFE_API_KEY | OPENAI_API_KEY) unset "$name" ;;
            esac
        done < <(compgen -e)
        shopt -u nocasematch
        "$@"
    )
}

# Run a cargo-crap report and fail only when it could not run. Exit 1 is a
# gate that tripped after the report was written and flushed, and the
# Action's last step is the only one that gates.
report_run() {
    local status=0
    "$@" || status=$?
    if [ "$status" -ge 2 ]; then
        return "$status"
    fi
}

# What is wrong with comment marker `$1`, or nothing. The marker is matched
# as a comment's exact first line, so it must be one line, and GitHub keeps
# no surrounding blanks on it.
marker_problem() {
    case "$1" in
        "") echo "comment-marker is empty: it would match every comment on the pull request" ;;
        *$'\n'* | *$'\r'*) echo "comment-marker must be one line: it is matched as the comment's first line" ;;
        [[:space:]]* | *[[:space:]]) echo "comment-marker must not start or end with a space" ;;
    esac
}

# `$1` percent-encoded for a URL path: every byte but letters, digits,
# `-._~` and `/`.
url_path_encode() {
    local LC_ALL=C s="$1" c i
    for ((i = 0; i < ${#s}; i++)); do
        c="${s:i:1}"
        case "$c" in
            [A-Za-z0-9._~/-]) printf '%s' "$c" ;;
            *) printf '%%%02X' "'$c" ;;
        esac
    done
}

# The name cargo-crap goes by on a `$1` runner.
binary_name() {
    case "$1" in
        Windows) echo cargo-crap.exe ;;
        *) echo cargo-crap ;;
    esac
}

# Whether config file `$1` sets `top` or `min`, which a binary without
# `--all-rows` cannot clear from a baseline run.
config_limits() {
    python3 -I -c '
import sys, tomllib
with open(sys.argv[1], "rb") as source:
    config = tomllib.load(source)
print("true" if "top" in config or "min" in config else "false")' "$1" || {
        echo "::error::cannot inspect $1 for top or min with Python 3.11+ tomllib: give baseline" >&2
        return 1
    }
}

# The flag a word of `args` sets that the Action sets itself, or nothing.
# Short flags cluster (`-hV`), and `-p` takes the rest of its cluster as
# the package name.
owned_flag() {
    local word="$1" i c
    case "${word%%=*}" in
        --lcov | --format | --output | --baseline | --commit-ref | --all-rows | --print-signal-regressions | --help | --version)
            echo "${word%%=*}"
            return 0
            ;;
        --*) return 0 ;;
        -?*)
            for ((i = 1; i < ${#word}; i++)); do
                c="${word:i:1}"
                case "$c" in
                    p) return 0 ;;
                    h | V)
                        echo "-$c"
                        return 0
                        ;;
                esac
            done
            ;;
    esac
}

# Sort the words of `args` (as arguments) into the runs that get them:
#   common      every run
#   limits      every run but the base run, which records every function
#   report_only the report run: --summary
#   compared_only  the report run when there is a comparison: --show-unchanged
# and set fold_fail_above, fold_fail_regression and has_no_cache. The gate
# flags are the Action's to add once, since cargo-crap refuses a repeated
# switch. A flag the Action owns fails naming it.
# shellcheck disable=SC2034 # the caller reads what this sets
partition_args() {
    common=()
    operands=()
    limits=()
    report_only=()
    compared_only=()
    fold_fail_above=false
    fold_fail_regression=false
    has_no_cache=false
    local word owned
    while [ "$#" -gt 0 ]; do
        word="$1"
        shift
        owned="$(owned_flag "$word")"
        if [ -n "$owned" ]; then
            echo "::error::args: $owned is set by the Action" >&2
            return 1
        fi
        case "$word" in
            --) operands=(-- "$@"); break ;;
            --fail-above) fold_fail_above=true ;;
            --fail-regression) fold_fail_regression=true ;;
            --summary) report_only+=("$word") ;;
            --show-unchanged) compared_only+=("$word") ;;
            --top=* | --min=*) limits+=("$word") ;;
            --top | --min)
                limits+=("$word")
                if [ "$#" -gt 0 ] && [ "$1" != -- ]; then
                    limits+=("$1")
                    shift
                fi
                ;;
            --no-cache)
                has_no_cache=true
                common+=("$word")
                ;;
            *) common+=("$word") ;;
        esac
    done
}

# Whether `$1`, or the value after `=` in it, is an absolute path that lies
# in the checkout at physical path `$2`. A base-commit run scores another
# tree, so such a path would name the pull request's files there.
names_checkout() {
    local value="$1" dir
    case "$value" in
        -*=*) value="${value#*=}" ;;
    esac
    case "$value" in
        /* | [A-Za-z]:[\\/]*) ;;
        *) return 1 ;;
    esac
    [ -e "$value" ] || return 1
    if [ -d "$value" ]; then
        dir="$(cd "$value" && pwd -P)" || return 1
    else
        dir="$(cd "$(dirname "$value")" && pwd -P)" || return 1
    fi
    inside_dir "$dir" "$2"
}

# Whether physical directory `$1` is `$2` or lies below it.
inside_dir() {
    [ "$1" = "$2" ] || [ "${1#"$2"/}" != "$1" ]
}
