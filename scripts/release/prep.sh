#!/usr/bin/env bash
# Prepare a release commit: write the new version everywhere a release
# writes it and date the CHANGELOG's [Unreleased] work, as the 0.8.0 and
# 0.9.0 release commits did by hand. Run from the repository root:
#
#   scripts/release/prep.sh 0.10.0 [2026-11-02]
#
# The date defaults to today. Every edit is checked: a file whose layout no
# longer matches stops the script, rather than leaving a release that names
# two versions. What is left to a person is printed at the end.
# The perl expressions are single-quoted on purpose: perl reads $ENV{...}.
# shellcheck disable=SC2016
set -euo pipefail

die() {
    echo "release prep: $1" >&2
    exit "${2:-1}"
}

version="${1:-}"
date="${2:-$(date +%Y-%m-%d)}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: prep.sh <X.Y.Z> [YYYY-MM-DD], got version '$version'" 2
[[ "$date" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "the date must be YYYY-MM-DD, got '$date'" 2

old="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
[ -n "$old" ] || die "no version line in Cargo.toml"
[ "$old" != "$version" ] || die "Cargo.toml already says $version"

# Only lines with text count: the section is blank between releases.
unreleased="$(awk '/^## \[Unreleased\]/{f=1; next} /^## \[/{f=0} f && NF' CHANGELOG.md)"
[ -n "$unreleased" ] || die "CHANGELOG.md has nothing under [Unreleased] to release"

export OLD="$old" NEW="$version" DATE="$date"

# Edit `file` with the perl substitution `expr`, then require `marker` (a
# fixed string) in the result.
edit() {
    local file="$1" expr="$2" marker="$3"
    perl -0pi -e "$expr" "$file"
    grep -qF -- "$marker" "$file" || die "$file: the edit did not apply; has its layout changed?"
}

edit Cargo.toml 's/^version = "\Q$ENV{OLD}\E"$/version = "$ENV{NEW}"/m' "version = \"$version\""
edit action.yml 's/^(    default: )"\Q$ENV{OLD}\E"$/$1"$ENV{NEW}"/m' "default: \"$version\""
for template in .github/ISSUE_TEMPLATE/bug_report.yml .github/ISSUE_TEMPLATE/feature_request.yml; do
    edit "$template" 's/cargo-crap \Q$ENV{OLD}\E\b/cargo-crap $ENV{NEW}/g' "cargo-crap $version"
done
edit docs/reference/json.md 's/"version": "\Q$ENV{OLD}\E",/"version": "$ENV{NEW}",/g' "\"version\": \"$version\","
edit CHANGELOG.md 's/^## \[Unreleased\]\n/## [Unreleased]\n\n\n## [$ENV{NEW}] - $ENV{DATE}\n/m' "## [$version] - $date"
edit CHANGELOG.md 's{^\[Unreleased\]: (\S+)/compare/v\Q$ENV{OLD}\E\.\.\.HEAD$}{[Unreleased]: $1/compare/v$ENV{NEW}...HEAD\n[$ENV{NEW}]: $1/compare/v$ENV{OLD}...v$ENV{NEW}}m' \
    "[$version]: "

cat <<EOF
Bumped $old to $version and dated the CHANGELOG $date.
Left for you:
  - a summary paragraph under "## [$version] - $date" in CHANGELOG.md
  - "Status: Implemented" on each spec whose tasks are all ticked
  - just dev, then a "chore(release): $version" pull request
  - after it merges: tag v$version on main and push the tag
EOF
