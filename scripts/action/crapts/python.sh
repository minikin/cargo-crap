#!/usr/bin/env bash
set -euo pipefail
# Windows runners name the interpreter python. Linux and macOS runners
# guarantee only python3.
if [[ ${RUNNER_OS:-} == Windows ]]; then exec python "$@"; fi
exec python3 "$@"
