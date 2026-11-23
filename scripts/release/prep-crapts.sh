#!/usr/bin/env bash
set -eu
exec python3 -B "$(dirname "$0")/prep_crapts.py" "$@"
