#!/usr/bin/env bash
# Renders the README hero from real cargo-crap and crapts runs:
#   docs/assets/hero/render.sh
# Writes docs/assets/hero-light.webp and docs/assets/hero-dark.webp.
#
# Rust: the shop crate in examples/hero-shop, its sources replaced by the
# pull request in examples/hero-shop/pr/, scored by this checkout's
# cargo-crap: the human table in colour, shields for the badge, and the
# pr-comment's duplicate section, triaged by OpenAI. Triage reads the
# verdicts in triage-cache/, so no API key is needed.
#
# TypeScript: shop-ts/ holds a main-branch tree (base/), the pull request's
# changed file (pr/) and the head coverage (pr.lcov). The coverage is a
# committed input, so the render needs no npm install. They become a
# two-commit Git repository, and this checkout's crapts scores it against
# main as pr-comment and github.
#
# Needs cargo-llvm-cov, gh (for GitHub's Markdown renderer), Google
# Chrome and cwebp. crapts needs Rust 1.98 or newer. HERO_KEEP=<dir> also
# keeps the runs, hero.html and the PNGs there, for working on the page.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
runs=$work/runs
mkdir -p "$runs"

# Both binaries land in this checkout's target, whatever CARGO_TARGET_DIR
# says, before it is pointed at the scratch shop below.
cargo build --quiet --release --features triage --manifest-path "$root/Cargo.toml" \
  --target-dir "$root/target"
cargo build --quiet --release -p crapts --manifest-path "$root/Cargo.toml" \
  --target-dir "$root/target"
crap=$root/target/release/cargo-crap
crapts=$root/target/release/crapts

# TypeScript. Git config is isolated and the dates fixed, so a user's signing
# or hooks never run and the commits are the same on every render. An
# inherited repository or index override (from a Git hook, say) would point
# git init, the commits and crapts' own git calls at the caller's repository.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY \
  GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_PREFIX GIT_CONFIG GIT_CONFIG_COUNT \
  GIT_CONFIG_PARAMETERS
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
  GIT_AUTHOR_NAME=Demo GIT_AUTHOR_EMAIL=demo@example.com \
  GIT_COMMITTER_NAME=Demo GIT_COMMITTER_EMAIL=demo@example.com \
  GIT_AUTHOR_DATE=2026-01-01T00:00:00Z GIT_COMMITTER_DATE=2026-01-01T00:00:00Z
cp -R "$here/shop-ts/base" "$work/shop-ts"
cp -R "$here/shop-ts/base" "$runs/ts-base"
(cd "$work/shop-ts"
  git init -q -b main && git add . && git commit -q -m main
  git checkout -q -b pr
  cp "$here"/shop-ts/pr/src/*.ts src/
  git commit -q -am "pull request")
cp -R "$work/shop-ts" "$runs/ts-pr" && rm -rf "$runs/ts-pr/.git"
# The coverage sits outside the repository, so discovery does not list it.
cp "$here/shop-ts/pr.lcov" "$runs/ts.lcov"
ts() { (cd "$work/shop-ts" && "$crapts" --coverage "$runs/ts.lcov" --threshold 15 --base main "$@"); }
ts --format pr-comment > "$runs/ts-comment.md"
ts --format github > "$runs/ts-github.txt"

# Rust.
cp -R "$root/examples/hero-shop" "$work/shop"
rm -rf "$work/shop/pr" "$work/shop/target"
cd "$work/shop"
export CARGO_TARGET_DIR=$work/shop/target
cp "$root"/examples/hero-shop/pr/*.rs src/
cargo llvm-cov --quiet --lcov --output-path "$runs/pr.lcov"

run() { "$crap" --lcov "$runs/pr.lcov" --threshold 15 "$@"; }
FORCE_COLOR=1 run > "$runs/human.ansi"
run --format shields > "$runs/shields.json"

# The duplicate section of the PR comment, triaged by OpenAI from the
# cached verdicts. No key is set, so a pair missing from the cache would
# leave the section untriaged rather than ask the API.
mkdir -p "$CARGO_TARGET_DIR/cargo-crap/triage"
cp "$here"/triage-cache/*.json "$CARGO_TARGET_DIR/cargo-crap/triage/"
printf '[duplicates]\nenabled = true\n\n[duplicates.triage]\nenabled = true\nprovider = "openai"\n' \
  > .cargo-crap.toml
(unset TYPESAFE_API_KEY OPENAI_API_KEY
  run --format pr-comment \
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
