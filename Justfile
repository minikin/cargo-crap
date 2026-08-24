# List available recipes
default:
    @just --list

# Parameters reach a recipe through the environment, under their own
# names, rather than as text spliced into its body. Without this, `just`
# interpolates {{SPEC}} literally, so a spec whose file name holds $( … )
# runs it when the recipe is parsed — before any guard inside the body
# can refuse it. `$SPEC` reads as `{{SPEC}}` did and cannot.
set export

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

# "Main" is one thing, decided once: the first of the four names below
# that exists in this repository. `mutants-diff` diffs against it and
# `keeler-land` tells it from a feature branch — through this one helper, so
# no two recipes can disagree about where main is and be silently wrong in
# different directions. Private (the leading `_`): it prints a ref for
# other recipes, not a line for a human reading `just --list`.
_main-ref:
    #!/usr/bin/env bash
    set -euo pipefail
    for ref in origin/main origin/master main master; do
        if git rev-parse --verify --quiet "$ref" >/dev/null 2>&1; then
            echo "$ref"
            exit 0
        fi
    done
    echo "no main branch here: none of origin/main, origin/master, main, master exists" >&2
    exit 1

# Diff-based by construction: all three of these measure this branch's
# own changes, and `crap-delta` reads the baseline without ever writing
# it. Moving the baseline is `just keeler-land`'s job, on main, at fan-in;
# CI refuses a keeler/* pull request whose diff touched it.
#
# `just dev` is untouched — an adopter on the linear road runs exactly the
# recipe they always did, and a spawned agent runs this one instead. The
# three go through `just` on PATH rather than recipe dependencies, because
# the order is the contract: dependencies all run before the body, and the
# sequence would stop being observable.
#
# The line below is the one `just --list` shows — `just` carries the last
# comment line above a recipe and no other, which is why each graph-mode
# recipe here ends its documentation with the one an adopter should read
# there. The rationale above it is for whoever opens this file.
#
# Graph mode: the gate a task branch runs — dev, then crap-delta, then mutants-diff.
keeler-branch:
    #!/usr/bin/env bash
    set -euo pipefail
    just dev
    # The delta gate needs a committed baseline to measure against — the
    # same condition /keeler:qa and the shipped workflow already check.
    # Without one, `dev`'s absolute CRAP threshold is the whole gate.
    # `git cat-file` and not `-f`: a baseline generated locally and left
    # uncommitted would have crap-delta measure this branch against itself,
    # a zero delta by construction and a gate that checked nothing.
    if git cat-file -e HEAD:crap-baseline.json 2>/dev/null; then
        just crap-delta
    else
        echo "no crap-baseline.json committed — threshold only, no delta gate"
    fi
    just mutants-diff

# The fork after approval, as a recipe. /keeler:spec asks which road once
# it has set `Status: Approved`, and the "graph" answer runs this — so the
# branch, the checkout and the one commit are a thing a human can also run
# by hand, after answering "linearly" and changing their mind. Three lines
# in a command file would behave one way for the agent and another for the
# human, and only one of them would be tested.
#
# Cut from main and nowhere else, found through `_main-ref` as every other
# recipe finds it: a feature cut from another feature's branch, or from a
# task branch, is a feature nested where nothing else in graph mode expects
# one. The branch's own name is the exception — re-approving an amended
# spec happens there, and checking out the branch you are already on is
# the same branch either way.
#
# The commit is of one path. An unrelated change in the working tree, or
# staged beside it, is not this commit's and stays exactly where it was:
# the human's consent was for the spec, and the spec is what is committed.
#
# Graph mode: cut feat/<spec-slug> from main and commit the approved spec there — `just keeler-feature-branch specs/01-foo.md`.
keeler-feature-branch SPEC:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="$SPEC"
    [ -f "$spec" ] || { echo "keeler-feature-branch: $spec is not a file" >&2; exit 1; }
    root="$(git rev-parse --show-toplevel)"
    # `pwd -P`, because `--show-toplevel` answers physically: a repository
    # reached through a symlink — /tmp on macOS is one — has a logical path
    # that is not a prefix of git's, and the guard below would refuse a
    # spec sitting in the repository it is looking at.
    spec_abs="$(cd "$(dirname "$spec")" && pwd -P)/$(basename "$spec")"
    case "$spec_abs" in
        "$root"/*) ;;
        *)
            echo "keeler-feature-branch: $spec_abs is not inside $root — a feature branch is cut in the repository the spec lives in." >&2
            exit 1
            ;;
    esac
    rel="${spec_abs#"$root"/}"
    slug="$(basename "$spec_abs" .md)"
    feature="feat/$slug"
    if ! git check-ref-format --branch "$feature" >/dev/null 2>&1; then
        echo "keeler-feature-branch: $rel would need the branch $feature, which git will not accept — rename the spec file." >&2
        exit 1
    fi
    # Where main is, is `_main-ref`'s answer and no one else's.
    main_ref="$(just _main-ref)"
    main_branch="${main_ref##*/}"
    here="$(git symbolic-ref --quiet --short HEAD || true)"
    if [ "$here" != "$main_branch" ] && [ "$here" != "$feature" ]; then
        echo "keeler-feature-branch: on ${here:-a detached HEAD} — a feature branch is cut from $main_branch, and a feature nested inside another feature's branch, or inside a task's, is a shape nothing else in graph mode expects. Check out $main_branch and run this again." >&2
        exit 1
    fi
    # The working tree's copy is the one the human just approved, so it is
    # the one that must end up on the branch — whatever the branch already
    # holds. Held aside first, because the switch below puts the spec back
    # to what *this* branch committed: git refuses to carry a modified file
    # across to a branch holding a different copy of it, and an approval
    # that ends in "your local changes would be overwritten" is an approval
    # that ends nowhere.
    approved="$(mktemp)"
    branch_copy="$(mktemp)"
    trap 'rm -f "$approved" "$branch_copy"' EXIT
    cp "$spec_abs" "$approved"
    existed=no
    if git show-ref --verify --quiet "refs/heads/$feature"; then existed=yes; fi
    if [ "$here" != "$feature" ]; then
        # What HEAD has, not what the index has: a spec written and `git
        # add`ed but never committed is known to git and absent from HEAD,
        # and `git checkout HEAD -- <it>` fails on a pathspec HEAD does not
        # know. The index entry goes with it — a staged addition carried
        # across would collide with a branch that committed its own copy.
        if git cat-file -e "HEAD:$rel" 2>/dev/null; then
            git checkout -q HEAD -- "$spec_abs"
        else
            git rm -q --cached --ignore-unmatch -- "$spec_abs" >/dev/null
            rm -f "$spec_abs"
        fi
        if [ "$existed" = yes ]; then
            switch=(checkout -q "$feature")
        else
            switch=(checkout -q -b "$feature")
        fi
        if ! git "${switch[@]}"; then
            # Whatever stopped the switch, the approved spec is the one
            # thing here that exists nowhere else — put it back before
            # saying so.
            cp "$approved" "$spec_abs"
            echo "keeler-feature-branch: could not check out $feature — $rel is as it was; commit or stash the rest of the working tree and run this again." >&2
            exit 1
        fi
        cp "$approved" "$spec_abs"
    fi
    if [ "$existed" = yes ]; then
        did="checked out $feature, which already existed"
    else
        did="cut $feature from $main_branch"
    fi
    # A spec that has not changed is not a commit. Re-approving an
    # untouched spec — or running this twice — leaves the branch alone.
    if git show "$feature:$rel" > "$branch_copy" 2>/dev/null && cmp -s "$branch_copy" "$spec_abs"; then
        echo "keeler-feature-branch: $did — $rel is already the copy it holds, so nothing was committed."
        exit 0
    fi
    git add -- "$spec_abs"
    # The pathspec is the point: `git commit -- <path>` commits that path
    # from the working tree and leaves everything else, staged or not,
    # exactly as it was.
    git commit -q -m "docs($slug): the approved spec" -- "$spec_abs"
    echo "keeler-feature-branch: $did — committed $rel there."

# The script's report is the machine's, one `<id> <state> [needs...]` line
# per task, and the spawn recipe reads it as such; this is the human's view
# of the same lines, where a blocked task shows only what it is still
# waiting on — the needs whose own line is not done — rather than every
# edge it declares. A refusal (a cycle, a need naming no task) is the
# script's: it exits non-zero naming the line, and nothing here is printed.
#
# Graph mode: what a spec's Tasks section says is ready, blocked or done — `just keeler-graph specs/01-foo.md`.
keeler-graph SPEC:
    #!/usr/bin/env bash
    set -euo pipefail
    report=$(bash scripts/keeler-graph.sh "$SPEC")
    printf '%s\n' "$report" | awk '
        NF { row[++n] = $0; if ($2 == "done") done_[$1] = 1 }
        END {
            for (r = 1; r <= n; r++) {
                k = split(row[r], f, " ")
                line = f[1] " " f[2]
                if (f[2] == "blocked") {
                    sep = " (waiting on "
                    for (j = 3; j <= k; j++) if (!(f[j] in done_)) { line = line sep f[j]; sep = ", " }
                    line = line ")"
                }
                print line
            }
        }'

# One generator for the runner, called by keeler-spawn when it cuts a
# worktree and by keeler-resume before it re-runs one. A runner is
# generated code and not a record of the run: kept from an earlier
# version it carries every defect that version had, so a resume writes it
# afresh rather than executing whatever is on disk.
_write-runner SPEC TASK BRANCH WORKTREE RUNNER EXIT_FILE LOG_FILE STREAM_FILE:
    #!/usr/bin/env bash
    set -euo pipefail
    rel="$SPEC"
    task="$TASK"
    # Derived here, in the main checkout, and baked into the runner: the
    # runner executes in the task's worktree, where the parser it needs is
    # the branch's own copy rather than this one.
    root="$(git rev-parse --show-toplevel)"
    slug="$(basename "$rel" .md)"
    tid="$(printf '%s' "$task" | tr '[:upper:]' '[:lower:]')"
    branch="$BRANCH"
    worktree="$WORKTREE"
    runner="$RUNNER"
    exit_file="$EXIT_FILE"
    log_file="$LOG_FILE"
    stream_file="$STREAM_FILE"
    prompt="Implement task $task of $rel, and nothing else.

    Read $rel in full first, then .claude/keeler.md.

    Run the whole per-task pipeline for this one task, in order and without
    stopping between stages: /keeler:tdd, then /keeler:qa, then
    /keeler:review, then /keeler:mutants. The gate is 'just keeler-branch';
    it must be green before the task is done.

    You are on branch $branch, in the worktree $worktree. Commit there as
    each stage finishes: the human ran keeler-spawn for this task, and that
    was the consent for those commits. Commit nowhere else, never push, and
    never open a pull request."
    # Enough to edit, test and commit inside the worktree, and no more. Not
    # bypassPermissions: a headless agent with an unrestricted shell is not
    # a decision a recipe should make by default.
    tools='Bash(cargo:*),Bash(just:*),Bash(git:*)'
    # Bash(git:*) grants push. "Nothing is pushed" has to be a permission,
    # not a sentence in a prompt: take it back explicitly. The match is on
    # the command prefix, so this stops the ordinary `git push` and not a
    # determined `git -C … push` or a recipe that pushes — it is a guard
    # against the accident, and the guarantee that nothing reaches the
    # remote remains the human owning the push.
    blocked='Bash(git push:*)'
    cat > "$runner" <<RUNNER
    #!/usr/bin/env bash
    # Written by 'just keeler-spawn' for $branch. Re-runnable by hand:
    #     bash "$runner"
    cd "$worktree" || exit 1
    prompt=\$(cat <<'KEELER_PROMPT'
    $prompt
    KEELER_PROMPT
    )
    # Everything the session prints is teed to the log as it goes —
    # '--verbose', because without it claude -p prints nothing until its
    # final answer, and four minutes of honest work looks exactly like a
    # hang to anyone watching.
    {
        # stream-json, because the exit code cannot answer the question
        # that matters. A session that hits its limit mid-work prints its
        # apology and exits zero, tidily, having finished nothing — this
        # project met exactly that. The stream's final 'result' record is
        # written only when the turn ends, so its presence is the signal
        # and its absence is the death, however calm the exit.
        claude -p "\$prompt" --verbose --output-format stream-json \
            --permission-mode acceptEdits --allowedTools '$tools' --disallowedTools '$blocked' \
            | tee "$stream_file"
        if ! grep -q '"type":"result"' "$stream_file" 2>/dev/null; then
            # The missing record says the process stopped. It does not say
            # the work did. So ask before giving up: a run killed
            # mid-write after committing everything is finished work with
            # no verdict, and calling that dead leaves a task whose board
            # line contradicts its own branch and whose only route onward
            # redoes what is already done.
            #
            # Everything is asked of the branch, not of the worktree, so
            # the sentence below is literally true and a tree that walked
            # off to another branch cannot answer for this one. The parser
            # is the main checkout's: a task branch editing that script —
            # which this project's own tasks do — would otherwise decide
            # whether its own box is ticked.
            k_why=""
            [ "\$(git symbolic-ref --quiet --short HEAD 2>/dev/null)" = "$branch" ] \
                || k_why="the worktree is no longer on $branch"
            [ -z "\$k_why" ] && [ -n "\$(git status --porcelain 2>/dev/null)" ] \
                && k_why="the worktree has uncommitted changes"
            [ -z "\$k_why" ] && ! git cat-file -e "$branch:reviews/$slug/$tid.md" 2>/dev/null \
                && k_why="reviews/$slug/$tid.md is not committed on the branch"
            if [ -z "\$k_why" ]; then
                k_state="\$(git show "$branch:$rel" 2>/dev/null > "$stream_file.spec" \
                    && bash "$root/scripts/keeler-graph.sh" "$stream_file.spec" 2>/dev/null \
                    | awk -v t="$task" 'tolower(\$1) == tolower(t) { print \$2 }')"
                rm -f "$stream_file.spec"
                [ "\$k_state" = done ] || k_why="$task is not ticked on the branch"
            fi
            if [ -n "\$k_why" ]; then
                echo "keeler: the agent ended without finishing its turn, and \$k_why — the gate did not run, and there is no verdict to mistake for one" >&2
                exit 1
            fi
            echo "keeler: the agent was cut off after its work — the record and the tick are committed on $branch and the tree is clean, so the gate runs" >&2
        fi
        # Scoped to the result record: is_error is a field of every failed
        # tool result too, and red-then-green means every Keeler task has
        # one. Unscoped, this reported 'died' for every task that did its
        # job — a false death, which is worse than the false pass it
        # replaced, because it also invites resuming finished work.
        if grep '"type":"result"' "$stream_file" 2>/dev/null | grep -q '"is_error":true'; then
            echo "keeler: the agent finished its turn with an error — the gate did not run" >&2
            exit 1
        fi
        just keeler-branch
        echo \$? > "$exit_file"
    } 2>&1 | tee -a "$log_file"
    RUNNER
    chmod +x "$runner"


# The guards `keeler-spawn` fires on its way to a worktree, and the graph
# it then reads: written once, here, because `keeler-fan-out` must refuse
# on the same grounds before it prints a wave, and two copies of a check
# are two checks that drift. Private, the way `_main-ref` is: it speaks
# for `keeler-spawn` — its refusals carry that name, and are the same words
# whichever recipe called it. On stdout it prints the graph as the feature
# branch commits it, one `<id> <state> [needs...]` line per task, which is
# what the caller reads readiness from.
#
# It refuses when tmux is missing, when the spec is not a file inside this
# repository, when it differs from HEAD or is not Approved (the worktree is
# cut from HEAD, so an uncommitted graph is one the agent would never see),
# and when HEAD is not the feature's own branch feat/<spec-slug>.
_spawn-preflight SPEC:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v tmux >/dev/null 2>&1; then
        echo "keeler-spawn: tmux is required — every spawned agent runs in a detached tmux session." >&2
        echo "  macOS:         brew install tmux" >&2
        echo "  Debian/Ubuntu: sudo apt-get install tmux" >&2
        exit 1
    fi
    spec="$SPEC"
    [ -f "$spec" ] || { echo "keeler-spawn: $spec is not a file" >&2; exit 1; }
    root="$(git rev-parse --show-toplevel)"
    spec_abs="$(cd "$(dirname "$spec")" && pwd)/$(basename "$spec")"
    case "$spec_abs" in
        "$root"/*) ;;
        *)
            echo "keeler-spawn: $spec_abs is not inside $root — the worktree is cut from this repository's HEAD and would never see it." >&2
            exit 1
            ;;
    esac
    rel="${spec_abs#"$root"/}"
    # The spec as committed, because that is what the new worktree will see:
    # /keeler:tasks leaves the graph uncommitted, and a spawn straight after
    # would compute readiness from a file the worktree does not have.
    if [ -n "$(git status --porcelain -- "$spec_abs")" ]; then
        echo "keeler-spawn: $rel differs from HEAD — commit it first; the worktree is cut from HEAD and would not see it." >&2
        exit 1
    fi
    status_line="$(grep -m1 -E '^[*_[:space:]]*Status:' "$spec_abs" || true)"
    case "$status_line" in
        *Approved*) ;;
        *)
            echo "keeler-spawn: $rel is not Approved (${status_line:-no Status: line}) — an unapproved spec is not a contract to build from." >&2
            exit 1
            ;;
    esac
    # Readiness is the graph script's answer, never this recipe's: one
    # parser reads the format, so a human and the tools cannot disagree.
    # And it is read from the spec on the feature's own branch. A tick on a
    # *task* branch unblocks nothing — that is what keeps parallel branches
    # from racing each other's dependencies — while a tick on the feature
    # branch does, because arriving there is the landing. Which branch that
    # is, is a name a machine checks rather than one someone remembers.
    # tmux reads a dot as the separator between a window and a pane, so a
    # session named keeler-06.1-foo-t3 cannot be addressed at all — not
    # even with the exact-match `=`, which answers "can't find pane:
    # 1-foo-t3". The board would never say running, a second spawn would
    # not see the first, and nothing could ever clean the session up. Git
    # accepts the branch name happily, so this is ours to refuse.
    case "$(basename "$spec_abs" .md)" in
        *.*)
            echo "keeler-spawn: $rel — a spec whose name holds a dot cannot be spawned: tmux reads the dot in keeler-$(basename "$spec_abs" .md)-<task> as a pane separator and the session becomes unaddressable. Rename the spec." >&2
            exit 1
            ;;
    esac
    feature="feat/$(basename "$spec_abs" .md)"
    if ! git check-ref-format --branch "$feature" >/dev/null 2>&1; then
        echo "keeler-spawn: $rel would need the branch $feature, which git will not accept — rename the spec file." >&2
        exit 1
    fi
    here="$(git symbolic-ref --quiet --short HEAD || true)"
    if [ "$here" != "$feature" ]; then
        echo "keeler-spawn: on ${here:-a detached HEAD}, but this spec's tasks fan out from $feature — check it out, or create it, and spawn from there." >&2
        exit 1
    fi
    feature_copy="$(mktemp -d)"
    trap 'rm -rf "$feature_copy"' EXIT
    if ! git show "$feature:$rel" > "$feature_copy/$(basename "$spec_abs")" 2>/dev/null; then
        echo "keeler-spawn: $rel is not committed on $feature — the worktree is cut from it, so an uncommitted graph is one the agent would never see." >&2
        exit 1
    fi
    bash "$root/scripts/keeler-graph.sh" "$feature_copy/$(basename "$spec_abs")"

# It refuses before creating anything on every ground `_spawn-preflight`
# refuses — tmux missing, the spec not a file in this repository, differing
# from HEAD, not Approved, HEAD not the feature's branch — and then when
# the graph script says the task is blocked or done, and when the task
# already has a worktree or a branch. Otherwise it creates a worktree
# beside the repository on keeler/<spec-slug>/<task-id>, writes a runner
# script under .keeler/runs/, starts a detached tmux session on it and
# returns at once. `just keeler-status <spec>` is the board afterwards.
#
# Graph mode: hand one ready task to a headless agent on its own branch — `just keeler-spawn specs/01-foo.md T3`.
keeler-spawn SPEC TASK:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="$SPEC"
    task="$TASK"
    # The just that is running this recipe runs its helper — not whichever
    # `just` PATH holds, which on a PATH stripped to the shell is none, and
    # would turn "tmux is required" into "just: command not found". `-q`: a
    # refusal is the preflight's one line, not that line plus just's report
    # that a recipe nobody named failed.
    report="$("{{just_executable()}}" -q _spawn-preflight "$spec")"
    root="$(git rev-parse --show-toplevel)"
    spec_abs="$(cd "$(dirname "$spec")" && pwd)/$(basename "$spec")"
    rel="${spec_abs#"$root"/}"
    entry="$(printf '%s\n' "$report" | awk -v id="$task" '$1 == id')"
    if [ -z "$entry" ]; then
        echo "keeler-spawn: $rel defines no task $task" >&2
        exit 1
    fi
    state="$(printf '%s\n' "$entry" | awk '{ print $2 }')"
    # A done task has landed. Spawning it cuts a branch to redo work the
    # graph already counts — and the guard that refused only `blocked`
    # let exactly that through.
    if [ "$state" = done ]; then
        echo "keeler-spawn: $task is already done — nothing to spawn" >&2
        exit 1
    fi
    if [ "$state" = blocked ]; then
        unmet=""
        for need in $(printf '%s\n' "$entry" | cut -d' ' -f3-); do
            if [ "$(printf '%s\n' "$report" | awk -v n="$need" '$1 == n { print $2 }')" != done ]; then
                unmet="$unmet $need"
            fi
        done
        echo "keeler-spawn: $task is blocked, waiting on:$unmet" >&2
        exit 1
    fi
    # One name in four places: the spec's file name, and the id lowercased
    # once, on the way out of the parser.
    slug="$(basename "$spec_abs" .md)"
    tid="$(printf '%s' "$task" | tr '[:upper:]' '[:lower:]')"
    branch="keeler/$slug/$tid"
    session="keeler-$slug-$tid"
    worktree="$(dirname "$root")/$(basename "$root")-$slug-$tid"
    if [ -e "$worktree" ]; then
        echo "keeler-spawn: $worktree already exists — $task is already spawned; land it or remove it first." >&2
        exit 1
    fi
    # A branch whose worktree is gone is the same task, half cleaned up:
    # the commits are still on it. Refuse here, naming it, rather than
    # let `git worktree add` fail somewhere deeper.
    if git show-ref --verify --quiet "refs/heads/$branch"; then
        echo "keeler-spawn: branch $branch already exists — $task is already spawned; land it, or delete the branch to start over." >&2
        exit 1
    fi
    # Nothing on disk until the branch and the worktree are real: a path
    # that refuses must create nothing — and destroy nothing either.
    git worktree add -b "$branch" "$worktree" HEAD
    runs="$root/.keeler/runs/$slug"
    mkdir -p "$runs"
    exit_file="$runs/$tid.exit"
    stream_file="$runs/$tid.stream"
    log_file="$runs/$tid.log"
    runner="$runs/$tid.sh"
    # A verdict left by an earlier run of this task is not this run's.
    rm -f "$exit_file"
    just _write-runner "$rel" "$task" "$branch" "$worktree" "$runner" "$exit_file" "$log_file" "$stream_file"
    printf -v run_cmd 'bash %q' "$runner"
    # If the session will not start, the branch and worktree must not
    # survive it: every retry would then refuse with "already spawned" for
    # a run that never began, and the task would be wedged.
    if ! tmux new-session -d -s "$session" -c "$worktree" "$run_cmd"; then
        echo "keeler-spawn: tmux would not start $session — undoing" >&2
        git worktree remove --force "$worktree" 2>/dev/null || true
        git branch -D "$branch" >/dev/null 2>&1 || true
        rm -f "$runner"
        exit 1
    fi
    echo "spawned $task on $branch"
    echo "  worktree: $worktree"
    echo "  session:  tmux attach -t $session   (a view, not a seat: claude -p is not interactive)"
    echo "  log:      $log_file"
    echo "  verdict:  $exit_file   (the exit code of just keeler-branch, once the run ends)"
    echo "  board:    just keeler-status $rel"

# "Running" is tmux's answer, never the absence of a file; a task with no
# verdict at all died before its gate ever ran, which is a different thing
# from a gate that failed, and its log and worktree are what a resume reads.
#
# Graph mode: what every task of a spec is doing right now — running, passed, incomplete, failed, died, done, or never spawned.
keeler-status SPEC:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="$SPEC"
    [ -f "$spec" ] || { echo "keeler-status: $spec is not a file" >&2; exit 1; }
    root="$(git rev-parse --show-toplevel)"
    spec_abs="$(cd "$(dirname "$spec")" && pwd)/$(basename "$spec")"
    slug="$(basename "$spec_abs" .md)"
    runs="$root/.keeler/runs/$slug"
    # The graph comes from the feature's branch, the same place
    # keeler-spawn reads it: a board that answered from the working tree
    # would be the one place in graph mode where an uncommitted tick
    # counts, and it would report a task done that spawn does not believe.
    rel="${spec_abs#"$root/"}"
    feature="feat/$slug"
    # After a feature lands, its branch is gone and the board must still
    # answer — so HEAD stands in, and the run says which ref it read
    # rather than leaving the reader to assume.
    graph_ref="$feature"
    git rev-parse --verify -q "$feature^{commit}" >/dev/null || graph_ref=HEAD
    echo "graph: $rel on $graph_ref"
    graph_copy="$(mktemp -d)"
    trap 'rm -rf "$graph_copy"' EXIT
    if ! git show "$graph_ref:$rel" > "$graph_copy/$(basename "$spec_abs")" 2>/dev/null; then
        echo "keeler-status: $rel is not committed on $graph_ref — there is no graph to report against." >&2
        exit 1
    fi
    report="$(bash "$root/scripts/keeler-graph.sh" "$graph_copy/$(basename "$spec_abs")")"
    while read -r id graph_state _rest; do
        [ -n "$id" ] || continue
        tid="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
        session="keeler-$slug-$tid"
        worktree="$(dirname "$root")/$(basename "$root")-$slug-$tid"
        exit_file="$runs/$tid.exit"
        log_file="$runs/$tid.log"
        branch="keeler/$slug/$tid"
        record="reviews/$slug/$tid.md"
        # Closed is three things, and two of them live on the task's own
        # branch until someone merges it. Reading them from here would
        # make `incomplete` mean "not merged yet" — which every task says
        # before a merge, finished or not — and the guard would be absent
        # at the one moment it is for: deciding whether to merge. So the
        # branch is asked while it exists, and this tree once it is gone.
        landed=no
        if [ "$graph_state" = done ] && [ -f "$root/$record" ]; then
            # Landed and closed: this tree counts the tick and carries the
            # record. A task branch left standing afterwards says nothing —
            # the work is here.
            landed=yes
        fi
        if [ "$landed" = no ] && git rev-parse --verify -q "$branch^{commit}" >/dev/null 2>&1; then
            has_record=no
            git cat-file -e "$branch:$record" 2>/dev/null && has_record=yes
            ticked=no
            branch_copy="$(mktemp -d)"
            if git show "$branch:$rel" > "$branch_copy/$(basename "$spec_abs")" 2>/dev/null; then
                branch_state="$(bash "$root/scripts/keeler-graph.sh" "$branch_copy/$(basename "$spec_abs")" 2>/dev/null | awk -v id="$id" '$1 == id { print $2 }')"
                [ "$branch_state" = done ] && ticked=yes
            fi
            rm -rf "$branch_copy"
        else
            has_record=no
            [ -f "$root/$record" ] && has_record=yes
            ticked=no
            [ "$graph_state" = done ] && ticked=yes
        fi
        missing=""
        [ "$has_record" = yes ] || missing="no review record"
        if [ "$ticked" = no ]; then
            [ -n "$missing" ] && missing="$missing, "
            missing="${missing}box not ticked"
        fi
        # The `=` is exact matching: without it tmux answers about t10 when
        # asked about t1.
        if command -v tmux >/dev/null 2>&1 && tmux has-session -t "=$session" 2>/dev/null; then
            state=running
        elif [ "$landed" = yes ]; then
            # Landed and closed: the graph on this branch counts it, and it
            # carries its record. This answers before any leftover does — a
            # stale verdict from a run that was later fixed and landed is
            # the record of something superseded.
            printf '%-6s %s\n' "$id" "done"
            continue
        elif [ -f "$exit_file" ] && [ "$(tr -d '[:space:]' < "$exit_file")" != 0 ]; then
            state="failed (exit $(tr -d '[:space:]' < "$exit_file"))"
        elif [ ! -f "$exit_file" ] && { [ -e "$worktree" ] || [ -f "$log_file" ]; }; then
            state=died
        elif [ ! -f "$exit_file" ] && [ "$graph_state" != done ] && [ ! -e "$worktree" ]; then
            # Nothing was ever started here, so there is nothing to be
            # incomplete about.
            printf '%-6s %s\n' "$id" "not spawned"
            continue
        elif [ -n "$missing" ]; then
            # A green gate is one stage of four: a task can pass it having
            # done only tdd, and calling that closed invites landing work
            # nobody has read. The tick is the cheapest of the three to
            # produce, so it is never the one that answers alone.
            state="incomplete ($missing)"
        else
            state=passed
        fi
        printf '%-6s %-16s log %s  worktree %s\n' "$id" "$state" "$log_file" "$worktree"
        # A death is ordinary rather than exceptional — five of this
        # project's first six spawns ended that way — so the board offers
        # the way back rather than leaving it to be remembered.
        if [ "$state" = died ]; then
            printf '       resume with: just keeler-resume %s %s\n' "$rel" "$id"
        fi
        case "$state" in
            failed*)
                # A verdict is a run's record, and a run can turn out not
                # to be believable — an earlier tooling, a gate that
                # measured a tree the agent never touched. Nothing here
                # takes it back: that is the human's judgement. But the
                # way out should not be a path the tool never mentions,
                # because without it the task is locked — the board says
                # failed and the resume refuses, for ever.
                printf '       verdict from an earlier run? rm %s to make it resumable\n' "$exit_file"
                ;;
        esac
    done <<< "$report"

# A spawned session that ended before its pipeline finished left its
# worktree, its branch and its commits exactly where they were — the
# runner it was started from is re-runnable, and this gives that a name.
# Nothing new is created: the task picks up in the tree it already has,
# and the commits already on the branch are how far it got.
#
# Graph mode: re-run a task whose session died — `just keeler-resume specs/01-foo.md T3`.
keeler-resume SPEC TASK:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="$SPEC"
    task="$TASK"
    command -v tmux >/dev/null 2>&1 || {
        echo "keeler-resume: tmux is not installed — a resume runs where a spawn ran, in a detached session. brew install tmux, or apt install tmux." >&2
        exit 1
    }
    [ -f "$spec" ] || { echo "keeler-resume: $spec is not a file" >&2; exit 1; }
    root="$(git rev-parse --show-toplevel)"
    spec_abs="$(cd "$(dirname "$spec")" && pwd)/$(basename "$spec")"
    slug="$(basename "$spec_abs" .md)"
    tid="$(printf '%s' "$task" | tr '[:upper:]' '[:lower:]')"
    runner="$root/.keeler/runs/$slug/$tid.sh"
    session="keeler-$slug-$tid"
    branch="keeler/$slug/$tid"
    worktree="$(dirname "$root")/$(basename "$root")-$slug-$tid"
    if [ ! -f "$runner" ]; then
        echo "keeler-resume: $task was never spawned — there is no run to resume; use just keeler-spawn $spec $task" >&2
        exit 1
    fi
    if tmux has-session -t "=$session" 2>/dev/null; then
        echo "keeler-resume: $task is still running — attach with tmux attach -t '=$session', or let it finish" >&2
        exit 1
    fi
    if [ -f "$root/.keeler/runs/$slug/$tid.exit" ]; then
        echo "keeler-resume: $task already reached its gate — its verdict stands; resume is for a session that ended before it" >&2
        exit 1
    fi
    # The graph decides what is done, here as everywhere: a task that
    # landed is not resumable, and keeler-land leaves the runner behind
    # whenever its cleanup was skipped.
    state="$(bash "$root/scripts/keeler-graph.sh" "$spec_abs" | awk -v id="$task" '$1 == id { print $2 }')"
    if [ "$state" = done ]; then
        echo "keeler-resume: $task is done — there is nothing to resume; its work landed already" >&2
        exit 1
    fi
    # Without the worktree the runner's own `cd` fails inside tmux, where
    # nobody reads it: resume would report success having run nothing, and
    # the task wedges — spawn refuses the branch, resume no-ops forever.
    if [ ! -d "$worktree" ]; then
        echo "keeler-resume: $worktree is gone — the run has no tree to resume in; remove the branch keeler/$slug/$tid and spawn again" >&2
        exit 1
    fi
    echo "keeler-resume: re-running $task in the worktree and branch it already has"
    # Written afresh: a runner is generated code, and one kept from an
    # earlier recipe carries every defect that recipe had. The worktree,
    # the branch and the log are what a resume keeps.
    rel="${spec_abs#"$root/"}"
    just _write-runner "$rel" "$task" "$branch" "$worktree" "$runner" \
        "$root/.keeler/runs/$slug/$tid.exit" \
        "$root/.keeler/runs/$slug/$tid.log" \
        "$root/.keeler/runs/$slug/$tid.stream"
    printf -v run_cmd 'bash %q' "$runner"
    tmux new-session -d -s "$session" -c "$worktree" "$run_cmd"
    echo "  worktree: $worktree"
    echo "  session:  tmux attach -t '=$session'"
    echo "  board:    just keeler-status $spec"
# The wave is what is ready and not yet spawned: `keeler-graph`'s ready
# tasks minus the ones `keeler-status` already knows — running, died,
# passed or failed but not landed — and this recipe computes neither. It
# owns no logic those two have; it reads them and prints one line per task
# in the spec's order: the graph's own line for a task that is done, ready
# or blocked (with what it waits on), the board's own line for a ready task
# already spawned. Then it names the wave and asks. `_spawn-preflight`
# runs first, so a checkout keeler-spawn would refuse is refused here in
# the same words, before any wave is printed.
#
# The yes is read from stdin — a human at a terminal is asked in the
# ordinary way, and a test pipes its answer — and it is `yes` or `y`, case
# aside, and nothing else. When there is nobody to ask, the recipe refuses
# and names KEELER_FAN_OUT_YES rather than hanging or exiting in silence:
# `read` at EOF under `set -e` would end the recipe with a bare 1, and a
# pipe nobody closes would hang it, so a stdin that is not a terminal gets
# a bounded read — an answer piped in advance is there at once; nothing
# within the wait is nobody. `KEELER_FAN_OUT_YES=1` answers yes without
# asking, for the caller who has already decided. It is the zero-yes path
# this recipe otherwise refuses, so no command file may set it, and a
# test says so.
#
# On the yes, the wave is handed to `keeler-spawn` one task at a time, in
# the order it was printed in — the same recipe a hand would run, so every
# refusal it has fires per task and nothing here cuts a worktree of its
# own. A refusal is that task's alone: the board was read a moment before,
# and a task another hand has spawned since is named and stepped over
# while the rest of the wave spawns. The run reports what spawned and what
# did not, and exits non-zero when anything was refused — one wave, one
# yes, one exit code that says whether it all went out.
#
# Then the wave is shown as one thing: a detached session
# keeler-<spec-slug>-wave holding a pane per run, each attached to that
# run's own session, tiled as they are added. The per-task sessions are
# the source of truth and outlive the view — the board reads them,
# `tmux attach` on one still works, closing the view kills nothing — so a
# view that cannot be built is reported and leaves the exit code alone.
#
# Graph mode: name every ready, unspawned task and ask for the one yes that spawns the wave — `just keeler-fan-out specs/01-foo.md`.
keeler-fan-out SPEC:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="$SPEC"
    # The same guards keeler-spawn fires, before a wave is printed — run
    # by the just that is running this recipe, as keeler-spawn runs them.
    # The preflight's report is discarded: once it passes, the working
    # tree's spec is the feature branch's, and `keeler-graph` reads that
    # same graph in the words this recipe prints — a blocked task with what
    # it waits on.
    just="{{just_executable()}}"
    "$just" -q _spawn-preflight "$spec" >/dev/null
    # One name for the wave's own things, derived the way every other name
    # in graph mode is: the spec's file name, which the preflight has just
    # established is a file inside this repository.
    root="$(git rev-parse --show-toplevel)"
    slug="$(basename "$spec" .md)"
    graph="$("$just" keeler-graph "$spec")"
    board="$("$just" keeler-status "$spec")"
    echo "keeler-fan-out: $spec on $(git symbolic-ref --quiet --short HEAD)"
    wave=""
    while read -r id state rest; do
        [ -n "$id" ] || continue
        if [ "$state" != ready ]; then
            printf '  %s %s%s\n' "$id" "$state" "${rest:+ $rest}"
            continue
        fi
        # A ready task the board already knows is listed in the board's
        # words and not offered; one it calls "not spawned" is the wave.
        known="$(printf '%s\n' "$board" | awk -v id="$id" '$1 == id')"
        case "$known" in
            *"not spawned"*)
                printf '  %s ready\n' "$id"
                wave="$wave${wave:+ }$id"
                ;;
            *)
                printf '  %s\n' "${known:-$id is not on the board}"
                ;;
        esac
    done <<< "$graph"
    if [ -z "$wave" ]; then
        echo "keeler-fan-out: nothing is ready to spawn — every task above is done, blocked, or already spawned and in the state the board gives it."
        exit 0
    fi
    echo "wave: $wave"
    if [ -n "${KEELER_FAN_OUT_YES:-}" ]; then
        answer="$KEELER_FAN_OUT_YES"
        echo "spawn $wave? [yes/no] $answer (from KEELER_FAN_OUT_YES)"
        # The variable is the answer given in advance, and it is read as
        # one: 1 is the documented yes; the words the prompt takes are too;
        # anything else is not.
        case "$answer" in 1) answer=yes ;; esac
    elif [ -t 0 ]; then
        printf 'spawn %s? [yes/no] ' "$wave"
        IFS= read -r answer || answer=""
    else
        printf 'spawn %s? [yes/no] ' "$wave"
        # Not a terminal: an answer piped in advance is read at once, and
        # nothing within the wait — EOF, or a pipe nobody writes to — is
        # nobody to ask.
        if ! IFS= read -r -t 5 answer; then
            echo
            echo "keeler-fan-out: nobody to ask — stdin is not a terminal and KEELER_FAN_OUT_YES is unset. Ask from a terminal, or answer in advance: KEELER_FAN_OUT_YES=1 just keeler-fan-out $spec" >&2
            exit 1
        fi
        echo "$answer"
    fi
    case "$(printf '%s' "$answer" | tr '[:upper:]' '[:lower:]')" in
        yes|y) ;;
        *)
            echo "keeler-fan-out: not a yes — nothing spawned." >&2
            exit 1
            ;;
    esac
    # The yes is given. What it spawns is a loop over `keeler-spawn`, one
    # task at a time, in the order the wave was printed in — the same recipe
    # a hand would run, so every refusal it has still fires per task and
    # nothing here cuts a worktree of its own.
    # The answer is the human's, and it must not travel: a spawned agent
    # that inherited it could run a wave with the one question already
    # answered. So it is unset before anything is spawned.
    unset KEELER_FAN_OUT_YES
    echo "keeler-fan-out: yes to $wave"
    spawned=""
    refused=""
    for id in $wave; do
        echo
        # A refusal is one task's, not the wave's: the graph moved under a
        # board read a moment ago — another hand spawned it, a branch is
        # still standing — and the tasks after it are unaffected. So the
        # loop records the outcome and goes on, and the exit code at the end
        # is what says a spawn refused.
        if "$just" keeler-spawn "$spec" "$id"; then
            spawned="$spawned${spawned:+ }$id"
        else
            refused="$refused${refused:+ }$id"
        fi
    done
    # The view, built after the spawns and out of the sessions they made:
    # one window, one pane per run, so the wave is watched together instead
    # of one attach at a time. The per-task sessions are the real thing and
    # outlive it — closing the view kills nothing, and a tmux that will not
    # build it is said so and does not touch the exit code below, which
    # answers about the wave and not about the window over it.
    view=""
    shown=""
    if [ -n "$spawned" ]; then
        view="keeler-$slug-wave"
        runs="$root/.keeler/runs/$slug"
        pane_runner="$runs/wave.sh"
        # A runner, the way spawn writes its own, and for one more reason:
        # the pane's command is run by the user's shell, and in zsh a word
        # beginning with `=` is a command to look up rather than a tmux
        # target. Inside a script it is neither.
        write_pane_runner() {
            mkdir -p "$runs" || return 1
            cat > "$pane_runner" <<'WAVE' || return 1
    #!/usr/bin/env bash
    # Written by 'just keeler-fan-out' — one pane of the wave view:
    #     bash wave.sh keeler-<spec-slug>-<task>
    # The target is exact: tmux matches a bare name as a prefix, and
    # keeler-<spec-slug> is a prefix of every task's session. TMUX= because
    # this attach is nested by construction — the pane is already in tmux.
    exec env TMUX= tmux attach-session -t "=$1"
    WAVE
            chmod +x "$pane_runner"
        }
        # Tested and not left to set -e, all of it: the spawns have already
        # gone out, and a disk that will not take a runner must not take the
        # wave's report down with it.
        if write_pane_runner; then
            anchor=""
            if ! tmux has-session -t "=$view" 2>/dev/null; then
                # The window is built in a pane of its own — killed once the
                # runs have theirs — because it has to outlive the building,
                # and a pane holding a run cannot promise that: a run that
                # ends takes its pane, and the last pane takes the session.
                # A run that ended in the second it took to lay the wave out
                # would otherwise leave the tasks after it unseen. -x/-y
                # large enough to split the whole wave into: an 80x24 session
                # has no room left at the fourth pane. A view left standing
                # by an earlier wave is added to rather than remade — its
                # panes are runs that are still going.
                anchor="$(tmux new-session -d -P -F '#{pane_id}' -s "$view" -x 200 -y 50 2>/dev/null || true)"
            fi
            for id in $spawned; do
                tid="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
                printf -v pane_cmd 'bash %q %q' "$pane_runner" "keeler-$slug-$tid"
                # `=name:` and not `=name`: the exact prefix is read off the
                # session part of a target, and a bare name given where a
                # pane is expected is looked up as a pane and not found. The
                # trailing colon is the session's current window, which is
                # the wave's one window.
                tmux split-window -t "=$view:" "$pane_cmd" || break
                # Tiled as each pane arrives, not once at the end: without it
                # the fourth split has nowhere to go and fails.
                tmux select-layout -t "=$view:" tiled >/dev/null || true
                shown="$shown${shown:+ }$id"
            done
            if [ -n "$anchor" ] && [ -n "$shown" ]; then
                tmux kill-pane -t "$anchor" >/dev/null 2>&1 || true
                tmux select-layout -t "=$view:" tiled >/dev/null || true
            elif [ -n "$anchor" ]; then
                # A window over nothing is not a view of anything.
                tmux kill-session -t "=$view" >/dev/null 2>&1 || true
            fi
        fi
        if [ "$shown" != "$spawned" ]; then
            # The window is short of the wave — and the wave is unaffected,
            # so this is said and not counted: every run is in its own
            # session whether or not anything is watching it.
            echo "keeler-fan-out: the view holds ${shown:-nothing} of the wave — tmux would go no further. The runs are in their own sessions regardless: tmux attach -t '=keeler-$slug-<task>'." >&2
            [ -n "$shown" ] || view=""
        fi
    fi
    echo
    echo "keeler-fan-out: spawned ${spawned:-nothing}"
    if [ -n "$view" ]; then
        # Inside tmux an attach fails at once, and the move that does what
        # the human meant is switch-client.
        if [ -n "${TMUX:-}" ]; then
            echo "  wave:     tmux switch-client -t '=$view'   (already inside tmux, where an attach would refuse)"
        else
            echo "  wave:     tmux attach -t '=$view'   (one pane per run; closing it kills nothing)"
        fi
    fi
    echo "  board:    just keeler-status $spec"
    if [ -n "$refused" ]; then
        # What went out is the line above, and it says `nothing` when the
        # whole wave refused: this one names what did not, and claims
        # nothing about the rest that would be untrue then.
        echo "keeler-fan-out: refused $refused — keeler-spawn said why above." >&2
        exit 1
    fi

# Upgrade Keeler itself (KEELER_REF=v0.4.0 just keeler-upgrade to pin a tag)
keeler-upgrade:
    curl -fsSL https://raw.githubusercontent.com/minikin/keeler/main/install.sh | bash -s .

# Landing happens twice, and the branch says which one this is. On the
# feature branch feat/<spec-slug>: `just dev`, then each landed task's
# clean worktree and branch are removed — that is where tasks fan out
# from and where their leftovers belong. On main: `just dev`, then the
# baseline is regenerated and staged, and a spec whose every box is
# ticked — as committed, never as the working tree happens to read — gets
# `Status: Implemented` staged beside it for the same human commit. The
# baseline is the whole team's reference and moves in one visible place;
# Implemented is what main says about a feature that has arrived. Anywhere
# else it refuses by name.
#
# The gate runs first at both levels and nothing moves if it is red: a
# baseline recorded from a red main writes "broken" down as the new
# normal, and two branches that were each green alone can be wrong
# together. Not `dev-full` — hours of mutants at every fan-in is a cost
# nobody would pay.
#
# Cleanup asks for a closed task, not a ticked one: the review record and
# a green gate beside the box, because a tick is the cheapest of the three
# to produce and a worktree may hold the only copy of unread work.
#
# Cleanup only when there is nothing else to lose: a worktree with uncommitted
# changes, one that has moved to another branch, and a branch holding
# commits the feature branch does not have are each named and left where
# they are. Nothing here is committed, and nothing a human has not seen is
# thrown away.
#
# Graph mode: fan-in — worktrees on the feature branch, baseline and Status: on main; staged, never committed.
keeler-land:
    #!/usr/bin/env bash
    set -euo pipefail
    # The gates run as nested `just` calls and not as recipe dependencies:
    # a dependency runs before this body, and so before the refusal below —
    # a branch would have paid for the whole gate suite to be told it was
    # on the wrong branch.
    main_ref="$(just _main-ref)"
    main_branch="${main_ref##*/}"
    current="$(git symbolic-ref --quiet --short HEAD || echo 'a detached HEAD')"
    # Landing happens twice, and the branch says which one this is. A task
    # lands into the feature branch, feat/<spec-slug>: its box is ticked
    # there and its worktree goes. The feature lands into main: that is
    # where Status: becomes Implemented and where the baseline moves — the
    # baseline is the whole team's reference and must move in one visible
    # place, not in every feature's branch. Anywhere else is neither.
    level=""
    case "$current" in
        "$main_branch") level=main ;;
        feat/*)         level=feature ;;
        *)
            echo "keeler-land: on $current — a task lands on its feature branch feat/<spec-slug>, a feature lands on $main_branch, and there is nothing to land anywhere else." >&2
            exit 1
            ;;
    esac
    if ! just dev; then
        echo "keeler-land: $current is red after fan-in — branches that were green alone are wrong together. Nothing is staged and nothing is removed; fix or revert, then land again." >&2
        exit 1
    fi
    if [ "$level" = main ]; then
        just crap-baseline
        # Staged, never committed: the rules say no commit without the
        # human's word, and a moved baseline is a decision worth a diff
        # someone reads.
        if [ -e crap-baseline.json ]; then
            git add -- crap-baseline.json
            echo "keeler-land: crap-baseline.json is staged, not committed — review the diff and commit it yourself:"
            echo "    git diff --cached -- crap-baseline.json"
        else
            echo "keeler-land: no baseline was produced — nothing in this project to measure yet, so nothing is staged."
        fi
    fi
    # Fan-in, part two: a spec whose every box is ticked is finished, and a
    # landed task's worktree is litter. Readiness is the graph script's
    # answer here as it is everywhere else — one parser reads the format,
    # so this recipe and `just keeler-graph` cannot disagree about which
    # tasks are done.
    root="$(git rev-parse --show-toplevel)"
    for spec in "$root"/specs/*.md; do
        [ -f "$spec" ] || continue
        rel="${spec#"$root"/}"
        # Ticks that are not committed are not landed. `keeler-spawn`
        # refuses the mirror of this and for the same reason: readiness is
        # what the repository records, not what a file happens to say right
        # now. It is also what keeps the staged diff honest — a spec with
        # unrelated edits would have them swept into the commit the human
        # is asked to review as the baseline and the Status: line.
        if [ -n "$(git status --porcelain -- "$spec")" ]; then
            echo "keeler-land: $rel differs from HEAD — nothing here reads uncommitted ticks; commit it and land again." >&2
            continue
        fi
        # A spec the parser refuses is one nothing here may act on: not
        # marked, not cleaned up after. Say so and leave it alone.
        if ! report="$(bash "$root/scripts/keeler-graph.sh" "$spec" 2>&1)"; then
            echo "keeler-land: $rel does not parse as a graph, so it is left exactly as it is:" >&2
            printf '%s\n' "$report" >&2
            continue
        fi
        slug="$(basename "$spec" .md)"
        # Every box ticked, and only that, is a finished spec — a spec with
        # no tasks at all has finished nothing.
        all_done="$(printf '%s\n' "$report" | awk '
            NF { n++; if ($2 == "done") d++ }
            END { print (n > 0 && n == d) ? "yes" : "no" }')"
        # The Status: line's value, not the line: `TEMPLATE.md` carries the
        # menu `Draft | Approved | Implemented`, and a substring test reads
        # that as an approval and rewrites the menu.
        status="$(grep -m1 -E '^[*_[:space:]]*Status:' "$spec" \
            | sed -E 's/^[*_[:space:]]*Status:[*_[:space:]]*//; s/[[:space:]]*$//' || true)"
        # Implemented follows Approved: a Draft nobody approved is not a
        # contract that can have been fulfilled, however many boxes are
        # ticked — and a spec already Implemented needs no second write.
        if [ "$level" = feature ] && [ "$all_done" = yes ] && [ "$status" = Approved ] && [ "$current" = "feat/$slug" ]; then
            echo "keeler-land: every task in $rel is ticked — the feature is finished here; land it on $main_branch to mark it Implemented and move the baseline."
        fi
        if [ "$level" = main ] && [ "$all_done" = yes ] && [ "$status" = Approved ]; then
            # Onto a copy and then a move, never a truncate-and-write: a
            # land stopped mid-write would otherwise leave the spec empty.
            # `cp -p` first, so the copy carries the spec's own permissions
            # rather than a temporary file's.
            marked="$spec.keeler-land"
            cp -p "$spec" "$marked"
            awk '!written && /^[*_[:space:]]*Status:/ { sub(/Approved/, "Implemented"); written = 1 } { print }' \
                "$spec" > "$marked"
            mv -f "$marked" "$spec"
            git add -- "$spec"
            echo "keeler-land: every task in $rel is ticked — its Status: is now Implemented, staged alongside the baseline for the same commit."
        fi
        # A landed task's worktree and branch have done their job — but
        # only when the worktree is clean. Uncommitted work is the human's
        # to look at before anything removes it.
        # Worktrees fan out from the feature branch, and are its to remove;
        # on main there is nothing of the kind to clean up, and a leftover
        # is the feature branch's business. And only its *own* — keeler-spawn
        # binds one spec to one feat/<slug> by name, and the land side must
        # honour the same binding or a feature branch would clean up after
        # tasks it never fanned out.
        if [ "$level" = main ]; then
            # Name what is left standing rather than pass it in silence: a
            # task worktree still here on main is the feature branch's to
            # remove, and the human should know it exists.
            for wt in "$(dirname "$root")/$(basename "$root")-$slug-"*; do
                [ -e "$wt" ] && echo "keeler-land: $wt is a task worktree — land on feat/$slug to remove it"
            done
            continue
        fi
        [ "$current" = "feat/$slug" ] || continue
        while read -r id state _rest; do
            [ "$state" = done ] || continue
            tid="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
            branch="keeler/$slug/$tid"
            worktree="$(dirname "$root")/$(basename "$root")-$slug-$tid"
            # Only a worktree this repository registered: a directory that
            # merely has the name is not ours to delete. No `grep -q`: it
            # closes the pipe on its first match, and under `pipefail` a
            # producer killed by that SIGPIPE makes the whole pipeline
            # non-zero — which here would silently skip a worktree that is
            # registered. Draining grep's input costs nothing.
            registered="$(git worktree list --porcelain)"
            printf '%s\n' "$registered" | grep -xF "worktree $worktree" > /dev/null || continue
            # Closed is three things, and the tick is only one of them. A
            # worktree may hold the only copy of work nobody has read, and
            # a box is cheap to tick — so the other two are asked for
            # before anything is removed. Below the registered filter, so
            # a task with no worktree is not told its worktree was left.
            if [ ! -f "$root/reviews/$slug/$tid.md" ]; then
                echo "keeler-land: $id is ticked but carries no review record at reviews/$slug/$tid.md — left in place, with its worktree; a tick without a record is not a landing"
                continue
            fi
            verdict="$root/.keeler/runs/$slug/$tid.exit"
            if [ -f "$verdict" ] && [ "$(tr -d '[:space:]' < "$verdict")" != 0 ]; then
                echo "keeler-land: $id is ticked but its gate failed (exit $(tr -d '[:space:]' < "$verdict")) — left in place, with its worktree; the tree is where the failure can still be read"
                continue
            fi
            # And it must still be the task's worktree: a human who
            # switched it to a branch of their own left the task's branch
            # standing somewhere else, and deleting it here would be
            # deleting something this recipe never looked at.
            on="$(git -C "$worktree" symbolic-ref --quiet --short HEAD || echo 'a detached HEAD')"
            if [ "$on" != "$branch" ]; then
                echo "keeler-land: $worktree is on $on, not $branch — left in place; it is not the task's worktree any more."
                continue
            fi
            if [ -n "$(git -C "$worktree" status --porcelain)" ]; then
                echo "keeler-land: $worktree has uncommitted changes — left in place, with $branch; look at it before removing them."
                continue
            fi
            # Committed is not merged. A branch holding commits main does
            # not have is work nobody has looked at, and deleting the
            # branch would leave it reachable only through the reflog. A
            # squash merge looks exactly like this from here, so the
            # refusal comes with the command that finishes the job.
            if ! git merge-base --is-ancestor "$branch" HEAD; then
                echo "keeler-land: $branch has commits that are not on $current — left in place, with $worktree. If they landed as a squash merge, finish it yourself:"
                echo "    git worktree remove $worktree && git branch -D $branch"
                continue
            fi
            # Tidying up is the last thing this recipe does and the least
            # of it: the baseline and the spec are already staged, so a
            # worktree that will not go — locked, on an unmounted disk —
            # is named and stepped over, not a raw git error that ends the
            # run over work already done.
            if ! git worktree remove "$worktree"; then
                echo "keeler-land: $worktree could not be removed — left in place, with $branch." >&2
                continue
            fi
            # `-d`, not `-D`: the branch is contained in HEAD by the check
            # above, so the safe delete is the one that can succeed — and
            # nothing here can quietly destroy a commit.
            if git branch -d "$branch" >/dev/null 2>&1; then
                echo "keeler-land: removed the landed worktree $worktree and its branch $branch"
            else
                echo "keeler-land: removed the landed worktree $worktree; its branch $branch is still here, delete it yourself"
            fi
            # The verdict and the runner belong to the run that has now
            # landed: `keeler-status` reads the verdict before the graph,
            # so a stale one has the board reporting "passed" beside the
            # path of a worktree this very land removed. The log stays —
            # it is the only record of what the run said.
            rm -f "$root/.keeler/runs/$slug/$tid.exit" "$root/.keeler/runs/$slug/$tid.sh" \
                  "$root/.keeler/runs/$slug/$tid.stream"
        done <<< "$report"
    done
