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
comment="$(boolean comment "$CRAP_COMMENT")"
if [ "$comment" = true ] && [ -z "$CRAP_COMMENT_MARKER" ]; then
    echo "::error::comment-marker is empty: it would match every comment on the pull request"
    exit 1
fi
base="${CRAP_BASE:-}"
given_lcov="${CRAP_LCOV:-}"
given_baseline="${CRAP_BASELINE:-}"
file_input lcov "$given_lcov"
file_input baseline "$given_baseline"
# Each step its own directory, so two steps in one job keep their reports.
work="$(mktemp -d "$RUNNER_TEMP/cargo-crap.XXXXXX")"
# Split before anything runs, through files so an empty word survives, as
# plain commands so input that cannot be split stops the step here.
words_of args "$CRAP_ARGS" > "$work/args.words"
words_of coverage-args "$CRAP_COVERAGE_ARGS" > "$work/coverage-args.words"
args=()
coverage_args=()
while IFS= read -r word; do args+=("$word"); done < "$work/args.words"
while IFS= read -r word; do coverage_args+=("$word"); done < "$work/coverage-args.words"

cover() {
    cargo llvm-cov --lcov --output-path "$1" ${coverage_args[@]+"${coverage_args[@]}"}
}

# Run a command with every provider key cleared, which cargo-crap reads as
# unset, so it sends no triage request. Triage is all or nothing: with a
# group left to ask and no key, the run reports no verdict at all, which
# none of the unpaid runs needs.
unpaid() {
    TYPESAFE_API_KEY='' OPENAI_API_KEY='' "$@"
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

# `base` as a commit: resolved here, or fetched from origin. Any failure ends
# the run, since comparing with anything else would pass a regression.
resolve_base() {
    if git cat-file -e "$base^{commit}" 2> /dev/null; then
        base="$(git rev-parse "$base^{commit}")"
    else
        git fetch -q --no-tags --depth=1 origin "$base"
        base="$(git rev-parse --verify "FETCH_HEAD^{commit}")"
    fi
}

# Whether the working directory exists at the base commit.
base_has_workdir() {
    local prefix
    prefix="$(git rev-parse --show-prefix)"
    [ -z "$prefix" ] || git cat-file -e "$base:${prefix%/}" 2> /dev/null
}

# Score `base` into `$work/base.json`: checked out in place, under the pull
# request's config, with every provider key cleared so it never pays for
# triage. Called as a plain command, never as a condition, so any failure
# stops the run instead of comparing with the wrong tree.
score_base() {
    local found
    head="$(git rev-parse HEAD)"
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
    unpaid cargo-crap --lcov "$work/base.lcov" \
        ${args[@]+"${args[@]}"} --format json --output "$work/base.json"
    trap - EXIT
    restore
}

compared=()
if [ -n "$given_baseline" ]; then
    if [ -n "$base" ]; then
        echo "::notice::comparing with $given_baseline, not with a run of the base commit"
    fi
    compared=(--baseline "$given_baseline")
elif [ -n "$base" ] && [ -n "$given_lcov" ]; then
    if [ "$fail_regression" = true ]; then
        echo "::error::with lcov given, the base commit has no coverage to score: give baseline as well, or set fail-regression to false"
        exit 1
    fi
    echo "::warning::with lcov given and no baseline, there is nothing to compare with"
elif [ -n "$base" ]; then
    resolve_base
    if base_has_workdir; then
        score_base
        compared=(--baseline "$work/base.json")
    else
        echo "::warning::$(git rev-parse --show-prefix) does not exist at the base commit, so there is nothing to compare with"
    fi
fi

if [ -n "$given_lcov" ]; then
    if [ "${#coverage_args[@]}" -gt 0 ]; then
        echo "::warning::coverage-args has no effect with lcov given"
    fi
    lcov="$given_lcov"
else
    lcov="$work/lcov.info"
    cover "$lcov"
fi

crap() {
    cargo-crap --lcov "$lcov" ${args[@]+"${args[@]}"} ${compared[@]+"${compared[@]}"} "$@"
}

# Links at the pull request's own head, not the merge commit GITHUB_SHA
# names on a pull_request event: GitHub deletes the merge ref when the pull
# request closes and keeps the head ref, so the links outlive it.
# cargo-crap writes paths relative to the working directory, so a crate in a
# subdirectory carries it in the ref: `blob/<sha>/crates/foo/src/lib.rs`.
links=()
ref="${CRAP_HEAD_SHA:-${GITHUB_SHA:-}}"
if [ -n "$ref" ]; then
    prefix="$(git rev-parse --show-prefix)"
    links=(--commit-ref "$ref${prefix:+/${prefix%/}}")
fi
report="$work/report.md"
if [ "${#compared[@]}" -gt 0 ]; then
    crap --format pr-comment ${links[@]+"${links[@]}"} --output "$report"
else
    crap --format markdown ${links[@]+"${links[@]}"} --output "$report"
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
if ! badge="$(unpaid crap --format shields 2> "$work/badge.err")"; then
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
# count. Its status goes to the `status` output, which the Action's last
# step exits with, so the comment is posted before the step fails. It runs
# unpaid: the report run has spent the triage budget, a run with a key would
# spend it again, and score regressions need no verdict. Its triage warnings
# are about that missing key, not the job's, so they are left out.
gate=()
if [ "$fail_above" = true ]; then
    gate+=(--fail-above)
fi
if [ "${#compared[@]}" -gt 0 ] && [ "$fail_regression" = true ]; then
    gate+=(--fail-regression)
fi
status=0
unpaid crap ${gate[@]+"${gate[@]}"} --format json --output "$work/result.json" 2> "$work/gate.err" || status=$?
grep -v -e '^warning: duplicate triage skipped' -e '^warning: the triage budget' "$work/gate.err" >&2 || true
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
    echo "compared=true" >> "$GITHUB_OUTPUT"
fi
{
    echo "status=$status"
    echo "comment=$comment"
} >> "$GITHUB_OUTPUT"
