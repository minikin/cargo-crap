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

# The mutants cases need both tools on PATH. A missing one fails the
# suite: a case that skipped would report a gate it never ran.
for tool in just cargo-mutants; do
    command -v "$tool" > /dev/null || { echo "FAIL $tool is not installed"; exit 1; }
done
real_cargo="$(command -v cargo)"
# CI sets CARGO_TERM_COLOR=always, which wraps cargo-mutants' listing in
# colour codes that no check here matches.
export CARGO_TERM_COLOR=never

# git in the fixtures reads no user or system config, so a signing key or a
# diff.renames setting on the machine cannot change what a case sees.
fixture_git() {
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
        GIT_AUTHOR_NAME=fixture GIT_AUTHOR_EMAIL=fixture@example.invalid \
        GIT_COMMITTER_NAME=fixture GIT_COMMITTER_EMAIL=fixture@example.invalid \
        git "$@"
}

# A workspace of a root package and a cargo-crap-core member, committed on
# main, with this repository's gate scripts and mutants config, so a config
# change that stops cargo-mutants reaching `crates/` fails here.
mutants_repo() {
    local dir
    dir="$(mktemp -d "$scratch/repo.XXXXXX")" || exit 1
    {
        mkdir -p "$dir/src" "$dir/crates/cargo-crap-core/src" "$dir/scripts" &&
            cp -R "$repo/scripts/gates" "$dir/scripts/" &&
            mkdir -p "$dir/.cargo" && cp "$repo/.cargo/mutants.toml" "$dir/.cargo/" &&
            printf '[package]\nname = "fixture-root"\nversion = "0.1.0"\nedition = "2024"\n\n[workspace]\nmembers = ["crates/cargo-crap-core"]\n' > "$dir/Cargo.toml" &&
            printf '[package]\nname = "cargo-crap-core"\nversion = "0.1.0"\nedition = "2024"\n' > "$dir/crates/cargo-crap-core/Cargo.toml" &&
            printf 'pub fn one() -> u32 {\n    1\n}\n' > "$dir/src/lib.rs" &&
            printf '//! The fixture core.\n' > "$dir/crates/cargo-crap-core/src/lib.rs" &&
            printf 'target/\n' > "$dir/.gitignore" &&
            (cd "$dir" && fixture_git init -q -b main && fixture_git add -A && fixture_git commit -q -m init)
    } > /dev/null 2>&1 || { echo "FAIL building the mutants fixture" >&2; exit 1; }
    echo "$dir"
}

# A `cargo` first on PATH that records its arguments, one call per line and
# each argument in brackets so their boundaries show, keeps a copy of the
# file given to --in-diff, then exits 0 without running anything.
shim="$scratch/shim"
mkdir -p "$shim"
cat > "$shim/cargo" <<'SHIM'
#!/usr/bin/env bash
printf '[%s]' "$@" >> "$SHIM_LOG"
echo >> "$SHIM_LOG"
prev=""
for arg in "$@"; do
    if [ "$prev" = "--in-diff" ]; then cp "$arg" "$SHIM_DIFF"; fi
    prev="$arg"
done
SHIM
chmod +x "$shim/cargo"

# Run "$@" in fixture $1 with the shim first on PATH. Afterwards $calls
# holds every cargo call the shim saw and $copied the diff it kept.
with_shim() {
    local dir="$1"
    shift
    export SHIM_LOG="$dir.shim.log" SHIM_DIFF="$dir.shim.diff"
    : > "$SHIM_LOG"
    rm -f "$SHIM_DIFF"
    out="$(cd "$dir" && PATH="$shim:$PATH" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$@" 2>&1)"
    status=$?
    calls="$(cat "$SHIM_LOG")"
    copied="$SHIM_DIFF"
}

# The mutants cargo-mutants lists in fixture $1, one per line: all of them,
# or with a second argument only those in that diff. A failed listing
# prints a line no mutant check matches, so it cannot read as "no mutants".
listed() {
    local list args=(mutants --list --workspace)
    if [ -n "${2:-}" ]; then args+=(--in-diff "$2"); fi
    if list="$(cd "$1" && "$real_cargo" "${args[@]}" 2>&1)"; then
        printf '%s\n' "$list"
    else
        echo "LISTING FAILED: $list"
    fi
}

# How many of fixture $1's listed mutants (in diff $3, if given) are in
# file $2, or "listing failed".
mutants_in() {
    local list
    list="$(listed "$1" "${3:-}")"
    if grep -q '^LISTING FAILED' <<< "$list"; then
        echo "listing failed"
    else
        grep -c "^$2:" <<< "$list" || true
    fi
}

shim_ran_mutants() {
    grep -q '^\[mutants\]\[--workspace\]\[--in-diff\]\[[^]]*\]$' <<< "$calls" && echo 1 || echo 0
}

double='pub fn double(x: u32) -> u32 { x * 2 }'

# Scenario: just mutants-diff mutates a changed line in the core.
dir="$(mutants_repo)" || exit 1
echo "$double" >> "$dir/crates/cargo-crap-core/src/lib.rs"
with_shim "$dir" just --justfile "$repo/Justfile" --working-directory "$dir" mutants-diff
check "just mutants-diff mutates a changed line in the core: cargo mutants --workspace --in-diff" \
    "1" "$(shim_ran_mutants)"
check "just mutants-diff mutates a changed line in the core: exits 0" "0" "$status"
check "just mutants-diff mutates a changed line in the core: the diff holds the line" \
    "1" "$(grep -qxF "+$double" "$copied" 2> /dev/null && echo 1 || echo 0)"
check "just mutants-diff mutates a changed line in the core: replace * with + in double" \
    "1" "$(listed "$dir" "$copied" | grep -q 'crates/cargo-crap-core/src/lib.rs:.*replace \* with + in double' && echo 1 || echo 0)"

# Scenario: CI's base-ref diff mutates a committed change in the core.
dir="$(mutants_repo)" || exit 1
(cd "$dir" && fixture_git checkout -q -b feature && echo "$double" >> crates/cargo-crap-core/src/lib.rs &&
    fixture_git commit -q -am double && echo docs > README.md && fixture_git add README.md &&
    fixture_git commit -q -m readme) > /dev/null 2>&1 || { echo "FAIL setting up the base-ref case"; exit 1; }
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base main
check "CI's base-ref diff mutates a committed change in the core: cargo mutants --workspace --in-diff" \
    "1" "$(shim_ran_mutants)"
check "CI's base-ref diff mutates a committed change in the core: exits 0" "0" "$status"
check "CI's base-ref diff mutates a committed change in the core: the diff holds the line" \
    "1" "$(grep -qxF "+$double" "$copied" 2> /dev/null && echo 1 || echo 0)"
check "CI's base-ref diff mutates a committed change in the core: replace * with + in double" \
    "1" "$(listed "$dir" "$copied" | grep -q 'crates/cargo-crap-core/src/lib.rs:.*replace \* with + in double' && echo 1 || echo 0)"

# Scenario: CI's base-ref diff with no source change measures nothing. The
# commit below the branch changes src/, so a fallback to the last commits
# would find something to mutate.
dir="$(mutants_repo)" || exit 1
(cd "$dir" && printf 'pub fn two() -> u32 {\n    2\n}\n' >> src/lib.rs && fixture_git commit -q -am two &&
    fixture_git checkout -q -b feature && echo docs > README.md && fixture_git add README.md &&
    fixture_git commit -q -m readme) > /dev/null 2>&1 || { echo "FAIL setting up the no-change case"; exit 1; }
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base main
check "CI's base-ref diff with no source change measures nothing: no cargo call" "" "$calls"
check "CI's base-ref diff with no source change measures nothing: says so" \
    "1" "$(grep -q 'nothing was measured' <<< "$out" && echo 1 || echo 0)"
check "CI's base-ref diff with no source change measures nothing: exits 0" "0" "$status"

# The six-function module the move cases move from src/ into the core.
x_module='pub fn a(x: u32) -> u32 { x + 1 }
pub fn c(x: u32) -> bool { x > 3 }
pub fn d(x: u32) -> u32 { x * 3 }
pub fn e(x: u32) -> bool { x == 7 }
pub fn f(x: u32) -> u32 { x - 1 }
pub fn g(x: u32) -> bool { x < 9 }'

# A fixture whose root package declares src/x.rs, committed, with
# diff.renames = false in its own config.
move_repo() {
    local dir
    dir="$(mutants_repo)" || exit 1
    (cd "$dir" && fixture_git config diff.renames false && echo "$x_module" > src/x.rs &&
        echo 'pub mod x;' >> src/lib.rs && fixture_git add -A && fixture_git commit -q -m x) > /dev/null 2>&1 ||
        { echo "FAIL building the move fixture" >&2; exit 1; }
    echo "$dir"
}

# Moves src/x.rs into the core of fixture $1 with `mv` (git learns of the new
# path only if a case adds it) and moves its `pub mod x;` along.
move_x() {
    mkdir -p "$1/crates/cargo-crap-core/src" &&
        mv "$1/src/x.rs" "$1/crates/cargo-crap-core/src/x.rs" &&
        printf 'pub fn one() -> u32 {\n    1\n}\n' > "$1/src/lib.rs" &&
        echo 'pub mod x;' >> "$1/crates/cargo-crap-core/src/lib.rs"
}

# The part of diff $1 about the file that ends up at $2.
section() {
    awk -v path="$2" '/^diff --git / { inside = ($0 ~ (" b/" path "$")) } inside' "$1" 2> /dev/null
}

core_x="crates/cargo-crap-core/src/x.rs"

# Scenario: a pure move costs no mutants.
dir="$(move_repo)" || exit 1
(move_x "$dir" && cd "$dir" && fixture_git add -A && fixture_git commit -q -m move) > /dev/null 2>&1 ||
    { echo "FAIL setting up the pure move"; exit 1; }
check "a pure move costs no mutants: the moved file has mutants unfiltered" \
    "1" "$(n="$(mutants_in "$dir" "$core_x")"; [ "$n" != "listing failed" ] && [ "$n" -gt 0 ] && echo 1 || echo 0)"
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base HEAD~1
check "a pure move costs no mutants: rename from src/x.rs" \
    "1" "$(grep -qx 'rename from src/x.rs' "$copied" 2> /dev/null && echo 1 || echo 0)"
check "a pure move costs no mutants: rename to the core" \
    "1" "$(grep -qx "rename to $core_x" "$copied" 2> /dev/null && echo 1 || echo 0)"
check "a pure move costs no mutants: no hunk for x.rs" "0" "$(section "$copied" "$core_x" | grep -c '^@@')"
check "a pure move costs no mutants: none listed in x.rs" "0" "$(mutants_in "$dir" "$core_x" "$copied")"

# Scenario: a move with one edited line mutates that line only.
dir="$(move_repo)" || exit 1
(move_x "$dir" && sed -i.bak '1s/x + 1/x + 2/' "$dir/$core_x" && rm "$dir/$core_x.bak" &&
    cd "$dir" && fixture_git add -A && fixture_git commit -q -m move) > /dev/null 2>&1 ||
    { echo "FAIL setting up the edited move"; exit 1; }
unfiltered="$(listed "$dir")"
check "a move with one edited line mutates that line only: a and c have mutants unfiltered" "1" \
    "$(grep -q "^$core_x:.* in a$" <<< "$unfiltered" && grep -q "^$core_x:.* in c$" <<< "$unfiltered" && echo 1 || echo 0)"
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base HEAD~1
filtered="$(listed "$dir" "$copied")"
check "a move with one edited line mutates that line only: a rename" \
    "1" "$(grep -qx "rename to $core_x" "$copied" 2> /dev/null && echo 1 || echo 0)"
check "a move with one edited line mutates that line only: replace + with - in a" \
    "1" "$(grep -q "^$core_x:.*replace + with - in a$" <<< "$filtered" && echo 1 || echo 0)"
check "a move with one edited line mutates that line only: every listed mutant in x.rs is on line 1" \
    "0" "$(if grep -q '^LISTING FAILED' <<< "$filtered"; then echo "listing failed"; else grep "^$core_x:" <<< "$filtered" | grep -vc "^$core_x:1:" || true; fi)"
check "a move with one edited line mutates that line only: none in c" \
    "0" "$(if grep -q '^LISTING FAILED' <<< "$filtered"; then echo "listing failed"; else grep -c "^$core_x:.* in c$" <<< "$filtered" || true; fi)"

# Scenario: an untracked moved file still pairs as a rename.
dir="$(move_repo)" || exit 1
move_x "$dir" || { echo "FAIL setting up the untracked move"; exit 1; }
check "an untracked moved file still pairs as a rename: the moved file has mutants unfiltered" \
    "1" "$(n="$(mutants_in "$dir" "$core_x")"; [ "$n" != "listing failed" ] && [ "$n" -gt 0 ] && echo 1 || echo 0)"
before="$(cd "$dir" && fixture_git status --porcelain)"
with_shim "$dir" bash scripts/gates/mutants-diff.sh
check "an untracked moved file still pairs as a rename: rename to the core" \
    "1" "$(grep -qx "rename to $core_x" "$copied" 2> /dev/null && echo 1 || echo 0)"
check "an untracked moved file still pairs as a rename: no hunk for x.rs" "0" "$(section "$copied" "$core_x" | grep -c '^@@')"
check "an untracked moved file still pairs as a rename: none listed in x.rs" "0" "$(mutants_in "$dir" "$core_x" "$copied")"
check "an untracked moved file still pairs as a rename: git status unchanged" \
    "$before" "$(cd "$dir" && fixture_git status --porcelain)"

# Scenario: a source file under a member's tests directory is not in the diff.
dir="$(mutants_repo)" || exit 1
mkdir -p "$dir/crates/cargo-crap-core/tests/fixtures/src"
echo 'pub fn y() -> u32 { 4 }' > "$dir/crates/cargo-crap-core/tests/fixtures/src/y.rs"
echo "$double" >> "$dir/crates/cargo-crap-core/src/lib.rs"
with_shim "$dir" bash scripts/gates/mutants-diff.sh
check "a source file under a member's tests directory is not in the diff: lib.rs is" \
    "1" "$(grep -q '^diff --git a/crates/cargo-crap-core/src/lib.rs ' "$copied" 2> /dev/null && echo 1 || echo 0)"
check "a source file under a member's tests directory is not in the diff: tests/ is not" \
    "0" "$(grep -c '^diff --git .*crates/cargo-crap-core/tests/' "$copied" 2> /dev/null || true)"

# Scenario: just mutants and just mutants-all reach the core.
dir="$(mutants_repo)" || exit 1
with_shim "$dir" just --justfile "$repo/Justfile" --working-directory "$dir" mutants crates/cargo-crap-core/src/lib.rs
first="$calls"
first_status=$status
with_shim "$dir" just --justfile "$repo/Justfile" --working-directory "$dir" mutants-all
check "just mutants and just mutants-all reach the core: both exit 0" "0 0" "$first_status $status"
check "just mutants and just mutants-all reach the core: mutants FILE" \
    "[mutants][--workspace][--file][crates/cargo-crap-core/src/lib.rs]" "$first"
check "just mutants and just mutants-all reach the core: mutants-all" "[mutants][--workspace]" "$calls"

# A base ref that does not resolve is an error, not an empty diff: a
# shallow clone or a renamed base branch must not pass having measured
# nothing.
dir="$(mutants_repo)" || exit 1
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base no-such-ref
check "an unknown base ref fails" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "an unknown base ref is named" "1" "$(grep -q 'no-such-ref' <<< "$out" && echo 1 || echo 0)"
check "an unknown base ref runs no cargo" "" "$calls"

# An argument the script does not take is refused, not dropped.
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base main --jobs 4
check "an extra argument is refused" "2" "$status"
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base ""
check "an empty base ref is refused" "2" "$status"

# A git whose `diff` fails, as a corrupt object store or a missing commit
# makes it: the gate must fail, not report that nothing was measured.
broken_git="$scratch/broken-git"
mkdir -p "$broken_git"
real_git="$(command -v git)"
cat > "$broken_git/git" <<BROKEN
#!/usr/bin/env bash
if [ "\$1" = "\${BROKEN_GIT:-diff}" ]; then echo "fatal: simulated \$1 failure" >&2; exit 128; fi
exec "$real_git" "\$@"
BROKEN
chmod +x "$broken_git/git"
dir="$(mutants_repo)" || exit 1
(cd "$dir" && fixture_git checkout -q -b feature && echo "$double" >> crates/cargo-crap-core/src/lib.rs &&
    fixture_git commit -q -am double) > /dev/null 2>&1 || { echo "FAIL setting up the failing-diff case"; exit 1; }
with_shim "$dir" env PATH="$broken_git:$shim:$PATH" bash scripts/gates/mutants-diff.sh --base main
check "a failed git diff fails the gate with --base" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "a failed git diff runs no cargo with --base" "" "$calls"
echo "$double" >> "$dir/src/lib.rs"
with_shim "$dir" env PATH="$broken_git:$shim:$PATH" bash scripts/gates/mutants-diff.sh
check "a failed git diff fails the gate without --base" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"

# A staged deletion of a tracked file, recreated unstaged, leaves the path
# untracked. Measuring it must not reset the deletion out of the index.
dir="$(mutants_repo)" || exit 1
(cd "$dir" && fixture_git rm -q --cached src/lib.rs) > /dev/null 2>&1 || { echo "FAIL setting up the staged-deletion case"; exit 1; }
staged_before="$(cd "$dir" && fixture_git diff --cached --name-status)"
with_shim "$dir" bash scripts/gates/mutants-diff.sh
check "measuring keeps a staged deletion: exits 0" "0" "$status"
check "measuring keeps a staged deletion" "$staged_before" "$(cd "$dir" && fixture_git diff --cached --name-status)"
check "the staged deletion is in the index before" "D	src/lib.rs" "$staged_before"

# The same for the listing of untracked files: a new source file that a
# failed `git ls-files` drops from the diff is a change never measured.
dir="$(mutants_repo)" || exit 1
echo "$double" > "$dir/src/new.rs"
echo "$double" >> "$dir/crates/cargo-crap-core/src/lib.rs"
with_shim "$dir" env BROKEN_GIT=ls-files PATH="$broken_git:$shim:$PATH" bash scripts/gates/mutants-diff.sh
check "a failed git ls-files fails the gate" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "a failed git ls-files runs no cargo" "" "$calls"

# A main branch that shares no history with HEAD is an error, not a reason
# to fall back to the last commit: the core edit below the README commit
# would go unmeasured.
dir="$(mutants_repo)" || exit 1
(cd "$dir" && fixture_git checkout -q --orphan unrelated && echo "$double" >> crates/cargo-crap-core/src/lib.rs &&
    fixture_git add -A && fixture_git commit -q -m double && echo docs > README.md && fixture_git add README.md &&
    fixture_git commit -q -m readme) > /dev/null 2>&1 || { echo "FAIL setting up the unrelated-main case"; exit 1; }
with_shim "$dir" bash scripts/gates/mutants-diff.sh
check "a main with no merge base fails the gate" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "a main with no merge base is named" "1" "$(grep -q 'main' <<< "$out" && echo 1 || echo 0)"
check "a main with no merge base runs no cargo" "" "$calls"

if [ "$failures" -gt 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
