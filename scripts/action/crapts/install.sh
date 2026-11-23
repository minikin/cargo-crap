#!/usr/bin/env bash
set -euo pipefail
fail() { echo "$*" >&2; exit 2; }
version=$(node --version) || fail "Use actions/setup-node with node-version 24 or later"
major=${version%%.*}
if ! [[ ${major#v} =~ ^[0-9]+$ ]] || (( ${major#v} < 24 )); then
    fail "Use actions/setup-node with node-version 24 or later"
fi
temp=$(mktemp -d "$RUNNER_TEMP/crapts.XXXXXXXX") || exit 2
node "$(dirname "$0")/inputs.mjs" > "$temp/args.json"
if [[ -n $INPUT_PACKAGE ]]; then
    [[ -f $INPUT_PACKAGE ]] || fail "package: missing file $INPUT_PACKAGE"
    package=$(node -p 'require("node:path").resolve(process.argv[1])' -- "$INPUT_PACKAGE")
    expected=$(tar -xOzf - package/package.json < "$package" | node -p 'JSON.parse(require("node:fs").readFileSync(0)).version') || fail "package: invalid manifest in $INPUT_PACKAGE"
else
    expected=$INPUT_VERSION
    if [[ $expected == latest ]]; then
        expected=$(cd "$temp" && npm view @minikin/crapts@latest version) || fail "version: cannot resolve latest"
    fi
    package="@minikin/crapts@$expected"
fi
[[ $expected =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] || fail "expected an exact package version, got $expected"
cd "$temp"
npm install --prefix . --ignore-scripts --include=optional --no-audit --no-fund -- "$package" || fail "could not install $package"
bin="$temp/node_modules/.bin"
actual=$("$bin/crapts" --version) || fail "could not verify crapts from $package"
[[ $actual == "crapts $expected" ]] || fail "expected $expected, actual ${actual#crapts }"
node -e '
  const fs = require("node:fs"), path = require("node:path");
  fs.appendFileSync(process.env.GITHUB_PATH, path.resolve("node_modules/.bin") + "\n");
  const local = require("node:module").createRequire(require.resolve("@minikin/crapts/package.json"));
  const {detectLibc, resolveTarget} = local("./lib/resolve.js");
  const target = resolveTarget(process.platform, process.arch, process.platform === "linux" ? detectLibc() : "none");
  const binary = local.resolve(target.name + "/" + target.binary);
  fs.appendFileSync(process.env.GITHUB_OUTPUT, "temp=" + process.cwd() + "\nbinary=" + binary + "\n");
' || fail "could not locate the executable from $package"
