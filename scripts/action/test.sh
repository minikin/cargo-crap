#!/usr/bin/env bash
# Tests for scripts/action/lib.sh. Run: bash scripts/action/test.sh
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source-path=SCRIPTDIR source=lib.sh
source "$here/lib.sh"

failures=0
check() {
    local name="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "ok   $name"
    else
        echo "FAIL $name: expected '$expected', got '$actual'"
        failures=$((failures + 1))
    fi
}

# The runner's OS and architecture pick the release archive's target.
check "linux x64" "x86_64-unknown-linux-gnu" "$(target_for Linux X64)"
check "linux arm64" "aarch64-unknown-linux-gnu" "$(target_for Linux ARM64)"
check "macos x64" "x86_64-apple-darwin" "$(target_for macOS X64)"
check "macos arm64" "aarch64-apple-darwin" "$(target_for macOS ARM64)"
check "windows x64" "x86_64-pc-windows-msvc" "$(target_for Windows X64)"
check "an unknown runner has no target" "" "$(target_for Windows ARM64 2>/dev/null)"
target_for Windows ARM64 >/dev/null 2>&1
check "an unknown runner fails" "1" "$?"

# The archive each target is released as.
check "a unix archive" "cargo-crap-x86_64-unknown-linux-gnu.tar.gz" \
    "$(archive_for x86_64-unknown-linux-gnu)"
check "a windows archive" "cargo-crap-x86_64-pc-windows-msvc.zip" \
    "$(archive_for x86_64-pc-windows-msvc)"

# Where a version is downloaded from.
check "a pinned version" \
    "https://github.com/minikin/cargo-crap/releases/download/v0.8.0/cargo-crap-aarch64-apple-darwin.tar.gz" \
    "$(download_url 0.8.0 cargo-crap-aarch64-apple-darwin.tar.gz)"
check "the latest version" \
    "https://github.com/minikin/cargo-crap/releases/latest/download/cargo-crap-aarch64-apple-darwin.tar.gz" \
    "$(download_url latest cargo-crap-aarch64-apple-darwin.tar.gz)"

# The checksums file sits beside the archives of the same release.
check "a pinned version's sums" \
    "https://github.com/minikin/cargo-crap/releases/download/v0.8.0/SHA256SUMS" \
    "$(download_url 0.8.0 SHA256SUMS)"

# An archive is installed only when its digest is the one the release lists.
sums_dir="$(mktemp -d)"
printf 'a release archive' > "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz"
printf 'another archive' > "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz"
(cd "$sums_dir" && digest_of cargo-crap-x86_64-unknown-linux-gnu.tar.gz) > "$sums_dir/digest"
check "a digest is sha256" "fd6e6ba7ed5cb5c7f77871e3d91e5ad51d7936c45e9c5a460d8a975601f70ea1" "$(cat "$sums_dir/digest")"
{
    echo "$(cat "$sums_dir/digest")  cargo-crap-x86_64-unknown-linux-gnu.tar.gz"
    echo "0000000000000000000000000000000000000000000000000000000000000000  cargo-crap-aarch64-apple-darwin.tar.gz"
} > "$sums_dir/SHA256SUMS"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/SHA256SUMS"
check "a listed digest passes" "0" "$?"
sed 's/$/\r/' "$sums_dir/SHA256SUMS" > "$sums_dir/crlf"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/crlf"
check "a sums file with CRLF line ends passes" "0" "$?"
{ cat "$sums_dir/SHA256SUMS"; cat "$sums_dir/SHA256SUMS"; } > "$sums_dir/twice"
verify_archive "$sums_dir/cargo-crap-x86_64-unknown-linux-gnu.tar.gz" "$sums_dir/twice"
check "a name listed twice passes" "0" "$?"
check "an archive that does not match the release's sums is refused" \
    "::error::cargo-crap-aarch64-apple-darwin.tar.gz does not match the digest in the release's SHA256SUMS" \
    "$(verify_archive "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" 2>&1)"
verify_archive "$sums_dir/cargo-crap-aarch64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" >/dev/null 2>&1
check "a mismatch fails" "1" "$?"
printf 'unlisted' > "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz"
check "an unlisted archive names itself" \
    "::error::the release's SHA256SUMS lists no cargo-crap-x86_64-apple-darwin.tar.gz" \
    "$(verify_archive "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" 2>&1)"
verify_archive "$sums_dir/cargo-crap-x86_64-apple-darwin.tar.gz" "$sums_dir/SHA256SUMS" >/dev/null 2>&1
check "an unlisted archive fails" "1" "$?"

# The shields badge's message counts the functions above the threshold.
check "none above" "0" "$(crappy_from_badge '{"message": "passing"}')"
check "some above" "12" "$(crappy_from_badge '{"label": "CRAP > 15", "message": "12 crappy"}')"

# Shell words from an input: quotes honoured, nothing executed.
words() {
    local out=() w
    while IFS= read -r w; do out+=("[$w]"); done < <(split_words "$1")
    echo "${out[*]-}"
}
check "plain words" "[--workspace] [--exclude] [benches/**]" "$(words "--workspace --exclude benches/**")"
check "quoted words" "[--exclude] [a b/**]" "$(words "--exclude 'a b/**'")"
check "double quotes" "[--exclude] [a b]" "$(words '--exclude "a b"')"
check "no words" "" "$(words "")"
check "only spaces" "" "$(words "   ")"
marker="$(mktemp -d)/ran"
# shellcheck disable=SC2016 # the literal `$(` is the point: it must survive unrun
check "nothing is executed" '[$(touch] ['"$marker"')]' "$(words "\$(touch $marker)")"
check "the command did not run" "absent" "$( [ -e "$marker" ] && echo present || echo absent)"
check "a backslash in quotes is kept" "[--exclude] [src\**]" "$(words "--exclude 'src\**'")"

# An input's words, or an error naming the input when they cannot be split.
check "an input's words" "--exclude
a b" "$(words_of args "--exclude 'a b'")"
check "no words of an empty input" "" "$(words_of args "")"
check "a stray quote prints no words" "" "$(words_of args "--threshold 20'" 2>/dev/null)"
check "a stray quote names its input" \
    "::error::args cannot be split into words: check its quotes" \
    "$(words_of args "--threshold 20'" 2>&1 >/dev/null)"
words_of args "--threshold 20'" >/dev/null 2>&1
check "a stray quote fails" "1" "$?"
words_file="$(mktemp)"
words_of args "--exclude ''" > "$words_file"
check "a trailing empty word survives" "2" "$(wc -l < "$words_file" | tr -d ' ')"
check "and it is empty" "" "$(sed -n 2p "$words_file")"

# A true/false input, and nothing else.
check "true" "true" "$(boolean fail-above true)"
check "false" "false" "$(boolean fail-above false)"
check "another spelling is refused" "" "$(boolean fail-above yes 2>/dev/null)"
boolean fail-above yes >/dev/null 2>&1
check "another spelling fails" "1" "$?"

# A count, and nothing else.
is_count 12
check "a count" "0" "$?"
is_count twelve
check "a word is not a count" "1" "$?"
is_count ""
check "nothing is not a count" "1" "$?"

# A path input names an existing file, or nothing.
present="$(mktemp)"
check "an existing file" "" "$(file_input lcov "$present" 2>&1)"
file_input lcov "$present"
check "an existing file passes" "0" "$?"
check "no value passes" "" "$(file_input lcov "" 2>&1)"
check "a missing file names its input" \
    "::error::lcov: no-such.info is not a file (paths are relative to the working directory)" \
    "$(file_input lcov no-such.info 2>&1)"
file_input lcov no-such.info >/dev/null 2>&1
check "a missing file fails" "1" "$?"

# A provider key reaches neither a program nor a shell function run keyless.
key_probe() { echo "${TYPESAFE_API_KEY-unset}/${OPENAI_API_KEY-unset}"; }
check "keyless unsets every key for a function" "unset/unset" \
    "$(TYPESAFE_API_KEY=a OPENAI_API_KEY=b keyless key_probe)"
# shellcheck disable=SC2016 # expanded by the inner shell, after keyless ran
check "keyless unsets every key for a program" "unset/unset" \
    "$(TYPESAFE_API_KEY=a OPENAI_API_KEY=b keyless bash -c 'echo "${TYPESAFE_API_KEY-unset}/${OPENAI_API_KEY-unset}"')"
# shellcheck disable=SC2016 # expanded by the inner shell
check "and leaves the caller's keys alone" "a" \
    "$(TYPESAFE_API_KEY=a bash -c 'source "$1"; keyless true; echo "$TYPESAFE_API_KEY"' _ "$here/lib.sh")"

# A report that tripped a gate is a report. Only a failure to run fails.
report_run true
check "a clean report passes" "0" "$?"
report_run bash -c 'exit 1'
check "a tripped gate passes" "0" "$?"
report_run bash -c 'exit 2'
check "a failed run fails with its status" "2" "$?"
report_run bash -c 'exit 101'
check "a crash fails with its status" "101" "$?"

# Gated signals that rose, from a delta report and its baseline.
signal_dir="$(mktemp -d)"
cat > "$signal_dir/head.json" <<'JSON'
{
  "entries": [
    {
      "function": "shipping",
      "signals": {
        "nesting": 2
      },
      "status": "regressed",
      "baseline_signals": {
        "nesting": 1
      },
      "reasons": [
        {
          "signal": "crap",
          "gating": true
        },
        {
          "signal": "nesting",
          "value": 2,
          "baseline": 1,
          "gating": true
        },
        {
          "signal": "lines",
          "gating": false
        },
        {
          "signal": "bumps",
          "gating": true
        }
      ]
    }
  ]
}
JSON
printf '{"entries":[{"function":"shipping","signals":{"nesting":1}}]}\n' > "$signal_dir/base.json"
printf '{"entries":[{"function":"shipping","crap":3.0}]}\n' > "$signal_dir/unmeasured.json"
check "gated signals count, a score regression does not" "2" \
    "$(signal_regressions_in "$signal_dir/head.json" "$signal_dir/base.json")"
check "a baseline without signals counts nothing" "" \
    "$(signal_regressions_in "$signal_dir/head.json" "$signal_dir/unmeasured.json")"
grep -v -e '"signals"' -e '"nesting": 2' "$signal_dir/head.json" > "$signal_dir/off.json"
check "a run without signals counts nothing" "" \
    "$(signal_regressions_in "$signal_dir/off.json" "$signal_dir/base.json")"
sed -e 's/"gating": true/"gating": false/' "$signal_dir/head.json" > "$signal_dir/none.json"
check "measured and none rose is zero" "0" \
    "$(signal_regressions_in "$signal_dir/none.json" "$signal_dir/base.json")"

# A comment marker is one line, without surrounding blanks.
check "the default marker is fine" "" "$(marker_problem '<!-- cargo-crap-report -->')"
check "an empty marker" "comment-marker is empty: it would match every comment on the pull request" \
    "$(marker_problem '')"
check "a marker with a newline" "comment-marker must be one line: it is matched as the comment's first line" \
    "$(marker_problem $'<!-- a -->\n<!-- b -->')"
check "a marker with a carriage return" "comment-marker must be one line: it is matched as the comment's first line" \
    "$(marker_problem $'<!-- a -->\r')"
check "a marker with a trailing space" "comment-marker must not start or end with a space" \
    "$(marker_problem '<!-- a --> ')"
check "a marker with a leading tab" "comment-marker must not start or end with a space" \
    "$(marker_problem $'\t<!-- a -->')"

# A path prefix in a link.
check "a plain prefix" "crates/shop" "$(url_path_encode crates/shop)"
check "a space" "crates/my%20shop" "$(url_path_encode 'crates/my shop')"
check "a hash and a percent" "a%23b%25c" "$(url_path_encode 'a#b%c')"
check "a multi-byte letter" "caf%C3%A9" "$(url_path_encode 'café')"

# The name a given binary is copied to.
check "on windows" "cargo-crap.exe" "$(binary_name Windows)"
check "elsewhere" "cargo-crap" "$(binary_name Linux)"

# Whether a config sets a display limit at its top level.
config_dir="$(mktemp -d)"
printf 'threshold = 20\nmin = 5\n' > "$config_dir/min.toml"
printf '  top=3\n' > "$config_dir/top.toml"
printf 'threshold = 20\n[signals]\nmin = 5\n' > "$config_dir/table.toml"
printf '# min = 5\nminimum = 1\n' > "$config_dir/none.toml"
config_limits "$config_dir/min.toml"
check "min" "0" "$?"
config_limits "$config_dir/top.toml"
check "top" "0" "$?"
config_limits "$config_dir/table.toml"
check "a key in a table is not a limit" "1" "$?"
config_limits "$config_dir/none.toml"
check "a comment and a longer key are not" "1" "$?"
config_limits "$config_dir/missing.toml"
check "no config sets none" "1" "$?"

# The flags of args the Action sets itself.
for flag in --lcov --format --output --baseline --commit-ref --all-rows --help --version; do
    check "$flag is owned" "$flag" "$(owned_flag "$flag")"
done
check "the = form is owned" "--format" "$(owned_flag --format=json)"
check "-h" "-h" "$(owned_flag -h)"
check "a cluster with V" "-V" "$(owned_flag -qV)"
check "a package named after h" "" "$(owned_flag -phV)"
check "a longer flag is not owned" "" "$(owned_flag --formatted)"
check "a value is not owned" "" "$(owned_flag src/**)"
check "a plain flag" "" "$(owned_flag --threshold)"

# Each run gets its own words of args.
partition_args --threshold 20 --fail-above --top 5 --min=2 --summary --show-unchanged --no-cache --exclude 'a b'
check "every run" "--threshold 20 --no-cache --exclude a b" "${common[*]}"
check "limits" "--top 5 --min=2" "${limits[*]}"
check "report only" "--summary" "${report_only[*]}"
check "compared only" "--show-unchanged" "${compared_only[*]}"
check "fail-above folds" "true" "$fold_fail_above"
check "fail-regression does not" "false" "$fold_fail_regression"
check "no-cache is seen" "true" "$has_no_cache"
partition_args --fail-regression --exclude ''
check "an empty word survives" "2" "${#common[@]}"
check "fail-regression folds" "true" "$fold_fail_regression"
check "no-cache is not seen" "false" "$has_no_cache"
check "an owned flag names itself" "::error::args: --output is set by the Action" \
    "$(partition_args --threshold 2 --output x 2>&1)"
partition_args --output x 2>/dev/null
check "an owned flag fails" "1" "$?"

# Absolute paths into the checkout, and nothing else.
checkout="$(mktemp -d)"
mkdir -p "$checkout/src"
touch "$checkout/src/lib.rs"
checkout="$(cd "$checkout" && pwd -P)"
names_checkout "$checkout/src" "$checkout"
check "a directory inside" "0" "$?"
names_checkout "--manifest-path=$checkout/src/lib.rs" "$checkout"
check "a file after =" "0" "$?"
names_checkout . "$checkout"
check "a relative path" "1" "$?"
names_checkout /vendor/ "$checkout"
check "a regex that is no path" "1" "$?"
names_checkout "$(dirname "$checkout")" "$checkout"
check "a directory outside" "1" "$?"
mkdir -p "${checkout}-sibling"
names_checkout "${checkout}-sibling" "$checkout"
check "a sibling with the same prefix" "1" "$?"
inside_dir /a/b /a
check "below" "0" "$?"
inside_dir /a /a
check "itself" "0" "$?"
inside_dir /ab /a
check "a prefix of the name is not below" "1" "$?"

# The Comment step's script, run from action.yml under node against a mock
# of the GitHub client.
if command -v node > /dev/null; then
    comment_test="$(mktemp -d)/comment.cjs"
    cat > "$comment_test" <<'JS'
const fs = require("fs");
const os = require("os");
const path = require("path");
const lines = fs.readFileSync(process.argv[2], "utf8").split("\n");
const start = lines.findIndex(l => l.trim() === "script: |") + 1;
const script = [];
for (const line of lines.slice(start)) {
  if (line.trim() && !line.startsWith("          ")) break;
  script.push(line.slice(10));
}
const run = new (Object.getPrototypeOf(async () => {}).constructor)(
  "github", "context", "core", "require", "process", script.join("\n"));
const failures = [];
const check = (name, expected, actual) => {
  const [e, a] = [JSON.stringify(expected), JSON.stringify(actual)];
  console.log(e === a ? `ok   ${name}` : `FAIL ${name}: expected ${e}, got ${a}`);
  if (e !== a) failures.push(name);
};
const refuse = status => Object.assign(new Error(`HTTP ${status}`), { status });
const DEFAULT = "<!-- cargo-crap-report -->";
async function comment({ report = `${DEFAULT}\n\nrows\n`, marker = DEFAULT, defaultToken = "false",
                         comments = [], user = "fixture-bot", viewer, create, update } = {}) {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "comment-")), "report.md");
  fs.writeFileSync(file, report);
  const calls = [];
  const said = { notice: [], warning: [] };
  const github = {
    rest: {
      users: { getAuthenticated: async () => { if (typeof user === "number") throw refuse(user); return { data: { login: user } }; } },
      issues: {
        listComments: () => {},
        updateComment: async args => { calls.push(["PATCH", args.comment_id, args.body]); if (update) throw refuse(update); },
        createComment: async args => { calls.push(["POST", args.issue_number, args.body]); if (create) throw refuse(create); },
      },
    },
    graphql: async () => { if (viewer === undefined || typeof viewer === "number") throw refuse(viewer ?? 403); return { viewer: { login: viewer } }; },
    paginate: async () => comments,
  };
  const core = { notice: m => said.notice.push(m), warning: m => said.warning.push(m) };
  const context = { repo: { owner: "o", repo: "r" }, serverUrl: "https://github.com", runId: 9 };
  const env = { REPORT: file, MARKER: marker, PR_NUMBER: "42", DEFAULT_TOKEN: defaultToken };
  let thrown = null;
  try { await run(github, context, core, require, { env }); } catch (error) { thrown = error.status; }
  return { calls, said, thrown };
}
const own = (id, body, login = "fixture-bot") => ({ id, body, user: { login, type: "Bot" } });
(async () => {
  let r = await comment({ defaultToken: "true", comments: [own(6, `${DEFAULT}\nold`, "other-bot[bot]"), own(7, `${DEFAULT}\nold`, "github-actions[bot]")] });
  check("the workflow token updates its own comment past another bot's", [["PATCH", 7]], r.calls.map(c => c.slice(0, 2)));
  r = await comment({ defaultToken: "true", comments: [own(6, `${DEFAULT}\nold`, "other-bot[bot]")] });
  check("another bot's comment with the marker is never updated", [["POST", 42]], r.calls.map(c => c.slice(0, 2)));
  r = await comment({ comments: [own(6, `${DEFAULT}\nold`, "other-bot[bot]"), own(7, `${DEFAULT}\r\nold`)] });
  check("another token finds its account through /user, and a CRLF first line matches", [["PATCH", 7]], r.calls.map(c => c.slice(0, 2)));
  r = await comment({ user: 403, viewer: "fixture-bot", comments: [own(7, `${DEFAULT}\nold`)] });
  check("then through the GraphQL viewer", [["PATCH", 7]], r.calls.map(c => c.slice(0, 2)));
  r = await comment({ user: 403, viewer: 403, comments: [own(6, `${DEFAULT}\nold`, "other-bot[bot]"), own(7, `${DEFAULT}\nold`)] });
  check("an account that cannot be told writes a new comment", [["POST", 42]], r.calls.map(c => c.slice(0, 2)));
  check("and says so", 1, r.said.notice.length);
  r = await comment({ user: 401, viewer: 401, create: 401 });
  check("a token GitHub refuses warns", [1, null], [r.said.warning.length, r.thrown]);
  r = await comment({ marker: "<!-- demo -->" });
  check("a custom marker takes the default marker's line", "<!-- demo -->\n\nrows\n", r.calls[0][2]);
  r = await comment({ report: "rows\n" });
  check("a report without a marker gets one", `${DEFAULT}\nrows\n`, r.calls[0][2]);
  const room = 65536 - DEFAULT.length - 1;
  r = await comment({ report: `${DEFAULT}\n${"x".repeat(room)}` });
  check("a body of 65,536 code units is posted whole", 65536, r.calls[0][2].length);
  r = await comment({ report: `${DEFAULT}\n${"x".repeat(room - 1)}\u{1F600}` });
  check("one surrogate pair over is replaced by a pointer", true, r.calls[0][2].startsWith(`${DEFAULT}\nThe cargo-crap report is too large`) && r.calls[0][2].includes("https://github.com/o/r/actions/runs/9"));
  check("and warns", 1, r.said.warning.length);
  r = await comment({ marker: "m".repeat(65536), report: "rows" });
  check("a marker with no room left writes nothing", [0, 1], [r.calls.length, r.said.warning.length]);
  r = await comment({ create: 422 });
  check("a 422 warns rather than fails", [1, null], [r.said.warning.length, r.thrown]);
  r = await comment({ create: 500 });
  check("a server error fails", 500, r.thrown);
  process.exit(failures.length ? 1 : 0);
})();
JS
    node "$comment_test" "$here/../../action.yml" || failures=$((failures + 1))
fi

# The score step end to end, on fixture repositories, when CARGO_CRAP_TEST_BIN
# names a cargo-crap to run it with. It runs the real `cargo llvm-cov`, so
# `just dev` leaves it out. CARGO_CRAP_OLD_BIN names a release without
# --all-rows for the fallback cases.
if [ -n "${CARGO_CRAP_TEST_BIN:-}" ]; then
    e2e_root="$(mktemp -d)"
    e2e_root="$(cd "$e2e_root" && pwd -P)"
    count=0
    # Run the install and score steps as the Action does, from directory
    # `$1` with binary `$2` and the CRAP_* variables in the environment.
    # Sets `outputs` (the step's outputs file), `log` and `step` (the score exit).
    step_in() {
        local dir="$1" bin="$2" temp
        count=$((count + 1))
        temp="$e2e_root/runner-$count"
        mkdir -p "$temp"
        outputs="$temp/outputs"
        log="$temp/log"
        : > "$outputs"
        : > "$temp/path"
        : > "$temp/summary"
        step=0
        (
            cd "$dir" || exit 2
            export RUNNER_TEMP="$temp" RUNNER_OS=macOS RUNNER_ARCH=ARM64 GITHUB_OUTPUT="$outputs" \
                GITHUB_PATH="$temp/path" GITHUB_STEP_SUMMARY="$temp/summary" BINARY="$bin" VERSION=unused
            "$here/install.sh" > "$log" 2>&1 || exit 2
            PATH="$(cat "$temp/path"):$PATH"
            "$here/score.sh"
        ) >> "$log" 2>&1 || step=$?
    }
    exists() { if [ -e "$1" ]; then echo yes; else echo no; fi; }
    output() { grep "^$1=" "$outputs" | tail -1 | cut -d= -f2-; }
    pr() { git -C "$1" rev-parse HEAD~1; }
    fixture() { bash "$here/fixture.sh" "$@"; }
    unset_inputs() {
        export CRAP_ARGS="" CRAP_COVERAGE_ARGS="" CRAP_FAIL_ABOVE=false CRAP_FAIL_REGRESSION=true \
            CRAP_BASE="" CRAP_LCOV="" CRAP_BASELINE="" CRAP_HEAD_SHA="" CRAP_COMMENT=false \
            CRAP_COMMENT_MARKER="<!-- cargo-crap-report -->" CRAP_PR_NUMBER=""
    }
    bin="$CARGO_CRAP_TEST_BIN"

    # A rise across a `min` in config is a regression, not a new function.
    unset_inputs
    fixture "$e2e_root/cross" regress
    printf 'min = 5\n' > "$e2e_root/cross/.cargo-crap.toml"
    CRAP_BASE="$(pr "$e2e_root/cross")" step_in "$e2e_root/cross" "$bin"
    check "e2e: a rise across min ran" "0" "$step"
    check "e2e: a rise across min is a regression" "1/1" "$(output status)/$(output regressed)"
    check "e2e: the base run recorded the function" "1" "$(grep -c '"shipping"' "$(dirname "$(output report)")/base.json")"
    CRAP_BASE="$(pr "$e2e_root/cross")" CRAP_ARGS="--min 5" step_in "$e2e_root/cross" "$bin"
    check "e2e: and across a min in args" "1/1" "$(output status)/$(output regressed)"
    if [ -n "${CARGO_CRAP_OLD_BIN:-}" ]; then
        CRAP_BASE="$(pr "$e2e_root/cross")" step_in "$e2e_root/cross" "$CARGO_CRAP_OLD_BIN"
        check "e2e: a release without --all-rows and a min in config fails" "1" "$step"
        check "e2e: and says why" "1" "$(grep -c 'has no --all-rows, and .* sets top or min' "$log")"
        rm "$e2e_root/cross/.cargo-crap.toml"
        CRAP_BASE="$(pr "$e2e_root/cross")" CRAP_ARGS="--min 5" step_in "$e2e_root/cross" "$CARGO_CRAP_OLD_BIN"
        check "e2e: a release without --all-rows still catches a min in args" "0/1/1" \
            "$step/$(output status)/$(output regressed)"
        check "e2e: and leaves the signal count unknown" "" "$(output signal-regressions)"
    fi

    # A gate in config, or in args, gates after the report is delivered.
    unset_inputs
    fixture "$e2e_root/gates" steady
    printf 'fail-above = true\nthreshold = 2\n' > "$e2e_root/gates/.cargo-crap.toml"
    CRAP_BASE="$(pr "$e2e_root/gates")" CRAP_COMMENT=true CRAP_PR_NUMBER=42 step_in "$e2e_root/gates" "$bin"
    check "e2e: a config gate lets the step finish" "0" "$step"
    check "e2e: and gates" "1/1/true" "$(output status)/$(output crappy)/$(output comment)"
    # shellcheck disable=SC2016 # the backticks are the report's
    check "e2e: the report is in the summary" "1" "$(grep -c '`shipping`' "$(dirname "$(output report)")/summary.md")"
    rm "$e2e_root/gates/.cargo-crap.toml"
    CRAP_ARGS="--threshold 2 --fail-above" CRAP_FAIL_ABOVE=true CRAP_COMMENT=true CRAP_PR_NUMBER=42 \
        step_in "$e2e_root/gates" "$bin"
    check "e2e: --fail-above in args and the input gate once" "0/1" "$step/$(output status)"
    check "e2e: a posted comment is the pr-comment report" "<!-- cargo-crap-report -->" "$(head -1 "$(output report)")"
    CRAP_ARGS="--fail-regression" step_in "$e2e_root/gates" "$bin"
    check "e2e: --fail-regression without a comparison is dormant" "0/0" "$step/$(output status)"
    check "e2e: and says so" "1" "$(grep -c 'fail-regression has no effect' "$log")"
    check "e2e: a push report is markdown" "1" "$(grep -c 'none exceed CRAP threshold' "$(output report)")"

    # Flags the Action owns, and presentation flags kept to the report.
    CRAP_ARGS="--format json" step_in "$e2e_root/gates" "$bin"
    check "e2e: an owned flag fails the step" "1" "$step"
    check "e2e: and names it" "1" "$(grep -c 'args: --format is set by the Action' "$log")"
    CRAP_BASE="$(pr "$e2e_root/gates")" CRAP_ARGS="--path . --summary --show-unchanged --no-cache" \
        step_in "$e2e_root/gates" "$bin"
    check "e2e: --summary, --show-unchanged and --no-cache pass" "0/0" "$step/$(output status)"
    check "e2e: and crappy is still counted" "0" "$(output crappy)"
    CRAP_BASE="$(pr "$e2e_root/gates")" CRAP_ARGS="--path $e2e_root/gates" step_in "$e2e_root/gates" "$bin"
    check "e2e: an absolute path into the checkout fails a base run" "1" "$step"

    # Signals: the full count, and none when a side was not measured.
    unset_inputs
    fixture "$e2e_root/signals" signal
    printf '[signals]\nenabled = true\n' > "$e2e_root/signals/.cargo-crap.toml"
    CRAP_BASE="$(pr "$e2e_root/signals")" CRAP_ARGS="--top 1" step_in "$e2e_root/signals" "$bin"
    check "e2e: a signal-only regression gates" "0/1/0/2" \
        "$step/$(output status)/$(output regressed)/$(output signal-regressions)"
    CRAP_BASE="$(pr "$e2e_root/signals")" CRAP_ARGS="--min 100" step_in "$e2e_root/signals" "$bin"
    check "e2e: a hidden signal regression is still counted" "1/0/2" \
        "$(output status)/$(output regressed)/$(output signal-regressions)"
    # Recorded without the config, so without signals.
    (
        cd "$e2e_root/signals" && mv .cargo-crap.toml "$e2e_root/signals.toml" && git checkout -q HEAD~1 &&
            "$bin" --path . --format json --output plain.json 2> /dev/null &&
            git checkout -q - && mv "$e2e_root/signals.toml" .cargo-crap.toml
    )
    CRAP_BASELINE=plain.json step_in "$e2e_root/signals" "$bin"
    check "e2e: a baseline without signals counts none" "0//0" \
        "$(output status)/$(output signal-regressions)/$(output regressed)"
    printf '' > "$e2e_root/signals/.cargo-crap.toml"
    CRAP_BASE="$(pr "$e2e_root/signals")" step_in "$e2e_root/signals" "$bin"
    check "e2e: signals off counts none" "0/" "$(output status)/$(output signal-regressions)"

    # The caller's checkout survives the base run untouched.
    unset_inputs
    fixture "$e2e_root/state" base-only
    (
        cd "$e2e_root/state" || exit 1
        git switch -q -c feature
        echo "// staged" >> src/lib.rs
        git add src/lib.rs
        echo "// unstaged" >> src/lib.rs
        echo "local" > src/old.rs
        git worktree add -q --detach "$e2e_root/sibling" HEAD
    )
    CRAP_BASE="$(pr "$e2e_root/state")" step_in "$e2e_root/state" "$bin"
    check "e2e: the comparison ran" "0/0" "$step/$(output status)"
    check "e2e: the branch is kept" "feature" "$(git -C "$e2e_root/state" symbolic-ref --short HEAD)"
    check "e2e: the staged edit is kept" "1" "$(git -C "$e2e_root/state" diff --cached | grep -c '^+// staged')"
    check "e2e: the unstaged edit is kept" "// unstaged" "$(tail -n 1 "$e2e_root/state/src/lib.rs")"
    check "e2e: an untracked file the base tracks is kept" "local" "$(cat "$e2e_root/state/src/old.rs")"
    check "e2e: the sibling worktree is kept, and no other" "2" "$(git -C "$e2e_root/state" worktree list | wc -l | tr -d ' ')"
    check "e2e: the sibling's files are kept" "yes" "$(exists "$e2e_root/sibling/src/lib.rs")"

    unset_inputs
    fixture "$e2e_root/broken" broken-base
    ( cd "$e2e_root/broken" && git switch -q -c feature && echo "// staged" >> src/lib.rs && git add src/lib.rs )
    CRAP_BASE="$(pr "$e2e_root/broken")" step_in "$e2e_root/broken" "$bin"
    check "e2e: a broken base fails the step" "1" "$([ "$step" -ne 0 ] && echo 1)"
    check "e2e: and keeps the branch and the index" "feature/1" \
        "$(git -C "$e2e_root/broken" symbolic-ref --short HEAD)/$(git -C "$e2e_root/broken" diff --cached | grep -c '^+// staged')"
    check "e2e: and removes its worktree" "1" "$(git -C "$e2e_root/broken" worktree list | wc -l | tr -d ' ')"

    unset_inputs
    fixture "$e2e_root/sub" submodule
    CRAP_BASE="$(pr "$e2e_root/sub")" step_in "$e2e_root/sub" "$bin"
    check "e2e: a submodule that cannot be fetched fails the step" "1" "$step"
    check "e2e: and says so" "1" "$(grep -c "base commit's submodules" "$log")"
    check "e2e: and removes its worktree" "1" "$(git -C "$e2e_root/sub" worktree list | wc -l | tr -d ' ')"

    # A base commit whose crate directory leads out of the worktree.
    unset_inputs
    mkdir -p "$e2e_root/outside"
    echo 'threshold = 1' > "$e2e_root/outside/.cargo-crap.toml"
    fixture "$e2e_root/escape" steady crates/shop
    (
        cd "$e2e_root/escape" || exit 1
        git checkout -q HEAD~1
        git rm -q -r crates/shop
        mkdir -p crates
        ln -s "$e2e_root/outside" crates/shop
        git add crates/shop
        git -c user.name=f -c user.email=f@e commit -q -m escape
        git rev-parse HEAD > "$e2e_root/escape.sha"
        git checkout -q -
    )
    CRAP_BASE="$(cat "$e2e_root/escape.sha")" step_in "$e2e_root/escape/crates/shop" "$bin"
    check "e2e: a symlinked base directory that leaves the worktree fails" "1" "$step"
    check "e2e: and nothing outside is removed" "threshold = 1" "$(cat "$e2e_root/outside/.cargo-crap.toml")"

    # A working directory reached through a symlink, and a target directory
    # of the caller's.
    unset_inputs
    fixture "$e2e_root/real" steady
    ln -s "$e2e_root/real" "$e2e_root/link"
    CARGO_TARGET_DIR="$e2e_root/tgt" CRAP_BASE="$(pr "$e2e_root/real")" step_in "$e2e_root/link" "$bin"
    check "e2e: through a symlink, with CARGO_TARGET_DIR" "0/0/0" "$step/$(output status)/$(output regressed)"
    check "e2e: both runs built in that target directory" "yes/no" \
        "$(exists "$e2e_root/tgt/llvm-cov-target")/$(exists "$e2e_root/real/target")"

    # Provider keys reach no build script and no test.
    unset_inputs
    fixture "$e2e_root/keys" key-probe
    KEY_PROBE="$e2e_root/keys.seen" TYPESAFE_API_KEY=not-a-key OPENAI_API_KEY=not-a-key \
        CRAP_BASE="$(pr "$e2e_root/keys")" step_in "$e2e_root/keys" "$bin"
    check "e2e: the key probe ran" "0" "$step"
    check "e2e: the project never saw a key" "0" "$(grep -c '^set$' "$e2e_root/keys.seen")"
    check "e2e: on both commits" "1" "$([ "$(grep -c '^unset$' "$e2e_root/keys.seen")" -ge 4 ] && echo 1)"

    # Links reach a crate whose directory has a space.
    unset_inputs
    fixture "$e2e_root/space" steady "crates/my shop"
    GITHUB_SERVER_URL=https://github.com GITHUB_REPOSITORY=o/r CRAP_HEAD_SHA=abc \
        step_in "$e2e_root/space/crates/my shop" "$bin"
    check "e2e: a link with a space" "1" "$([ "$(grep -c '/blob/abc/crates/my%20shop/' "$(output report)")" -ge 1 ] && echo 1)"

    # A given binary is copied under its own name.
    cp "$bin" "$e2e_root/crap-under-test"
    printf '#!/bin/sh\nexit 3\n' > "$e2e_root/cargo-crap"
    chmod +x "$e2e_root/cargo-crap"
    step_in "$e2e_root/real" "$e2e_root/crap-under-test"
    check "e2e: a renamed binary is used, not its sibling" "0/0" "$step/$(output status)"
fi

if [ "$failures" -gt 0 ]; then
    echo "$failures failed"
    exit 1
fi
echo "all passed"
