#!/usr/bin/env bash
set -euo pipefail
fail() { echo "$*" >&2; exit 2; }
# shellcheck source-path=SCRIPTDIR source=../lib.sh
source "$(dirname "$0")/../lib.sh"
version=$(keyless node --version) || fail "Use actions/setup-node with node-version 24 or later"
major=${version%%.*}
if ! [[ ${major#v} =~ ^[0-9]+$ ]] || (( ${major#v} < 24 )); then
    fail "Use actions/setup-node with node-version 24 or later"
fi
[[ -n ${RUNNER_TEMP:-} ]] || fail "RUNNER_TEMP is required to install crapts"
temp=$(mktemp -d "$RUNNER_TEMP/crapts.XXXXXXXX") || exit 2
keyless node "$(dirname "$0")/inputs.mjs" > "$temp/args.json"
keyless node "$(dirname "$0")/cli.mjs" trusted-urls >> "$GITHUB_OUTPUT"
if [[ -n $INPUT_PACKAGE ]]; then
    [[ -f $INPUT_PACKAGE ]] || fail "package: missing file $INPUT_PACKAGE"
    package=$(keyless node -p 'require("node:path").resolve(process.argv[1])' -- "$INPUT_PACKAGE")
    expected=$(tar -xOzf - package/package.json < "$package" | keyless node -p 'JSON.parse(require("node:fs").readFileSync(0)).version') || fail "package: invalid manifest in $INPUT_PACKAGE"
else
    expected=$INPUT_VERSION
    if [[ $expected == latest ]]; then
        expected=$(cd "$temp" && keyless npm view @minikin_/crapts@latest version) || fail "version: cannot resolve latest"
    fi
    package="@minikin_/crapts@$expected"
fi
[[ $expected =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || fail "expected an exact package version, got $expected"
cd "$temp"
keyless npm install --prefix . --ignore-scripts --include=optional --no-audit --no-fund -- "$package" || fail "could not install $package"
bin="$temp/node_modules/.bin"
actual=$(keyless "$bin/crapts" --version) || fail "could not verify crapts from $package"
[[ $actual == "crapts $expected" ]] || fail "expected $expected, actual ${actual#crapts }"
keyless node -e '
  const fs = require("node:fs"), path = require("node:path");
  fs.appendFileSync(process.env.GITHUB_PATH, path.resolve("node_modules/.bin") + "\n");
  const local = require("node:module").createRequire(require.resolve("@minikin_/crapts/package.json"));
  const {detectLibc, resolveTarget} = local("./lib/resolve.js");
  const target = resolveTarget(process.platform, process.arch, process.platform === "linux" ? detectLibc() : "none");
  const binary = local.resolve(target.name + "/" + target.binary);
  fs.appendFileSync(process.env.GITHUB_OUTPUT, "temp=" + process.cwd() + "\nbinary=" + binary + "\n");
' || fail "could not locate the executable from $package"
