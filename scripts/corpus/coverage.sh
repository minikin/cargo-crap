#!/usr/bin/env bash
set -euo pipefail

die() {
    echo "corpus coverage: $1" >&2
    exit "${2:-1}"
}

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
fixture_dir="$repo_root/crates/crapts/tests/fixtures/corpus/excalidraw"
pin_file="$fixture_dir/pin.txt"
provider="v8"
repo_url="https://github.com/excalidraw/excalidraw"

[ -f "$pin_file" ] || die "missing pin file: $pin_file"

line_count="$(wc -l < "$pin_file" | tr -d '[:space:]')"
if [ "$line_count" -gt 1 ]; then
    die "pin file must contain a single commit SHA: $pin_file"
fi

IFS= read -r raw_sha < "$pin_file" || true
sha="${raw_sha%$'\r'}"
if [[ ! "$sha" =~ ^[0-9a-f]{40}$ ]]; then
    die "invalid commit SHA in $pin_file: '$sha'"
fi

for tool in git node yarn gzip cargo; do
    command -v "$tool" > /dev/null 2>&1 || die "required tool not found: $tool"
done

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

checkout_dir="$work_dir/excalidraw"
bash "$script_dir/checkout.sh" "$checkout_dir"

yarn --cwd "$checkout_dir" install --frozen-lockfile

coverage_cmd="yarn vitest run --coverage --coverage.reporter=json --coverage.provider=$provider --coverage.reportOnFailure=true"
vitest_exit=0
(
    cd "$checkout_dir"
    yarn vitest run \
        --coverage \
        --coverage.reporter=json \
        --coverage.provider="$provider" \
        --coverage.reportOnFailure=true
) || vitest_exit=$?

raw_coverage="$checkout_dir/coverage/coverage-final.json"
[ -s "$raw_coverage" ] || die "vitest exited $vitest_exit without producing $raw_coverage"

normalized_coverage="$work_dir/coverage-final.json"
node - "$checkout_dir" "$raw_coverage" "$normalized_coverage" <<'EOF'
const fs = require("fs");
const path = require("path");

const [checkoutArg, inputPath, outputPath] = process.argv.slice(2);
const roots = Array.from(
  new Set([
    path.resolve(checkoutArg),
    fs.realpathSync(checkoutArg),
  ]),
).map((root) => root.replace(/\\/g, "/").replace(/\/+$/, ""));

function relativize(rawPath) {
  const normalized = String(rawPath).replace(/\\/g, "/");
  let rel = normalized;
  for (const root of roots) {
    const prefix = `${root}/`;
    if (rel.startsWith(prefix)) {
      rel = rel.slice(prefix.length);
      break;
    }
  }
  rel = rel.replace(/^\.\//, "");
  if (
    rel.length === 0 ||
    rel.startsWith("/") ||
    rel.startsWith("../") ||
    rel === ".." ||
    /^[A-Za-z]:/.test(rel)
  ) {
    throw new Error(`coverage path is not checkout-relative: ${rawPath} -> ${rel}`);
  }
  return rel;
}

const raw = JSON.parse(fs.readFileSync(inputPath, "utf8"));
const entries = Object.entries(raw).map(([key, value]) => {
  const relKey = relativize(key);
  const relPath = relativize(value.path);
  if (relKey !== relPath) {
    throw new Error(`coverage key/path mismatch: ${relKey} !== ${relPath}`);
  }
  return [relKey, { ...value, path: relPath }];
});
entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));

const out = Object.fromEntries(entries);
fs.writeFileSync(outputPath, `${JSON.stringify(out)}\n`, "utf8");
EOF

gzip -n -9 -c "$normalized_coverage" > "$fixture_dir/coverage-final.json.gz"

node_version="$(node --version)"
yarn_version="$(yarn --version)"
vitest_version="$(node -e 'console.log(require(process.argv[1]).version)' "$checkout_dir/node_modules/vitest/package.json")"

rm -rf "$checkout_dir/node_modules" "$checkout_dir/coverage"
(
    cd "$repo_root"
    CRAPTS_CORPUS_DIR="$checkout_dir" \
        CRAPTS_CORPUS_UPDATE=1 \
        cargo test --locked -p crapts --test corpus -- --ignored
)

node - \
    "$fixture_dir/coverage-meta.json" \
    "$repo_url" \
    "$sha" \
    "$coverage_cmd" \
    "$provider" \
    "$vitest_exit" \
    "$node_version" \
    "$yarn_version" \
    "$vitest_version" \
    "$fixture_dir/coverage-final.json.gz" \
    "$fixture_dir/golden.json" \
    "$fixture_dir/golden.stderr" <<'EOF'
const fs = require("fs");

const [
  metaPath,
  repository,
  commit,
  command,
  provider,
  vitestExit,
  nodeVersion,
  yarnVersion,
  vitestVersion,
  gzPath,
  goldenJsonPath,
  goldenStderrPath,
] = process.argv.slice(2);

function requireFileSize(filePath) {
  if (!fs.existsSync(filePath)) {
    throw new Error(`missing corpus artifact: ${filePath}`);
  }
  return fs.statSync(filePath).size;
}

const meta = {
  repository,
  commit,
  command,
  provider,
  reportOnFailure: true,
  vitestExitCode: Number(vitestExit),
  versions: {
    node: nodeVersion,
    yarn: yarnVersion,
    vitest: vitestVersion,
  },
  artifacts: {
    coverage_final_json_gz_bytes: requireFileSize(gzPath),
    golden_json_bytes: requireFileSize(goldenJsonPath),
    golden_stderr_bytes: requireFileSize(goldenStderrPath),
  },
};

fs.writeFileSync(metaPath, `${JSON.stringify(meta, null, 2)}\n`, "utf8");
EOF
