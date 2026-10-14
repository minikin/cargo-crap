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
given_lcov="${CRAP_LCOV:-}"
given_baseline="${CRAP_BASELINE:-}"
for given in "$given_lcov" "$given_baseline"; do
    if [ -n "$given" ] && [ ! -f "$given" ]; then
        echo "::error::$given is not a file (paths are relative to the working directory)"
        exit 1
    fi
done
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
# repository's root and never past it, or nothing. Physical paths on both
# sides, so a symlinked ancestor or Git Bash's /d/ spelling cannot hide the
# root.
nearest_config() {
    local dir top
    dir="$(pwd -P)"
    top="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
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

head=""
pr_config=""
stashed=false
# Put the pull request back as the job prepared it: its commit, its local
# changes and an untracked config. Run on the way out of the base run,
# whether it finished or failed.
restore() {
    git checkout -q --detach --force "$head"
    if [ "$stashed" = true ]; then
        git stash pop -q
        stashed=false
    fi
    if [ -n "$pr_config" ]; then
        cp "$work/pr-config.toml" "$pr_config"
    fi
}

# Score `base` into `$work/base.json`: checked out in place, under the pull
# request's config, with every provider key cleared so it never pays for
# triage. Fails, with the pull request restored, when the base commit has no
# working directory to score.
score_base() {
    local found prefix
    head="$(git rev-parse HEAD)"
    if git cat-file -e "$base^{commit}" 2> /dev/null; then
        base="$(git rev-parse "$base^{commit}")"
    else
        git fetch -q --no-tags --depth=1 origin "$base"
        base="$(git rev-parse FETCH_HEAD)"
    fi
    prefix="$(git rev-parse --show-prefix)"
    if [ -n "$prefix" ] && ! git cat-file -e "$base:${prefix%/}" 2> /dev/null; then
        echo "::warning::$prefix does not exist at the base commit, so there is nothing to compare with"
        return 1
    fi
    pr_config="$(nearest_config)"
    if [ -n "$pr_config" ]; then
        cp "$pr_config" "$work/pr-config.toml"
    fi
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
        git stash push -q
        stashed=true
    fi
    trap restore EXIT
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
    trap - EXIT
    restore
}

compared=()
if [ -n "$given_baseline" ]; then
    compared=(--baseline "$given_baseline")
elif [ -n "$base" ] && [ -n "$given_lcov" ]; then
    echo "::warning::with lcov given, the base commit has no coverage to score: give baseline as well to compare"
elif [ -n "$base" ] && score_base; then
    compared=(--baseline "$work/base.json")
fi

if [ -n "$given_lcov" ]; then
    lcov="$given_lcov"
else
    lcov="$work/lcov.info"
    cover "$lcov"
fi

crap() {
    cargo-crap --lcov "$lcov" ${args[@]+"${args[@]}"} ${compared[@]+"${compared[@]}"} "$@"
}

report="$work/report.md"
if [ "${#compared[@]}" -gt 0 ]; then
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

# Its stderr kept aside: with duplicates on in config, every badge run says
# they do not apply to it, and only a failure should show.
if ! badge="$(crap --format shields 2> "$work/badge.err")"; then
    cat "$work/badge.err" >&2
    echo "::error::cargo-crap could not count the functions above the threshold"
    exit 1
fi
crappy="$(crappy_from_badge "$badge")"
if ! is_count "$crappy"; then
    echo "::error::could not count the functions above the threshold from: $badge"
    exit 1
fi
{
    echo "crappy=$crappy"
    echo "report=$report"
} >> "$GITHUB_OUTPUT"

# One JSON run is both the gate and, on a pull request, the regression
# count: its status is held until the outputs are written.
gate=()
if [ "$fail_above" = true ]; then
    gate+=(--fail-above)
fi
if [ "${#compared[@]}" -gt 0 ] && [ "$fail_regression" = true ]; then
    gate+=(--fail-regression)
fi
status=0
crap ${gate[@]+"${gate[@]}"} --format json --output "$work/result.json" || status=$?
# 2 is cargo-crap failing to run, not a gate: nothing to count.
if [ "$status" -ge 2 ]; then
    exit "$status"
fi
if [ "${#compared[@]}" -gt 0 ]; then
    regressed="$(grep -c '"status": "regressed"' "$work/result.json" || true)"
    if ! is_count "$regressed"; then
        echo "::error::could not count the regressions in $work/result.json"
        exit 1
    fi
    echo "regressed=$regressed" >> "$GITHUB_OUTPUT"
fi
exit "$status"
