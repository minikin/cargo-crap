#!/usr/bin/env bash
set -euo pipefail

die() {
    echo "corpus checkout: $1" >&2
    exit "${2:-1}"
}

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    die "usage: checkout.sh <target-dir>" 2
fi

dir="$1"
if [ -e "$dir" ]; then
    die "target directory already exists: $dir" 2
fi

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
pin_file="$repo_root/crates/crapts/tests/fixtures/corpus/excalidraw/pin.txt"

[ -f "$pin_file" ] || die "missing pin file: $pin_file"

line_count="$(wc -l < "$pin_file" | tr -d '[:space:]')"
if [ "$line_count" -gt 1 ]; then
    die "pin file must contain a single commit SHA: $pin_file"
fi

IFS= read -r raw_sha < "$pin_file" || true
sha="${raw_sha%$'\r'}"
if [[ ! "$sha" =~ ^[0-9a-f]{40}$ ]]; then
    die "invalid commit SHA in $pin_file: '$sha'"
fi

repo_url="${CRAPTS_CORPUS_REPO_URL:-https://github.com/excalidraw/excalidraw}"

git init "$dir"
git -C "$dir" remote add origin "$repo_url"
git -C "$dir" fetch --depth 1 origin "$sha"
git -C "$dir" checkout --detach FETCH_HEAD

actual="$(git -C "$dir" rev-parse HEAD)"
if [ "$actual" != "$sha" ]; then
    die "checked out HEAD $actual does not match pinned SHA $sha"
fi
