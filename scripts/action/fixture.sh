#!/usr/bin/env bash
# A fixture repository for the Action's scenario jobs: a small tested crate
# committed in `dir`. Usage: fixture.sh <dir>
set -euo pipefail
dir="$1"
mkdir -p "$dir/src"
cd "$dir"
cat > Cargo.toml <<'TOML'
[package]
name = "shop"
version = "0.1.0"
edition = "2021"

[workspace]
TOML
cat > src/lib.rs <<'RUST'
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
git init -q
git -c user.name=fixture -c user.email=fixture@example.com add -A
git -c user.name=fixture -c user.email=fixture@example.com commit -q -m base
