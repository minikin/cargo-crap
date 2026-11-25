#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
export PYTHONUTF8=1
CRAPTS_TEST_BASH=$(node -p 'process.argv[1]' "$BASH")
export CRAPTS_TEST_BASH
bash scripts/action/crapts/python.sh -m unittest discover -s scripts/action/crapts/tests -v
node --test scripts/action/crapts/tests/*.test.mjs
