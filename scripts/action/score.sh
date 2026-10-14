#!/usr/bin/env bash
# Score the crate in the working directory: coverage, the markdown report in
# the job summary, the outputs, and the gate.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

fail_above="$(boolean fail-above "$CRAP_FAIL_ABOVE")"
# Each step its own directory, so two steps in one job keep their reports.
work="$(mktemp -d "$RUNNER_TEMP/cargo-crap.XXXXXX")"
args=()
coverage_args=()
while IFS= read -r word; do args+=("$word"); done < <(split_words "$CRAP_ARGS")
while IFS= read -r word; do coverage_args+=("$word"); done < <(split_words "$CRAP_COVERAGE_ARGS")

lcov="$work/lcov.info"
cargo llvm-cov --lcov --output-path "$lcov" ${coverage_args[@]+"${coverage_args[@]}"}

crap() {
    cargo-crap --lcov "$lcov" ${args[@]+"${args[@]}"} "$@"
}

report="$work/report.md"
crap --format markdown --output "$report"
# The runner drops a job summary over 1 MiB whole, so a report that large is
# left to the `report` output.
if [ "$(wc -c < "$report")" -lt 1000000 ]; then
    cat "$report" >> "$GITHUB_STEP_SUMMARY"
else
    echo "The cargo-crap report is too large for the job summary. It is at the step's \`report\` output." >> "$GITHUB_STEP_SUMMARY"
fi
cp "$GITHUB_STEP_SUMMARY" "$work/summary.md"

badge="$(crap --format shields)"
crappy="$(crappy_from_badge "$badge")"
if ! is_count "$crappy"; then
    echo "::error::could not count the functions above the threshold from: $badge"
    exit 1
fi
{
    echo "crappy=$crappy"
    echo "report=$report"
} >> "$GITHUB_OUTPUT"

if [ "$fail_above" = true ]; then
    crap --fail-above --format markdown --output "$work/gate.md"
fi
