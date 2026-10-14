#!/usr/bin/env bash
# Give a fixture an LCOV file and a baseline of its base commit scored with
# it, so a scenario job about anything but coverage runs the Action in
# seconds and still compares like with like.
# Usage: prepared.sh <dir> <cargo-crap>
set -euo pipefail
cd "$1"
chmod +x "$2"
{
    echo "SF:src/lib.rs"
    for line in $(seq 2 20); do echo "DA:$line,1"; done
    echo end_of_record
} > lcov.info
git checkout -q HEAD~1
"$2" --path . --lcov lcov.info --format json --output base.json
git checkout -q -
