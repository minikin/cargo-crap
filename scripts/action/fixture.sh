#!/usr/bin/env bash
# A fixture repository for the Action's scenario jobs: a small tested crate
# committed in `dir`, and, with a change named, a second commit standing for
# a pull request. Prints nothing. The base commit is `HEAD~1` when there is a
# change, `HEAD` otherwise.
#
# Usage: fixture.sh <dir> [regress|steady|config-skew|extra|lone-copy]
#   regress      the pull request adds untested branches to `shipping`
#   steady       the pull request only rewords a comment
#   config-skew  the base commit's config excludes src/lib.rs, the pull
#                request's does not
#   extra        both commits hold src/extra.rs, the pull request changes it
#   lone-copy    the base commit holds a duplicate pair, the pull request
#                deletes one of the two
set -euo pipefail
dir="$1"
change="${2:-}"
mkdir -p "$dir/src"
cd "$dir"
commit() {
    git -c user.name=fixture -c user.email=fixture@example.com add -A
    git -c user.name=fixture -c user.email=fixture@example.com commit -q -m "$1"
}
cat > Cargo.toml <<'TOML'
[package]
name = "shop"
version = "0.1.0"
edition = "2021"

[workspace]
TOML
shipping() {
    cat <<'RUST'
/// Shipping in cents by parcel weight in grams.
pub fn shipping(grams: u32) -> u32 {
    if grams > 10_000 {
        900
    } else if grams > 2_000 {
        450
    } else {
        250
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_light_parcel_is_cheap() {
        assert_eq!(super::shipping(500), 250);
    }
}
RUST
}
copy() {
    cat <<RUST
pub fn $1(xs: &[i32]) -> Vec<i32> {
    let mut ys = Vec::new();
    for x in xs {
        if x % 2 == 1 {
            ys.push(x + 1);
        }
    }
    ys
}
RUST
}
shipping > src/lib.rs
case "$change" in
    config-skew) printf 'exclude = ["src/lib.rs"]\n' > .cargo-crap.toml ;;
    extra) copy extra_one > src/extra.rs && echo "pub mod extra;" >> src/lib.rs ;;
    lone-copy) { copy one; copy two; } > src/copies.rs && echo "pub mod copies;" >> src/lib.rs ;;
esac
git init -q
commit base
case "$change" in
    "") exit 0 ;;
    regress)
        python3 - <<'PY'
import pathlib
p = pathlib.Path("src/lib.rs"); s = p.read_text()
s = s.replace("    if grams > 10_000 {\n        900\n", "    if grams > 50_000 {\n        2_000\n    } else if grams > 30_000 {\n        1_400\n    } else if grams > 10_000 {\n        900\n", 1)
p.write_text(s)
PY
        ;;
    steady) sed -i.bak 's|/// Shipping in cents by parcel weight in grams.|/// Shipping in cents, by weight in grams.|' src/lib.rs && rm src/lib.rs.bak ;;
    config-skew) rm .cargo-crap.toml ;;
    extra) copy extra_two > src/extra.rs ;;
    lone-copy) copy one > src/copies.rs ;;
    *) echo "unknown change: $change" >&2 && exit 1 ;;
esac
commit pr
