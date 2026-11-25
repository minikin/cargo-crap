#!/usr/bin/env bash
set -euo pipefail
dir=$1
mkdir -p "$dir"
bash "$(dirname "$0")/python.sh" "$(dirname "$0")/tests/registry.py" "$dir" > "$dir/registry.log" 2>&1 &
pid=$!
for ((i=0; i<300; i++)); do
    if [[ -s "$dir/registry-url" ]]; then
        echo "npm_config_registry=$(cat "$dir/registry-url")" >> "$GITHUB_ENV"
        exit 0
    fi
    kill -0 "$pid" || { cat "$dir/registry.log" >&2; exit 1; }
    sleep 0.2
done
echo "Timed out starting the fixture registry" >&2
exit 1
