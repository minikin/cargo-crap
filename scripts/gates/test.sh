#!/usr/bin/env bash
# Tests for the workspace gates.
#
# Each case builds what it needs in a temp dir from a named list of the
# repository's files, never from `git ls-files`: under jj, what git tracks
# depends on the moment, and cargo-mutants' scratch copies have no `.git`.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"

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

scratch="$(mktemp -d)" || exit 1
trap 'rm -rf "$scratch"' EXIT

# The lint case reads clippy's JSON with jq: name it when it is missing
# rather than fail that case for a reason it does not show.
command -v jq > /dev/null || { echo "FAIL jq is not installed"; exit 1; }

# The root manifest, the lock and the core, with stub crate roots for
# cargo-crap: `-p cargo-crap-core` then compiles the core and nothing else.
lint_workspace() {
    local dir="$scratch/lint"
    {
        mkdir -p "$dir/src" "$dir/crates" &&
            cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/clippy.toml" "$repo/rust-toolchain.toml" "$dir/" &&
            cp -R "$repo/crates/cargo-crap-core" "$dir/crates/" &&
            echo 'fn main() {}' > "$dir/src/main.rs" &&
            : > "$dir/src/lib.rs"
    } || { echo "FAIL building the lint workspace" >&2; exit 1; }
    echo "$dir"
}

# Scenario: the core inherits the root's pedantic lints.
dir="$(lint_workspace)" || exit 1
probe="$dir/crates/cargo-crap-core/src/lib.rs"
double_line=$(($(wc -l < "$probe") + 2))
printf '\npub fn double(x: u32) -> u32 {\n    x * 2\n}\n' >> "$probe"
json="$(cd "$dir" && CARGO_TARGET_DIR="$scratch/lint-target" \
    cargo clippy --locked -q --message-format=json -p cargo-crap-core -- -D warnings 2>/dev/null)"
status=$?
# Every error that points at code, as "code file line text". A probe
# rejected for any other reason (a stale lock, a compile error) shows here.
errors="$(jq -r 'select(.reason == "compiler-message") | .message
    | select(.level == "error" and (.spans | length) > 0)
    | (.spans[] | select(.is_primary)) as $span
    | "\(.code.code // "none") \($span.file_name) \($span.line_start) \($span.text[0].text)"' <<< "$json")"
check "the core inherits the root's pedantic lints: clippy fails" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "the core inherits the root's pedantic lints: one error, must_use_candidate on double" \
    "clippy::must_use_candidate crates/cargo-crap-core/src/lib.rs $double_line pub fn double(x: u32) -> u32 {" \
    "$errors"

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
