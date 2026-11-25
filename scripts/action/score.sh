#!/usr/bin/env bash
# Score the crate in the working directory. On a pull request (BASE set),
# score the base commit first, in a worktree of its own and under the pull
# request's config, and compare. Then the report in the job summary, the
# outputs, and the gate.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

fail_above="$(boolean fail-above "$CRAP_FAIL_ABOVE")"
fail_regression="$(boolean fail-regression "$CRAP_FAIL_REGRESSION")"
comment="$(boolean comment "$CRAP_COMMENT")"
if [ "$comment" = true ]; then
    problem="$(marker_problem "$CRAP_COMMENT_MARKER")"
    if [ -n "$problem" ]; then
        echo "::error::$problem"
        exit 1
    fi
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
partition_args ${args[@]+"${args[@]}"}
if [ "$fold_fail_above" = true ]; then
    echo "::notice::args: --fail-above works as fail-above: true"
    fail_above=true
fi
if [ "$fold_fail_regression" = true ]; then
    echo "::notice::args: --fail-regression works as fail-regression: true"
    fail_regression=true
fi

# The comment is posted only on a pull request, and then it is the
# pr-comment report, which fits a comment.
posting=false
if [ "$comment" = true ] && [ -n "${CRAP_PR_NUMBER:-}" ]; then
    posting=true
fi
in_git=false
if git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
    in_git=true
fi
head_dir="$(pwd)"

# Which of the flags the Action adds this cargo-crap knows: a `version`
# pinned to an older release has neither.
keyless cargo-crap --help > "$work/help.txt"
probed=()
supports_all_rows=false
if grep -q -- '--all-rows' "$work/help.txt"; then
    supports_all_rows=true
    probed+=(--all-rows)
fi
if [ "$has_no_cache" = false ] && grep -q -- '--no-cache' "$work/help.txt"; then
    probed+=(--no-cache)
fi

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

# The target directory Cargo builds the crate here into, or nothing when it
# cannot be read. Parsed as JSON, never matched as text: a Windows path is
# escaped there.
target_dir() {
    local json
    json="$(keyless cargo metadata --no-deps --format-version 1 2> /dev/null)" || return 0
    if command -v node > /dev/null; then
        printf '%s' "$json" | node -e '
            let s = "";
            process.stdin.on("data", d => s += d).on("end", () => {
                try {
                    const t = JSON.parse(s).target_directory;
                    if (typeof t === "string") process.stdout.write(t);
                } catch {}
            });' 2> /dev/null || true
    elif command -v python3 > /dev/null; then
        printf '%s' "$json" | python3 -I -c '
import json, sys
try:
    t = json.load(sys.stdin).get("target_directory")
except ValueError:
    t = None
if isinstance(t, str):
    sys.stdout.write(t)' 2> /dev/null || true
    fi
}

# `base` as a commit: resolved here, or fetched from origin. Any failure ends
# the run, since comparing with anything else would pass a regression.
resolve_base() {
    local resolved
    case "$base" in
        -*)
            echo "::error::base must name a commit, not start with -: $base"
            exit 1
            ;;
    esac
    if resolved="$(git rev-parse --verify --quiet --end-of-options "$base^{commit}")"; then
        base="$resolved"
    else
        # `origin/main` is the branch `main` on origin.
        git fetch -q --no-tags --depth=1 origin --end-of-options "${base#origin/}"
        base="$(git rev-parse --verify "FETCH_HEAD^{commit}")"
    fi
}

# Whether the working directory exists at the base commit.
base_has_workdir() {
    local prefix
    prefix="$(git rev-parse --show-prefix)"
    [ -z "$prefix" ] || git cat-file -e "$base:${prefix%/}" 2> /dev/null
}

# Refuse an absolute path in `args` or `coverage-args` that names the
# checkout: the base-commit run scores another tree, where it would still
# name the pull request's files.
refuse_checkout_paths() {
    local top word
    top="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
    for word in ${common[@]+"${common[@]}"}; do
        if names_checkout "$word" "$top"; then
            echo "::error::args: $word names the pull request's checkout, which the base-commit run does not score: use a path relative to working-directory"
            exit 1
        fi
    done
    for word in ${coverage_args[@]+"${coverage_args[@]}"}; do
        if names_checkout "$word" "$top"; then
            echo "::error::coverage-args: $word names the pull request's checkout, which the base-commit run does not score: use a path relative to working-directory"
            exit 1
        fi
    done
}

base_tree=""
# Remove the worktree this run added, and nothing else.
remove_tree() {
    cd "$head_dir"
    if [ -n "$base_tree" ] && ! git worktree remove --force "$base_tree" 2> /dev/null; then
        rm -rf "$base_tree"
        echo "::warning::git could not remove the base-commit worktree $base_tree: its record stays under .git/worktrees"
    fi
    base_tree=""
}

# Score `base` into `$work/base.json`: in a worktree of its own outside the
# checkout, which stays as the job left it, under the pull request's config,
# keyless so it never pays for triage. Called as a plain command, never as
# a condition, so any failure stops the run instead of comparing with the
# wrong tree.
score_base() {
    local head_top prefix pr_config tree dir found destination target
    head_top="$(cd "$(git rev-parse --show-toplevel)" && pwd -P)"
    prefix="$(git rev-parse --show-prefix)"
    pr_config="$(nearest_config)"
    if [ "$supports_all_rows" = false ]; then
        if [ -n "$pr_config" ] && config_limits "$pr_config"; then
            echo "::error::$(keyless cargo-crap --version) has no --all-rows, and $pr_config sets top or min, so the base-commit run would leave functions out and a rise in their scores would pass: use cargo-crap 0.10.0 or later, or give baseline"
            exit 1
        fi
        if [ "${#limits[@]}" -gt 0 ]; then
            echo "::notice::$(keyless cargo-crap --version) has no --all-rows: the base-commit run leaves --top and --min out"
        fi
    fi
    if [ -n "$pr_config" ]; then
        cp "$pr_config" "$work/pr-config.toml"
    fi
    # The checkout's own target directory, so the base run reuses what is
    # already built there, as it did checked out in place.
    target="$(target_dir)"
    tree="$(mktemp -d "$RUNNER_TEMP/cargo-crap-base.XXXXXX")"
    base_tree="$tree"
    trap remove_tree EXIT
    git worktree add -q --detach "$tree" "$base"
    tree="$(cd "$tree" && pwd -P)"
    if [ -f "$tree/.gitmodules" ] && ! git -C "$tree" submodule update -q --init --recursive; then
        echo "::error::could not check out the base commit's submodules: see the error above, or give baseline"
        exit 1
    fi
    # The base commit decides what its path is, and a symlink there could
    # lead out of the worktree: nothing is removed or run until it is
    # known to stay inside.
    dir="$tree/${prefix%/}"
    if ! dir="$(cd "$dir" 2> /dev/null && pwd -P)" || ! inside_dir "$dir" "$tree"; then
        echo "::error::${prefix%/} at the base commit is not a directory inside its worktree"
        exit 1
    fi
    cd "$dir"
    while found="$(nearest_config)" && [ -n "$found" ]; do
        rm "$found"
    done
    if [ -n "$pr_config" ]; then
        destination="$tree/${pr_config#"$head_top"/}"
        if ! inside_dir "$(cd "$(dirname "$destination")" && pwd -P)" "$tree"; then
            echo "::error::the pull request's config would land outside the base commit's worktree"
            exit 1
        fi
        cp "$work/pr-config.toml" "$destination"
    fi
    if [ -n "$target" ]; then
        CARGO_TARGET_DIR="$target" keyless cover "$work/base.lcov"
    else
        keyless cover "$work/base.lcov"
    fi
    report_run keyless cargo-crap --lcov "$work/base.lcov" \
        ${common[@]+"${common[@]}"} ${probed[@]+"${probed[@]}"} \
        --format json --output "$work/base.json"
    if [ ! -s "$work/base.json" ]; then
        echo "::error::the base-commit run wrote no report"
        exit 1
    fi
    trap - EXIT
    remove_tree
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
    if [ "$in_git" = false ]; then
        echo "::error::working-directory is not in a Git work tree, so the base commit cannot be scored: set base to '' or give baseline"
        exit 1
    fi
    refuse_checkout_paths
    resolve_base
    if base_has_workdir; then
        score_base
        compared=(--baseline "$work/base.json")
    else
        echo "::warning::$(git rev-parse --show-prefix) does not exist at the base commit, so there is nothing to compare with"
    fi
fi
if [ "${#compared[@]}" -eq 0 ]; then
    if [ "${#compared_only[@]}" -gt 0 ]; then
        echo "::notice::args: --show-unchanged has no effect with nothing to compare with"
    fi
    if [ "$fold_fail_regression" = true ]; then
        echo "::notice::args: --fail-regression has no effect with nothing to compare with"
    fi
fi

if [ -n "$given_lcov" ]; then
    if [ "${#coverage_args[@]}" -gt 0 ]; then
        echo "::warning::coverage-args has no effect with lcov given"
    fi
    lcov="$given_lcov"
else
    lcov="$work/lcov.info"
    keyless cover "$lcov"
fi

crap() {
    cargo-crap --lcov "$lcov" ${common[@]+"${common[@]}"} ${limits[@]+"${limits[@]}"} \
        ${compared[@]+"${compared[@]}"} "$@"
}

# Links at the pull request's own head, not the merge commit GITHUB_SHA
# names on a pull_request event: GitHub deletes the merge ref when the pull
# request closes and keeps the head ref, so the links outlive it.
# cargo-crap writes paths relative to the working directory, so a crate in a
# subdirectory carries it in the ref, percent-encoded as cargo-crap encodes
# the path after it: `blob/<sha>/crates/my%20shop/src/lib.rs`.
links=()
ref="${CRAP_HEAD_SHA:-${GITHUB_SHA:-}}"
if [ -n "$ref" ]; then
    prefix=""
    if [ "$in_git" = true ]; then
        prefix="$(git rev-parse --show-prefix)"
    fi
    links=(--commit-ref "$ref${prefix:+/$(url_path_encode "${prefix%/}")}")
fi
# The only run that may spend the triage budget.
report="$work/report.md"
shown=()
if [ "${#compared[@]}" -gt 0 ]; then
    shown=(${compared_only[@]+"${compared_only[@]}"})
fi
if [ "${#compared[@]}" -gt 0 ] || [ "$posting" = true ]; then
    format=pr-comment
else
    format=markdown
fi
report_run crap ${report_only[@]+"${report_only[@]}"} ${shown[@]+"${shown[@]}"} \
    --format "$format" ${links[@]+"${links[@]}"} --output "$report"
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
if ! badge="$(report_run keyless crap --format shields 2> "$work/badge.err")"; then
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
# step exits with, so the comment is posted before the step fails. A gate
# set in config applies here too. It runs keyless: the report run has spent
# the triage budget, and score regressions need no verdict. Its triage
# warnings are about that missing key, not the job's, so they are left out.
gate=()
if [ "$fail_above" = true ]; then
    gate+=(--fail-above)
fi
if [ "${#compared[@]}" -gt 0 ] && [ "$fail_regression" = true ]; then
    gate+=(--fail-regression)
fi
status=0
keyless crap ${gate[@]+"${gate[@]}"} --format json --output "$work/result.json" 2> "$work/gate.err" || status=$?
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
    # The gate counts gated signals over every function, so the count reads
    # a report of every row, not the rows `--top` and `--min` leave.
    signal_regressions=""
    if [ "$supports_all_rows" = true ]; then
        if ! report_run keyless crap --all-rows --format json --output "$work/signals.json" 2> "$work/signals.err"; then
            cat "$work/signals.err" >&2
            echo "::error::cargo-crap could not count the gated signals that rose"
            exit 1
        fi
        signal_regressions="$(signal_regressions_in "$work/signals.json" "${compared[1]}")"
    fi
    {
        echo "regressed=$regressed"
        echo "signal-regressions=$signal_regressions"
        echo "compared=true"
    } >> "$GITHUB_OUTPUT"
fi
{
    echo "status=$status"
    echo "comment=$posting"
} >> "$GITHUB_OUTPUT"
