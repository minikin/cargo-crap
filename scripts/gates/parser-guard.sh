#!/usr/bin/env bash
# Neither front end links the other's parser, and the core links none.
#
# What the tree holds is what the package links:
#   --all-features  a parser behind an optional feature does not hide
#   --target all    nor does one behind a cfg(windows) table on a Linux host
#   normal,build    dev-dependencies are left out: a parser only tests use
#                   links nothing
#   no-proc-macro   syn compiles on the host for clap_derive and
#                   serde_derive whatever the package links
#
# PARSER_GUARD_TREES=<dir> reads <dir>/<package>.tree instead of running
# cargo, which is how the tests feed it trees no real package has.
set -euo pipefail

# One rule per line: a package and a prefix no line of its tree may start
# with. The packages checked are the ones named here, so a package cannot be
# listed without a rule. `syn ` keeps its space: `synstructure` is not syn.
rules=(
    "cargo-crap oxc_"
    "cargo-crap-core oxc_"
    "cargo-crap-core syn "
)

# `-` defaults: macOS's /bin/bash 3.2 reads an empty array as unbound
# under `set -u`.
packages=()
for rule in "${rules[@]}"; do
    package="${rule%% *}"
    case " ${packages[*]-} " in
        *" $package "*) ;;
        *) packages+=("$package") ;;
    esac
done

tree_of() {
    if [ -n "${PARSER_GUARD_TREES:-}" ]; then
        cat "$PARSER_GUARD_TREES/$1.tree" 2> /dev/null
    else
        cargo tree -p "$1" --all-features --target all -e normal,build,no-proc-macro --prefix none --locked
    fi
}

violations=0
for package in "${packages[@]}"; do
    if ! tree="$(tree_of "$package")" || [ -z "$tree" ]; then
        echo "parser guard: $package: no tree was read" >&2
        exit 1
    fi
    for rule in "${rules[@]}"; do
        [ "${rule%% *}" = "$package" ] || continue
        prefix="${rule#* }"
        # Matched in bash over the tree's lines, with no pipe, process
        # substitution or here-string: a helper's exit status or a redirect
        # that failed (bash 3.2 backs a here-string with a temp file) would
        # read as a clean tree.
        rest="$tree"
        while [ -n "$rest" ]; do
            line="${rest%%$'\n'*}"
            if [ "$line" = "$rest" ]; then rest=""; else rest="${rest#*$'\n'}"; fi
            case "$line" in
                "$prefix"*)
                    echo "parser guard: $package links $line" >&2
                    violations=$((violations + 1))
                    ;;
            esac
        done
    done
done

if [ "$violations" -gt 0 ]; then
    exit 1
fi
checked="${packages[*]}"
# Recorded trees say so, so a guard fed stale files cannot pass for one
# that read the repository.
source="${PARSER_GUARD_TREES:+recorded trees from $PARSER_GUARD_TREES: }"
echo "parser guard: ${source}checked ${checked// /, }"
