#!/usr/bin/env bash
# Tests for scripts/release/prep.sh. Run: bash scripts/release/test.sh
#
# Each case copies the repository's own release files into a temp dir, so a
# change to their layout that prep.sh no longer understands fails here.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
prep="$here/prep.sh"

failures=0
check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: expected '$expected', got '$actual'"
        failures=$((failures + 1))
    fi
}

files=(
    Cargo.toml
    CHANGELOG.md
    action.yml
    .github/ISSUE_TEMPLATE/bug_report.yml
    .github/ISSUE_TEMPLATE/feature_request.yml
    docs/reference/json.md
)

# A copy of the release files with one entry under [Unreleased], so the
# copy has something to release whatever state the real CHANGELOG is in.
fixture() {
    local dir
    dir="$(mktemp -d)" || exit 1
    for f in "${files[@]}"; do
        mkdir -p "$dir/$(dirname "$f")"
        cp "$repo/$f" "$dir/$f"
    done
    perl -0pi -e 's/^## \[Unreleased\]\n/## [Unreleased]\n\n### Added\n\n- Something.\n/m' "$dir/CHANGELOG.md"
    echo "$dir"
}

current="$(awk -F'"' '/^version = /{print $2; exit}' "$repo/Cargo.toml")"
IFS=. read -r major minor _ <<< "$current"
next="$major.$((minor + 1)).0"

# A release bumps every place the version is written and dates the CHANGELOG.
dir="$(fixture)"
(cd "$dir" && bash "$prep" "$next" 2031-02-03 > /dev/null)
check "it succeeds" "0" "$?"
check "Cargo.toml" "$next" "$(awk -F'"' '/^version = /{print $2; exit}' "$dir/Cargo.toml")"
check "the Action's default version" "1" \
    "$(grep -c "^    default: \"$next\"$" "$dir/action.yml")"
check "the bug report placeholder" "1" \
    "$(grep -c "cargo-crap $next" "$dir/.github/ISSUE_TEMPLATE/bug_report.yml")"
check "the feature request placeholder" "1" \
    "$(grep -c "cargo-crap $next" "$dir/.github/ISSUE_TEMPLATE/feature_request.yml")"
check "the JSON reference's examples" "0" \
    "$(grep -c "\"version\": \"$current\"" "$dir/docs/reference/json.md")"
check "the dated heading follows an empty Unreleased" "## [Unreleased]|||## [$next] - 2031-02-03" \
    "$(grep -A3 '^## \[Unreleased\]$' "$dir/CHANGELOG.md" | tr '\n' '|' | cut -d'|' -f1-4)"
check "the Unreleased link compares from the new tag" "1" \
    "$(grep -c "^\[Unreleased\]: .*/compare/v$next\.\.\.HEAD$" "$dir/CHANGELOG.md")"
check "the new version's link compares the two tags" "1" \
    "$(grep -c "^\[$next\]: .*/compare/v$current\.\.\.v$next$" "$dir/CHANGELOG.md")"
check "the old version's link is kept" "1" \
    "$(grep -c "^\[$current\]: " "$dir/CHANGELOG.md")"
rm -rf "$dir"

# Nothing to release: an empty [Unreleased] is refused, and no file changes.
dir="$(fixture)"
cp "$repo/CHANGELOG.md" "$dir/CHANGELOG.md"
perl -0pi -e 's/^## \[Unreleased\]\n.*?(?=^## \[)/## [Unreleased]\n\n\n/ms' "$dir/CHANGELOG.md"
cp "$dir/Cargo.toml" "$dir/Cargo.toml.before"
(cd "$dir" && bash "$prep" "$next" > /dev/null 2>&1)
check "an empty Unreleased fails" "1" "$?"
check "and leaves Cargo.toml alone" "" "$(diff "$dir/Cargo.toml.before" "$dir/Cargo.toml")"
rm -rf "$dir"

# Arguments that are not a version, a date or a change are refused.
dir="$(fixture)"
(cd "$dir" && bash "$prep" "v$next" > /dev/null 2>&1)
check "a leading v fails" "2" "$?"
(cd "$dir" && bash "$prep" "$next" 03/02/2031 > /dev/null 2>&1)
check "a date in another form fails" "2" "$?"
(cd "$dir" && bash "$prep" "$current" > /dev/null 2>&1)
check "the current version fails" "1" "$?"
(cd "$dir" && bash "$prep" > /dev/null 2>&1)
check "no version fails" "2" "$?"
rm -rf "$dir"

if [ "$failures" -gt 0 ]; then
    echo "$failures failure(s)"
    exit 1
fi
echo "all passed"
