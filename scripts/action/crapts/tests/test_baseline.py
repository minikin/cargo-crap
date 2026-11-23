import json
from pathlib import Path
from support import ActionTest, BINARY


def function(name, cc):
    return f"export function {name}(n: number) {{ " + "if (n) n--; " * (cc - 1) + "return n; }\n"


def coverage(hits):
    return "".join(f"SF:{name}.ts\nDA:1,{count}\nend_of_record\n" for name, count in hits.items())


class Baselines(ActionTest):
    def project_pair(self, config=None):
        prefix = "packages/shop/"
        common = {prefix + name + ".ts": function(name, cc) for name, cc in [("fee", 8), ("steady", 2), ("probe", 1)]}
        common.update({prefix + "lock": "common\n", prefix + ".crapts.toml": 'exclude=["fee.ts"]\n',
                       prefix + "tests/fee.test.ts": "export {};\n"})
        head = {prefix + "lock": "head\n", prefix + ".crapts.toml": None, prefix + "tests/fee.test.ts": None}
        if config == "tracked":
            head["packages/.crapts.toml"] = 'exclude=["shop/probe.ts"]\n'
        self.base = self.history(common, head, base_changes={prefix + "lock": "base\n"})
        self.project = self.project / prefix
        if config == "untracked":
            (self.project.parent / ".crapts.toml").write_text('exclude=["shop/probe.ts"]\n')
        (self.project / "node_modules").mkdir()
        (self.project / "node_modules/proof").write_text("head dependencies")
        self.trace = self.temp / "commands"
        self.env.update(INPUT_BASE=self.base, TRACE=str(self.trace), INPUT_COVERAGE_FILE="coverage/coverage-final.json",
                        INPUT_COVERAGE="coverage/coverage-final.json", INPUT_INSTALL_COMMAND="""
echo "install $(cat lock) $(git rev-parse HEAD)" >> "$TRACE"
mkdir -p node_modules && cp lock node_modules/installed
""", INPUT_COVERAGE_COMMAND="""
set -eu
echo "cover $(cat node_modules/installed) $(git rev-parse HEAD)" >> "$TRACE"
test ! -e coverage/coverage-final.json
mkdir -p coverage
if test -f tests/fee.test.ts; then hits=1; else hits=0; fi
for name in fee steady probe; do
  if test "$name" = fee; then count=$hits; else count=1; fi
  printf 'SF:%s.ts\nDA:1,%s\nend_of_record\n' "$name" "$count"
done > coverage/coverage-final.json
""")
        self.head_coverage = self.project / self.env["INPUT_COVERAGE"]
        self.head_coverage.parent.mkdir()
        self.head_coverage.write_text(coverage({"fee": 0, "steady": 1, "probe": 1}))

    def generate(self, config, existing):
        self.project_pair(config)
        edited = (self.project / "steady.ts").read_bytes() + b"\n"
        (self.project / "steady.ts").write_bytes(edited)
        (self.project / "keep").write_bytes(b"untracked")
        if not existing:
            self.env["INPUT_COVERAGE"] = ""
        head = self.git("rev-parse", "HEAD")
        git_state = self.git("worktree", "list", "--porcelain")
        args = ["--top", "1", "--fail-above", "--fail-changed", "--fail-regression"]
        (self.run_dir / "args.json").write_text(json.dumps(args))
        self.prepare()
        self.stage("baseline")
        state = self.prepared()
        doc = json.loads(Path(state["baseline"]).read_text())
        expected = {"fee": 8, "steady": 2}
        if config is None:
            expected["probe"] = 1
        self.assertEqual({row["function"]: row["crap"] for row in doc["entries"]}, expected)
        self.assertIsNone(doc["selection"])
        calls = self.calls()
        self.assertEqual(len(calls), 1)
        self.assertNotIn("--base", calls[0]["args"])
        self.assertNotIn("--baseline", calls[0]["args"])
        self.assertFalse(any(arg.startswith("--fail-") for arg in calls[0]["args"]))
        for arg in ["--top", "1", "--all-rows", "--format", "json"]:
            self.assertIn(arg, calls[0]["args"])
        self.assertFalse(Path(calls[0]["cwd"]).exists())
        self.assertEqual(self.git("worktree", "list", "--porcelain"), git_state)
        self.assertEqual(self.git("rev-parse", "HEAD"), head)
        self.assertEqual((self.project / "node_modules/proof").read_text(), "head dependencies")
        self.assertEqual((self.project / "steady.ts").read_bytes(), edited)
        self.assertEqual((self.project / "keep").read_bytes(), b"untracked")
        expected_trace = [] if existing else [f"install head {head}", f"cover head {head}"]
        self.assertEqual(self.trace.read_text().splitlines(), expected_trace + [f"install base {self.base}", f"cover base {self.base}"])
        scored = self.command([str(BINARY), "--format", "json", "--all-rows", "--base", self.base, "--baseline",
                               state["baseline"], "--coverage", state["coverage"]])
        report = json.loads(scored.stdout)
        self.assertEqual(report["summary"]["regressed"], 1)
        self.assertEqual(report["summary"]["changed_failures"], 0)
        self.assertEqual(next(row["crap"] for row in report["entries"] if row["function"] == "fee"), 72)

    def test_tracked_config(self):
        self.generate("tracked", True)

    def test_untracked_config(self):
        self.generate("untracked", True)

    def test_no_config(self):
        self.generate(None, True)

    def test_generate_both(self):
        self.generate(None, False)

    def test_absolute_coverage_file_in_project_is_relocated_at_base(self):
        self.project_pair()
        original = self.head_coverage.read_bytes()
        self.env["INPUT_COVERAGE_FILE"] = str(self.head_coverage)
        self.prepare()
        self.stage("baseline")
        self.assertEqual(self.head_coverage.read_bytes(), original)
        doc = json.loads(Path(self.prepared()["baseline"]).read_text())
        self.assertEqual(next(row["crap"] for row in doc["entries"] if row["function"] == "fee"), 8)

    def test_base_config_gate_keeps_a_complete_report(self):
        self.project_pair()
        (self.project / ".crapts.toml").write_text("threshold=0\nfail-above=true\n")
        self.prepare()
        self.stage("baseline")
        doc = json.loads(Path(self.prepared()["baseline"]).read_text())
        self.assertEqual(doc["summary"]["crappy"], 3)
        self.assertEqual(len(doc["entries"]), 3)

    def test_base_failures_restore_head(self):
        self.project_pair()
        (self.project / "fee.ts").write_text(function("fee", 8) + "\n")
        (self.project / "untracked").write_text("keep")
        before = self.git("worktree", "list", "--porcelain")
        self.prepare()
        for commands, diagnostic in [
                ({"INPUT_INSTALL_COMMAND": "exit 5"}, "install command.*status 5"),
                ({"INPUT_COVERAGE_COMMAND": "touch extra; exit 7"}, "coverage command.*status 7"),
                ({"INPUT_COVERAGE_COMMAND": "true"}, "coverage command.*coverage-file"),
                ({"INPUT_COVERAGE_COMMAND": "mkdir -p coverage; echo wrong > coverage/coverage-final.json"}, "crapts.*status 2")]:
            with self.subTest(commands=commands):
                result = self.stage("baseline", 2, **commands)
                self.assertRegex(result.stderr, diagnostic)
                self.assertIn(self.base, result.stderr)
                self.assertEqual(self.git("worktree", "list", "--porcelain"), before)
                self.assertFalse((self.run_dir / "base-worktree").exists())
                self.assertEqual((self.project / "fee.ts").read_text(), function("fee", 8) + "\n")
                self.assertEqual((self.project / "untracked").read_text(), "keep")
                self.assertEqual((self.project / "node_modules/proof").read_text(), "head dependencies")

    def test_saved_baseline_bypasses_generation(self):
        self.project_pair()
        saved = self.command([str(BINARY), "--coverage", str(self.head_coverage), "--format", "json", "--all-rows"]).stdout
        (self.project / "saved.json").write_bytes(saved.encode())
        for base in ["", self.base]:
            self.prepare(INPUT_BASE=base)
            trace = self.temp / "git.trace"
            self.stage("baseline", INPUT_BASELINE="saved.json", GIT_TRACE=str(trace))
            self.assertEqual(Path(self.prepared()["baseline"]).read_bytes(), saved.encode())
            self.assertFalse(self.trace.exists())
            self.assertFalse(trace.exists() and "worktree add" in trace.read_text())
