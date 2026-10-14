#!/usr/bin/env bash
# Score the crate in the working directory: coverage, the markdown report in
# the job summary, the outputs, and the gate.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

work="$RUNNER_TEMP/cargo-crap"
mkdir -p "$work"
# `args` and `coverage-args` are shell words, quotes and all, never globs.
args=()
coverage_args=()
set -f
eval "args=($CRAP_ARGS)"
eval "coverage_args=($CRAP_COVERAGE_ARGS)"
set +f

lcov="$work/lcov.info"
cargo llvm-cov --lcov --output-path "$lcov" ${coverage_args[@]+"${coverage_args[@]}"}

crap() {
    cargo-crap --lcov "$lcov" ${args[@]+"${args[@]}"} "$@"
}

report="$work/report.md"
crap --format markdown --output "$report"
cat "$report" >> "$GITHUB_STEP_SUMMARY"
{
    echo "crappy=$(crappy_from_badge "$(crap --format shields)")"
    echo "report=$report"
} >> "$GITHUB_OUTPUT"

if [ "$FAIL_ABOVE" = true ]; then
    crap --fail-above --format markdown --output "$work/gate.md"
fi
