#!/usr/bin/env bash
# Score the crate in the working directory. On a pull request (BASE set),
# score the base commit first, in place and under the pull request's config,
# and compare. Then the report in the job summary, the outputs, and the gate.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

fail_above="$(boolean fail-above "$CRAP_FAIL_ABOVE")"
fail_regression="$(boolean fail-regression "$CRAP_FAIL_REGRESSION")"
base="${CRAP_BASE:-}"
# Each step its own directory, so two steps in one job keep their reports.
work="$(mktemp -d "$RUNNER_TEMP/cargo-crap.XXXXXX")"
args=()
coverage_args=()
while IFS= read -r word; do args+=("$word"); done < <(split_words "$CRAP_ARGS")
while IFS= read -r word; do coverage_args+=("$word"); done < <(split_words "$CRAP_COVERAGE_ARGS")

cover() {
    cargo llvm-cov --lcov --output-path "$1" ${coverage_args[@]+"${coverage_args[@]}"}
}

# The `.cargo-crap.toml` cargo-crap would read from here, up to the
# repository's root, or nothing.
nearest_config() {
    local dir top
    dir="$(pwd)"
    top="$(git rev-parse --show-toplevel)"
    while :; do
        if [ -f "$dir/.cargo-crap.toml" ]; then
            echo "$dir/.cargo-crap.toml"
            return 0
        fi
        if [ "$dir" = "$top" ] || [ "$dir" = / ]; then
            return 0
        fi
        dir="$(dirname "$dir")"
    done
}

# Score `base` into `$work/base.json`: checked out in place, under the pull
# request's config, with every provider key cleared so it never pays for
# triage. The pull request's commit, and an untracked config, come back after.
score_base() {
    local head pr_config found
    head="$(git rev-parse HEAD)"
    pr_config="$(nearest_config)"
    if [ -n "$pr_config" ]; then
        cp "$pr_config" "$work/pr-config.toml"
    fi
    if ! git cat-file -e "$base^{commit}" 2> /dev/null; then
        git fetch -q --no-tags --depth=1 origin "$base"
    fi
    git checkout -q --detach --force "$base"
    while found="$(nearest_config)" && [ -n "$found" ]; do
        rm "$found"
    done
    if [ -n "$pr_config" ]; then
        cp "$work/pr-config.toml" "$pr_config"
    fi
    cover "$work/base.lcov"
    TYPESAFE_API_KEY='' OPENAI_API_KEY='' cargo-crap --lcov "$work/base.lcov" \
        ${args[@]+"${args[@]}"} --format json --output "$work/base.json"
    git checkout -q --detach --force "$head"
    if [ -n "$pr_config" ]; then
        cp "$work/pr-config.toml" "$pr_config"
    fi
}

compared=()
if [ -n "$base" ]; then
    score_base
    compared=(--baseline "$work/base.json")
fi

lcov="$work/lcov.info"
cover "$lcov"

crap() {
    cargo-crap --lcov "$lcov" ${args[@]+"${args[@]}"} ${compared[@]+"${compared[@]}"} "$@"
}

report="$work/report.md"
if [ -n "$base" ]; then
    crap --format pr-comment --output "$report"
else
    crap --format markdown --output "$report"
fi
# The runner drops a job summary over 1 MiB whole, so a report that large is
# left to the `report` output.
if [ "$(wc -c < "$report")" -lt 1000000 ]; then
    cat "$report" >> "$GITHUB_STEP_SUMMARY"
else
    echo "The cargo-crap report is too large for the job summary. It is at the step's \`report\` output." >> "$GITHUB_STEP_SUMMARY"
fi
cp "$GITHUB_STEP_SUMMARY" "$work/summary.md"

# Quiet: a config with duplicates on makes every badge run say they do not
# apply to it. A real failure leaves no count, which is checked below.
badge="$(crap --format shields 2> /dev/null)"
crappy="$(crappy_from_badge "$badge")"
if ! is_count "$crappy"; then
    echo "::error::could not count the functions above the threshold from: $badge"
    exit 1
fi
{
    echo "crappy=$crappy"
    echo "report=$report"
} >> "$GITHUB_OUTPUT"
if [ -n "$base" ]; then
    crap --format json --output "$work/delta.json"
    regressed="$(jq '[.entries[] | select(.status == "regressed")] | length' "$work/delta.json")"
    echo "regressed=$regressed" >> "$GITHUB_OUTPUT"
fi

gate=()
if [ "$fail_above" = true ]; then
    gate+=(--fail-above)
fi
if [ -n "$base" ] && [ "$fail_regression" = true ]; then
    gate+=(--fail-regression)
fi
if [ "${#gate[@]}" -gt 0 ]; then
    crap "${gate[@]}" --format json --output "$work/gate.json"
fi
