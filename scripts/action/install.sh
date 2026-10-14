#!/usr/bin/env bash
# Put cargo-crap on PATH: the BINARY given, or the release archive of VERSION
# for this runner.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

if [ -n "${BINARY:-}" ]; then
    if [ ! -f "$BINARY" ]; then
        echo "::error::binary: $BINARY is not a file"
        exit 1
    fi
    dir="$(cd "$(dirname "$BINARY")" && pwd)"
else
    target="$(target_for "$RUNNER_OS" "$RUNNER_ARCH")"
    archive="$(archive_for "$target")"
    dir="$RUNNER_TEMP/cargo-crap-bin"
    mkdir -p "$dir"
    curl -fsSL --retry 3 -o "$dir/$archive" "$(download_url "$VERSION" "$archive")"
    case "$archive" in
        *.zip) unzip -q -o "$dir/$archive" -d "$dir" ;;
        *) tar xzf "$dir/$archive" -C "$dir" ;;
    esac
fi
# The runner reads GITHUB_PATH as a native path: on Windows, not Git Bash's.
if command -v cygpath > /dev/null; then
    cygpath -w "$dir" >> "$GITHUB_PATH"
else
    echo "$dir" >> "$GITHUB_PATH"
fi
PATH="$dir:$PATH" cargo-crap --version
