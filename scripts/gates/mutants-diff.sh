#!/usr/bin/env bash
# Mutation tests on the changed lines of every workspace member.
#
#   mutants-diff.sh               uncommitted changes, else the branch base,
#                                 else the last commit (`just mutants-diff`)
#   mutants-diff.sh --base <ref>  the changes since the merge base of <ref>
#                                 and HEAD, and nothing else (CI)
#
# Runs in the caller's directory, which is the repository it measures.
set -euo pipefail

usage() {
    echo "usage: mutants-diff.sh [--base <ref>]" >&2
    exit 2
}
base_ref=""
case "$#:${1:-}" in
    0:) ;;
    2:--base) base_ref="$2" ;;
    *) usage ;;
esac
if [ "$#" -eq 2 ] && [ -z "$base_ref" ]; then usage; fi

# `:(glob)` keeps `*` inside one path component: a plain pathspec `*` also
# crosses `/`, so `crates/*/src/*.rs` would match a member's
# `tests/fixtures/src/`.
paths=(':(glob)src/**/*.rs' ':(glob)crates/*/src/**/*.rs')

diff_file=$(mktemp)
index_copy="$diff_file.index"
listing="$diff_file.untracked"
trap 'rm -f "$diff_file" "$index_copy" "$listing"' EXIT

# -M on every diff: with diff.renames = false in a user's git config, a
# moved file shows as deleted plus added, and every line of it is mutated.
changes() {
    git diff -M "$@" -- "${paths[@]}" > "$diff_file" || {
        echo "git diff $* failed: nothing was measured" >&2
        exit 1
    }
}

if [ -n "$base_ref" ]; then
    merge_base=$(git merge-base "$base_ref" HEAD) || {
        echo "no merge base of $base_ref and HEAD: nothing to measure against" >&2
        exit 1
    }
    changes "$merge_base" HEAD
else
    # New files aren't in `git diff HEAD`: intent-to-add makes their full
    # content show up in the diff. It goes into a copy of the index, so the
    # caller's staging (a staged deletion of a file recreated on disk, say)
    # is never touched, even when the run is interrupted.
    # NUL-delimited into an array, so paths with spaces stay whole.
    real_index="$(git rev-parse --git-path index)"
    if [ -f "$real_index" ]; then cp "$real_index" "$index_copy"; fi
    GIT_INDEX_FILE="$index_copy" git ls-files -z --others --exclude-standard -- "${paths[@]}" > "$listing" || {
        echo "git ls-files failed: nothing was measured" >&2
        exit 1
    }
    untracked=()
    while IFS= read -r -d '' f; do untracked+=("$f"); done < "$listing"
    if [ "${#untracked[@]}" -gt 0 ]; then GIT_INDEX_FILE="$index_copy" git add -N -- "${untracked[@]}"; fi
    GIT_INDEX_FILE="$index_copy" changes HEAD
    if [ ! -s "$diff_file" ]; then
        # A clean tree is not a measured tree: changes committed earlier on
        # this branch still need mutating, so diff against the branch base.
        # The first main branch that exists is the base. One that shares no
        # history with HEAD is an error: falling back to the last commit
        # would measure less than the branch changed.
        main_ref=""
        for ref in origin/main origin/master main master; do
            if git rev-parse -q --verify "$ref^{commit}" > /dev/null; then
                main_ref="$ref"
                break
            fi
        done
        if [ -n "$main_ref" ]; then
            base=$(git merge-base HEAD "$main_ref") || {
                echo "no merge base of $main_ref and HEAD: nothing to measure against" >&2
                exit 1
            }
            if [ "$base" != "$(git rev-parse HEAD)" ]; then
                changes "$base" HEAD
            fi
        fi
    fi
    if [ ! -s "$diff_file" ] && git rev-parse -q --verify HEAD~1 > /dev/null; then
        changes HEAD~1 HEAD
    fi
fi

if [ ! -s "$diff_file" ]; then
    # An honest gate says what it did not measure: it never reports the
    # absence of survivors as evidence about a change it cannot see. For
    # everything mutants can measure, use `just mutants-all`.
    echo "No src/ or crates/*/src/ changes: outside the mutation gate's reach, nothing was measured"
    exit 0
fi
echo "Running mutants on changed lines (--in-diff)"
cargo mutants --workspace --in-diff "$diff_file"
