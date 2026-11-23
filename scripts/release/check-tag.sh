#!/usr/bin/env bash
set -eu
exec python3 -B "$(dirname "$0")/release_version.py" "$@"
