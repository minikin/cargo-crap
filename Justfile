# List available recipes
default:
    @just --list

# Run all tests
test:
    cargo nextest run --all-targets
    cargo test --doc

# Apply formatting
fmt:
    cargo fmt --all

# Check formatting and lints (mirrors CI)
lint:
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings

# Fast compile check without building test binaries
check:
    cargo check --all-targets

# Run all CI checks locally (requires cargo-nextest: cargo binstall cargo-nextest)
ci: lint test

# The CRAP recipes below run cargo-crap from *this* source tree
# (`cargo run --release`), never the installed `cargo crap`: the tool under
# test is the tool doing the measuring, so gating on a released binary would
# score code that isn't the code under review. Fixtures are excluded to match
# the Self-score job in CI — deliberately crappy sample projects are inputs,
# not source.
crap_bin := "cargo run --release --"
crap_scope := "--workspace --exclude 'tests/fixtures/**'"

# Line coverage summary; fails mechanically below 90% lines
cov:
    cargo llvm-cov nextest --all-targets --summary-only --fail-under-lines 90

# Dogfood: coverage + CRAP gate, fails if any function scores above threshold 15
crap:
    cargo llvm-cov --lcov --output-path lcov.info --workspace
    {{crap_bin}} --lcov lcov.info {{crap_scope}} --threshold 15 --fail-above

# Record a CRAP baseline (run before starting a feature)
crap-baseline:
    cargo llvm-cov --lcov --output-path lcov.info --workspace
    {{crap_bin}} --lcov lcov.info {{crap_scope}} --format json --sort file --output crap-baseline.json
    @echo "CRAP baseline saved to crap-baseline.json"

# CRAP delta vs the recorded baseline: fails on threshold breach OR any regression
crap-delta:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ ! -f crap-baseline.json ]; then
        echo "No crap-baseline.json — run 'just crap-baseline' and commit it first." >&2
        exit 1
    fi
    cargo llvm-cov --lcov --output-path lcov.info --workspace
    {{crap_bin}} --lcov lcov.info {{crap_scope}} --threshold 15 --fail-above \
        --baseline crap-baseline.json --fail-regression

# Full local gate: format, lint, tests, coverage, CRAP
dev: fmt lint test crap

# Mutation tests for a specific file: just mutants src/delta.rs
mutants FILE:
    cargo mutants --file {{FILE}}

# Mutation tests on every file (slow)
mutants-all:
    cargo mutants

# Mutation tests on changed lines only
mutants-diff:
    #!/usr/bin/env bash
    set -euo pipefail
    # Which lines: uncommitted changes vs HEAD, else the branch base, else
    # the last commit.
    # Both pathspecs: 'src/**/*.rs' catches files inside subdirectories
    # (src/report/sarif.rs, etc.) while 'src/*.rs' catches the top level.
    paths=('src/*.rs' 'src/**/*.rs')
    # New files aren't in `git diff HEAD` — intent-to-add makes their full
    # content show up in the diff; reset afterwards to leave the index as-is.
    # NUL-delimited into an array: paths with spaces stay whole.
    untracked=()
    while IFS= read -r -d '' f; do untracked+=("$f"); done \
        < <(git ls-files -z --others --exclude-standard -- "${paths[@]}" 2>/dev/null || true)
    if [ "${#untracked[@]}" -gt 0 ]; then git add -N -- "${untracked[@]}"; fi
    diff_file=$(mktemp)
    trap 'rm -f "$diff_file"' EXIT
    git diff HEAD -- "${paths[@]}" > "$diff_file" || true
    if [ "${#untracked[@]}" -gt 0 ]; then git reset -q -- "${untracked[@]}"; fi
    if [ ! -s "$diff_file" ]; then
        # A clean tree is not a measured tree: src changes committed earlier
        # on this branch still need mutating, so diff against the branch base.
        base=""
        for ref in origin/main origin/master main master; do
            if candidate=$(git merge-base HEAD "$ref" 2>/dev/null); then base="$candidate"; break; fi
        done
        if [ -n "$base" ] && [ "$base" != "$(git rev-parse HEAD)" ]; then
            git diff "$base" HEAD -- "${paths[@]}" > "$diff_file" || true
        fi
    fi
    if [ ! -s "$diff_file" ]; then
        git diff HEAD~1 HEAD -- "${paths[@]}" > "$diff_file" 2>/dev/null || true
    fi
    if [ ! -s "$diff_file" ]; then
        # An honest gate says what it did not measure — it never reports the
        # absence of survivors as evidence about a change it cannot see.
        # For everything mutants can measure, use `just mutants-all`.
        echo "No src/ changes — outside the mutation gate's reach; nothing was measured"
        exit 0
    fi
    echo "Running mutants on changed lines (--in-diff)"
    cargo mutants --in-diff "$diff_file"

# The pre-commit gate: the full fast gate, then mutants on what changed
dev-mutants-diff: dev mutants-diff

# Full validation including mutation tests (slow)
dev-full: dev mutants-all

# Upgrade Keeler itself (KEELER_REF=v0.1.0 just keeler-upgrade to pin a tag)
keeler-upgrade:
    curl -fsSL https://raw.githubusercontent.com/minikin/keeler/main/install.sh | bash -s .
