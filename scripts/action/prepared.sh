#!/usr/bin/env bash
# Give a fixture a baseline of its base commit and an LCOV file, so a
# scenario job about anything but coverage runs the Action in seconds.
# Usage: prepared.sh <dir> <cargo-crap>
set -euo pipefail
cd "$1"
chmod +x "$2"
git checkout -q HEAD~1
"$2" --path . --format json --output base.json
git checkout -q -
{
    echo "SF:src/lib.rs"
    for line in $(seq 2 20); do echo "DA:$line,1"; done
    echo end_of_record
} > lcov.info
