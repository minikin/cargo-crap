#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
if command -v python3 >/dev/null; then python=python3; else python=python; fi
"$python" -m unittest discover -s scripts/action/crapts/tests -v
