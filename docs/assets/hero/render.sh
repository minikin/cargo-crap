#!/usr/bin/env bash
# Renders the README hero from real cargo-crap runs over examples/hero-shop:
#   docs/assets/hero/render.sh
# Writes docs/assets/hero-light.webp and docs/assets/hero-dark.webp.
#
# The shop crate's main-branch source is examples/hero-shop/src/ and its
# pull request replaces it with the files in examples/hero-shop/pr/. Each
# panel is one invocation of this checkout's cargo-crap: the human table in
# colour, and pr-comment (with --duplicates, triaged by OpenAI), github and
# shields against a baseline taken from the main-branch source. Triage reads
# the verdicts in triage-cache/, so no API key is needed.
# Needs cargo-llvm-cov, gh (for GitHub's Markdown renderer), curl, Google
# Chrome and cwebp. HERO_KEEP=<dir> also keeps the runs, hero.html and the
# PNGs there, for working on the page.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
runs=$work/runs
mkdir -p "$runs"

cargo build --quiet --release --features triage --manifest-path "$root/Cargo.toml"
crap=$root/target/release/cargo-crap

cp -R "$root/examples/hero-shop" "$work/shop"
rm -rf "$work/shop/pr" "$work/shop/target"
cd "$work/shop"
export CARGO_TARGET_DIR=$work/shop/target

# main: coverage and the baseline the pull request is compared with.
mkdir -p "$runs/base" && cp -R src "$runs/base/"
cargo llvm-cov --quiet --lcov --output-path "$runs/base.lcov"
"$crap" --lcov "$runs/base.lcov" --format json --output "$runs/baseline.json"

# The pull request.
cp "$root"/examples/hero-shop/pr/*.rs src/
mkdir -p "$runs/pr" && cp -R src "$runs/pr/"
cargo llvm-cov --quiet --lcov --output-path "$runs/pr.lcov"

run() { "$crap" --lcov "$runs/pr.lcov" --threshold 15 "$@"; }
FORCE_COLOR=1 run > "$runs/human.ansi"
run --baseline "$runs/baseline.json" --format github > "$runs/github.txt"
run --format shields > "$runs/shields.json"

# The PR comment, with its duplicate section triaged by OpenAI from the
# cached verdicts. No key is set, so a pair missing from the cache would
# leave the section untriaged rather than ask the API.
mkdir -p "$CARGO_TARGET_DIR/cargo-crap/triage"
cp "$here"/triage-cache/*.json "$CARGO_TARGET_DIR/cargo-crap/triage/"
printf '[duplicates]\nenabled = true\n\n[duplicates.triage]\nenabled = true\nprovider = "openai"\n' \
  > .cargo-crap.toml
(unset TYPESAFE_API_KEY OPENAI_API_KEY
  run --baseline "$runs/baseline.json" --format pr-comment \
    --repo-url https://github.com/minikin/cargo-crap --commit-ref main > "$runs/pr-comment.md")

python3 "$here/build.py" "$runs"

chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
for candidate in google-chrome google-chrome-stable chromium; do
  command -v "$candidate" >/dev/null 2>&1 && chrome=$candidate && break
done
for theme in light dark; do
  "$chrome" --headless=new --disable-gpu --hide-scrollbars \
    --allow-file-access-from-files --force-device-scale-factor=3 \
    --window-size=1600,1112 --virtual-time-budget=10000 \
    --screenshot="$work/hero-$theme.png" "file://$here/hero.html?theme=$theme" 2>/dev/null
  cwebp -quiet -q 90 -m 6 "$work/hero-$theme.png" -o "$root/docs/assets/hero-$theme.webp"
done
if [ -n "${HERO_KEEP:-}" ]; then
  mkdir -p "$HERO_KEEP"
  cp -R "$runs" "$work"/hero-*.png "$HERO_KEEP/"
  mv "$here/hero.html" "$HERO_KEEP/"
else
  rm "$here/hero.html"
fi
