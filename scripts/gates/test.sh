#!/usr/bin/env bash
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

command -v jq > /dev/null || { echo "FAIL jq is not installed"; exit 1; }

lint_workspace() {
    local dir="$scratch/lint"
    {
        mkdir -p "$dir/src" "$dir/crates/crapts/src" &&
            cp "$repo/Cargo.toml" "$repo/Cargo.lock" "$repo/clippy.toml" "$repo/rust-toolchain.toml" "$dir/" &&
            cp -R "$repo/crates/cargo-crap-core" "$dir/crates/" &&
            cp "$repo/crates/crapts/Cargo.toml" "$dir/crates/crapts/" &&
            : > "$dir/crates/crapts/src/lib.rs" &&
            printf 'fn main() {}\n' > "$dir/crates/crapts/src/main.rs" &&
            echo 'fn main() {}' > "$dir/src/main.rs" &&
            : > "$dir/src/lib.rs"
    } || { echo "FAIL building the lint workspace" >&2; exit 1; }
    echo "$dir"
}

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

for tool in just cargo-mutants cargo-nextest; do
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

x_module='pub fn a(x: u32) -> u32 { x + 1 }
pub fn c(x: u32) -> bool { x > 3 }
pub fn d(x: u32) -> u32 { x * 3 }
pub fn e(x: u32) -> bool { x == 7 }
pub fn f(x: u32) -> u32 { x - 1 }
pub fn g(x: u32) -> bool { x < 9 }'

move_repo() {
    local dir
    dir="$(mutants_repo)" || exit 1
    (cd "$dir" && fixture_git config diff.renames false && echo "$x_module" > src/x.rs &&
        echo 'pub mod x;' >> src/lib.rs && fixture_git add -A && fixture_git commit -q -m x) > /dev/null 2>&1 ||
        { echo "FAIL building the move fixture" >&2; exit 1; }
    echo "$dir"
}

move_x() {
    mkdir -p "$1/crates/cargo-crap-core/src" &&
        mv "$1/src/x.rs" "$1/crates/cargo-crap-core/src/x.rs" &&
        printf 'pub fn one() -> u32 {\n    1\n}\n' > "$1/src/lib.rs" &&
        echo 'pub mod x;' >> "$1/crates/cargo-crap-core/src/lib.rs"
}

section() {
    awk -v path="$2" '/^diff --git / { inside = ($0 ~ (" b/" path "$")) } inside' "$1" 2> /dev/null
}

core_x="crates/cargo-crap-core/src/x.rs"

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

dir="$(mutants_repo)" || exit 1
mkdir -p "$dir/crates/cargo-crap-core/tests/fixtures/src"
echo 'pub fn y() -> u32 { 4 }' > "$dir/crates/cargo-crap-core/tests/fixtures/src/y.rs"
echo "$double" >> "$dir/crates/cargo-crap-core/src/lib.rs"
with_shim "$dir" bash scripts/gates/mutants-diff.sh
check "a source file under a member's tests directory is not in the diff: lib.rs is" \
    "1" "$(grep -q '^diff --git a/crates/cargo-crap-core/src/lib.rs ' "$copied" 2> /dev/null && echo 1 || echo 0)"
check "a source file under a member's tests directory is not in the diff: tests/ is not" \
    "0" "$(grep -c '^diff --git .*crates/cargo-crap-core/tests/' "$copied" 2> /dev/null || true)"

dir="$(mutants_repo)" || exit 1
with_shim "$dir" just --justfile "$repo/Justfile" --working-directory "$dir" mutants crates/cargo-crap-core/src/lib.rs
first="$calls"
first_status=$status
with_shim "$dir" just --justfile "$repo/Justfile" --working-directory "$dir" mutants-all
check "just mutants and just mutants-all reach the core: both exit 0" "0 0" "$first_status $status"
check "just mutants and just mutants-all reach the core: mutants FILE" \
    "[mutants][--workspace][--file][crates/cargo-crap-core/src/lib.rs]" "$first"
check "just mutants and just mutants-all reach the core: mutants-all" "[mutants][--workspace]" "$calls"

dir="$scratch/consumer"
{
    mkdir -p "$dir/src" "$dir/tests" "$dir/crates/core/src" "$dir/.cargo" &&
        cp "$repo/.cargo/mutants.toml" "$dir/.cargo/" &&
        {
            printf '[package]\nname = "consumer-root"\nversion = "0.1.0"\nedition = "2024"\n\n'
            printf '[dependencies]\nconsumer-core = { path = "crates/core" }\n\n'
            printf '[workspace]\nmembers = ["crates/core"]\n\n'
            awk '/^\[profile\.mutants\]$/ { keep = 1; print; next } /^\[/ { keep = 0 } keep' "$repo/Cargo.toml"
        } > "$dir/Cargo.toml" &&
        printf '[package]\nname = "consumer-core"\nversion = "0.1.0"\nedition = "2024"\n' > "$dir/crates/core/Cargo.toml" &&
        printf 'pub fn double(x: u32) -> u32 {\n    x + x\n}\n' > "$dir/crates/core/src/lib.rs" &&
        : > "$dir/src/lib.rs" &&
        printf '#[test]\nfn double_of_three_is_six() {\n    assert_eq!(consumer_core::double(3), 6);\n}\n' > "$dir/tests/double.rs" &&
        printf 'target/\nmutants.out*/\n' > "$dir/.gitignore" &&
        (cd "$dir" && fixture_git init -q -b main && fixture_git add -A && fixture_git commit -q -m init) &&
        printf 'pub fn double(x: u32) -> u32 {\n    x * 2\n}\n' > "$dir/crates/core/src/lib.rs" &&
        (cd "$dir" && fixture_git diff > "$scratch/consumer.diff")
} > /dev/null 2>&1 || { echo "FAIL building the consumer fixture"; exit 1; }
out="$(cd "$dir" && CARGO_TARGET_DIR="$scratch/consumer-target" "$real_cargo" nextest run --workspace 2>&1)"
status=$?
[ "$status" -eq 0 ] || printf '%s\n' "$out"
check "a root consumer test can catch a core mutant: the root test passes on x * 2" "0" "$status"
# cargo-mutants baselines only the core, where nextest finds zero tests.
out="$(cd "$dir" && CARGO_TARGET_DIR="$scratch/consumer-target" NEXTEST_NO_TESTS=pass \
    "$real_cargo" mutants --workspace --in-diff "$scratch/consumer.diff" 2>&1)"
status=$?
[ "$status" -eq 0 ] || printf '%s\n' "$out"
check "a root consumer test can catch a core mutant: cargo mutants exits 0" "0" "$status"
check "a root consumer test can catch a core mutant: replace * with + in double is caught" "1" \
    "$(grep -q '^crates/core/src/lib.rs:.*replace \* with + in double$' "$dir/mutants.out/caught.txt" 2> /dev/null && echo 1 || echo 0)"
check "a root consumer test can catch a core mutant: none missed" "" \
    "$(cat "$dir/mutants.out/missed.txt" 2> /dev/null || echo 'no missed.txt')"

# The parser guard on recorded trees: $1 holds one `<package>.tree` per
# package. A package with no file reads as a `cargo tree` that failed.
guard() {
    out="$(cd "$repo" && PARSER_GUARD_TREES="$1" bash scripts/gates/parser-guard.sh 2>&1)"
    status=$?
}

trees() {
    local dir
    dir="$(mktemp -d "$scratch/trees.XXXXXX")" || exit 1
    printf 'cargo-crap v0.9.0 (/repo)\nanyhow v1.0.104\nsyn v2.0.119\n' > "$dir/cargo-crap.tree"
    printf 'cargo-crap-core v0.1.0 (/repo/crates/cargo-crap-core)\n' > "$dir/cargo-crap-core.tree"
    printf 'crapts v0.1.0\noxc_parser v0.159.0\ncargo-crap-core v0.1.0\nsynstructure v0.13.0\n' > "$dir/crapts.tree"
    echo "$dir"
}

out="$(cd "$repo" && bash scripts/gates/parser-guard.sh 2>&1)"
status=$?
check "the parser guard passes on this repository: exits 0" "0" "$status"
check "the parser guard passes on this repository: names what it checked" "1" \
    "$(grep -q 'checked cargo-crap, cargo-crap-core, crapts$' <<< "$out" && echo 1 || echo 0)"

# The syn cargo-crap links directly, which every build downloads. The lock
# holds more than one, and the full tree reaches another through a proc
# macro first, so the query stops at cargo-crap's own dependencies.
syn_version="$(cd "$repo" && "$real_cargo" tree -p cargo-crap -e normal --depth 1 --prefix none --locked 2> /dev/null |
    awk '$1 == "syn" { sub(/^v/, "", $2); print $2; exit }')"
guard_workspace() {
    local dir="$scratch/$1"
    {
        mkdir -p "$dir/src" "$dir/crates/cargo-crap-core/src" "$dir/crates/crapts/src" &&
            printf '[package]\nname = "cargo-crap"\nversion = "0.1.0"\nedition = "2024"\n\n[workspace]\nmembers = ["crates/cargo-crap-core", "crates/crapts"]\n' > "$dir/Cargo.toml" &&
            printf '[package]\nname = "cargo-crap-core"\nversion = "0.1.0"\nedition = "2024"\n\n%s\n' "$2" > "$dir/crates/cargo-crap-core/Cargo.toml" &&
            printf '[package]\nname = "crapts"\nversion = "0.1.0"\nedition = "2024"\n\n%s\n' "${3:-}" > "$dir/crates/crapts/Cargo.toml" &&
            : > "$dir/crates/crapts/src/lib.rs" &&
            printf 'fn main() {}\n' > "$dir/crates/crapts/src/main.rs" &&
            : > "$dir/src/lib.rs" && : > "$dir/crates/cargo-crap-core/src/lib.rs" &&
            (cd "$dir" && "$real_cargo" generate-lockfile --offline -q)
    } > /dev/null 2>&1 || { echo "FAIL building the $1 workspace (syn $syn_version cached?)" >&2; exit 1; }
    echo "$dir"
}

gated="$(guard_workspace gated "[dependencies]
syn = { version = \"=$syn_version\", optional = true }
[features]
parse = [\"dep:syn\"]")" || exit 1
out="$(cd "$gated" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "the parser guard sees a feature-gated parser dependency: exits 1" "1" "$status"
check "the parser guard sees a feature-gated parser dependency: names the core and syn" "1" \
    "$(grep -q "cargo-crap-core links syn v$syn_version" <<< "$out" && echo 1 || echo 0)"

dir="$(guard_workspace platform "[target.'cfg(windows)'.dependencies]
syn = \"=$syn_version\"")" || exit 1
out="$(cd "$dir" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "the parser guard sees a platform-gated parser dependency: exits 1" "1" "$status"
check "the parser guard sees a platform-gated parser dependency: names the core and syn" "1" \
    "$(grep -q "cargo-crap-core links syn v$syn_version" <<< "$out" && echo 1 || echo 0)"

dir="$(guard_workspace dev-only "[dev-dependencies]
syn = \"=$syn_version\"")" || exit 1
out="$(cd "$dir" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "a parser only the core's tests use is not a violation" "0" "$status"

dir="$(guard_workspace ts-build '' "[target.'cfg(windows)'.build-dependencies]
cargo-crap = { path = '../..' }")" || exit 1
out="$(cd "$dir" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "the parser guard rejects cargo-crap in a crapts target build" "1" "$status"
check "the parser guard identifies the forbidden front end" "1" \
    "$(grep -q 'crapts links cargo-crap v0.1.0' <<< "$out" && echo 1 || echo 0)"

dir="$(guard_workspace ts-cli '' "[dependencies]
syn = '=$syn_version'")" || exit 1
out="$(cd "$dir" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "a crapts package with a binary cannot link syn" "1" "$status"
check "the guard identifies the binary package's parser" "1" \
    "$(grep -q "crapts links syn v$syn_version" <<< "$out" && echo 1 || echo 0)"

dir="$(guard_workspace ts-dev '' "[dev-dependencies]
cargo-crap = { path = '../..' }
syn = '=$syn_version'")" || exit 1
out="$(cd "$dir" && bash "$repo/scripts/gates/parser-guard.sh" 2>&1)"
status=$?
check "the parser guard permits both crates as crapts test dependencies" "0" "$status"

dir="$(trees)" || exit 1
guard "$dir"
check "the parser guard allows Oxc, cargo-crap-core and synstructure in crapts" "0" "$status"
printf 'syn v2.0.119\n' >> "$dir/crapts.tree"
guard "$dir"
check "the parser guard rejects syn in crapts" "1" "$status"
check "the parser guard identifies the forbidden Rust parser" "1" \
    "$(grep -q 'crapts links syn v2.0.119' <<< "$out" && echo 1 || echo 0)"

dir="$(trees)" || exit 1
printf 'cargo-crap-core v0.1.0 (/repo/crates/cargo-crap-core)\nsyn v2.0.119\n' > "$dir/cargo-crap-core.tree"
guard "$dir"
check "the parser guard fails when the core links syn: exits 1" "1" "$status"
check "the parser guard fails when the core links syn: names it" "1" \
    "$(grep -q 'cargo-crap-core links syn v2.0.119' <<< "$out" && echo 1 || echo 0)"

dir="$(trees)" || exit 1
printf 'oxc_parser v0.159.0\n' >> "$dir/cargo-crap.tree"
guard "$dir"
check "the parser guard fails when cargo-crap links an Oxc crate: exits 1" "1" "$status"
check "the parser guard fails when cargo-crap links an Oxc crate: names it" "1" \
    "$(grep -q 'cargo-crap links oxc_parser v0.159.0' <<< "$out" && echo 1 || echo 0)"

dir="$(trees)" || exit 1
guard "$dir"
check "syn in cargo-crap's tree is not a violation" "0" "$status"
check "recorded trees are named in the output" "1" \
    "$(grep -q "recorded trees from $dir" <<< "$out" && echo 1 || echo 0)"

dir="$(trees)" || exit 1
cp "$dir/cargo-crap-core.tree" "$dir/crapts.tree"
guard "$dir"
check "a tree from another package cannot stand in for the crapts binary package" "1" "$status"

# The core's rules: `syn ` with its space, so a crate whose name only starts
# with syn is not a parser, and `oxc_` as for cargo-crap.
dir="$(trees)" || exit 1
printf 'synstructure v0.13.2\n' >> "$dir/cargo-crap-core.tree"
guard "$dir"
check "a crate named syn-something in the core is not syn" "0" "$status"
printf 'oxc_ast v0.159.0\n' >> "$dir/cargo-crap-core.tree"
guard "$dir"
check "the core may not link an Oxc crate" "1" "$status"
check "the core's Oxc crate is named" "1" \
    "$(grep -q 'cargo-crap-core links oxc_ast v0.159.0' <<< "$out" && echo 1 || echo 0)"

dir="$(trees)" || exit 1
: > "$dir/cargo-crap-core.tree"
guard "$dir"
check "a guard that saw an empty tree does not pass: exits 1" "1" "$status"
check "a guard that saw an empty tree does not pass: says so" "1" \
    "$(grep -q 'cargo-crap-core: no tree was read' <<< "$out" && echo 1 || echo 0)"
rm "$dir/cargo-crap-core.tree"
guard "$dir"
check "a guard whose cargo tree failed does not pass: exits 1" "1" "$status"
check "a guard whose cargo tree failed does not pass: says so" "1" \
    "$(grep -q 'cargo-crap-core: no tree was read' <<< "$out" && echo 1 || echo 0)"

# A cargo tree that prints part of a tree and then fails is a failed read,
# not a tree to judge.
partial="$scratch/partial-cargo"
mkdir -p "$partial"
cat > "$partial/cargo" <<'PARTIAL'
#!/usr/bin/env bash
echo "cargo-crap v0.9.0 (/repo)"
exit 101
PARTIAL
chmod +x "$partial/cargo"
out="$(cd "$repo" && PATH="$partial:$PATH" bash scripts/gates/parser-guard.sh 2>&1)"
status=$?
check "a guard whose cargo tree printed a tree and failed does not pass: exits 1" "1" "$status"
check "a guard whose cargo tree printed a tree and failed does not pass: says so" "1" \
    "$(grep -q 'cargo-crap: no tree was read' <<< "$out" && echo 1 || echo 0)"

# The guard runs under macOS's system bash 3.2 as well as a current one.
if [ -x /bin/bash ]; then
    dir="$(trees)" || exit 1
    out="$(cd "$repo" && PARSER_GUARD_TREES="$dir" /bin/bash scripts/gates/parser-guard.sh 2>&1)"
    status=$?
    check "the parser guard runs under /bin/bash: exits 0" "0" "$status"
    printf 'syn v2.0.119\n' >> "$dir/cargo-crap-core.tree"
    out="$(cd "$repo" && PARSER_GUARD_TREES="$dir" /bin/bash scripts/gates/parser-guard.sh 2>&1)"
    check "the parser guard runs under /bin/bash: names a violation" "1" \
        "$(grep -q 'cargo-crap-core links syn v2.0.119' <<< "$out" && echo 1 || echo 0)"
    # bash 3.2 backs a here-string with a temp file, so a shell that cannot
    # write one (an unwritable TMPDIR and /tmp, here a zero file-size limit
    # with SIGXFSZ ignored) breaks any scan that reads the tree through one.
    # The guard must still name the violation.
    out="$(cd "$repo" && trap '' XFSZ && ulimit -f 0 && PARSER_GUARD_TREES="$dir" /bin/bash scripts/gates/parser-guard.sh 2>&1)"
    status=$?
    check "the parser guard under /bin/bash that cannot write a temp file: exits 1" "1" "$status"
    check "the parser guard under /bin/bash that cannot write a temp file: names the violation" "1" \
        "$(grep -q 'cargo-crap-core links syn v2.0.119' <<< "$out" && echo 1 || echo 0)"
fi

# A base ref that does not resolve is an error, not an empty diff: a
# shallow clone or a renamed base branch must not pass having measured
# nothing.
dir="$(mutants_repo)" || exit 1
with_shim "$dir" bash scripts/gates/mutants-diff.sh --base no-such-ref
check "an unknown base ref fails" "1" "$([ "$status" -ne 0 ] && echo 1 || echo 0)"
check "an unknown base ref is named" "1" "$(grep -q 'no-such-ref' <<< "$out" && echo 1 || echo 0)"
check "an unknown base ref runs no cargo" "" "$calls"

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
