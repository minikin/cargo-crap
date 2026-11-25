#!/usr/bin/env bash
# Put cargo-crap on PATH: the BINARY given, or the release archive of VERSION
# for this runner.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

name="$(binary_name "$RUNNER_OS")"
if [ -n "${BINARY:-}" ]; then
    file_input binary "$BINARY"
    # A copy under the name the later steps call, in a directory of its own:
    # a renamed artifact still runs, a sibling named cargo-crap does not, and
    # a binary inside the checkout cannot change while the Action runs.
    dir="$(mktemp -d "$RUNNER_TEMP/cargo-crap-given.XXXXXX")"
    dir="$(cd "$dir" && pwd)"
    cp "$BINARY" "$dir/$name"
    # An uploaded artifact loses its execute bit on the way down.
    chmod +x "$dir/$name"
else
    target="$(target_for "$RUNNER_OS" "$RUNNER_ARCH")"
    archive="$(archive_for "$target")"
    dir="$(cd "$RUNNER_TEMP" && pwd)/cargo-crap-bin"
    mkdir -p "$dir"
    # The sums first: without them nothing is installed, so a failure costs
    # no archive download.
    if ! curl -fsSL --retry 3 -o "$dir/SHA256SUMS" "$(download_url "$VERSION" SHA256SUMS)"; then
        echo "::error::could not download SHA256SUMS from the cargo-crap $VERSION release, which every release carries: see the error above"
        exit 1
    fi
    curl -fsSL --retry 3 -o "$dir/$archive" "$(download_url "$VERSION" "$archive")"
    # Checked before anything is unpacked, so a mismatch leaves nothing to run.
    verify_archive "$dir/$archive" "$dir/SHA256SUMS"
    case "$archive" in
        *.zip)
            if command -v unzip > /dev/null; then
                unzip -q -o "$dir/$archive" -d "$dir"
            else
                tar xf "$dir/$archive" -C "$dir"
            fi
            ;;
        *) tar xzf "$dir/$archive" -C "$dir" ;;
    esac
fi
# The runner reads GITHUB_PATH as a native path: on Windows, not Git Bash's.
if command -v cygpath > /dev/null; then
    cygpath -w "$dir" >> "$GITHUB_PATH"
else
    echo "$dir" >> "$GITHUB_PATH"
fi
# The first run of the binary, and it needs no provider key.
keyless "$dir/$name" --version
