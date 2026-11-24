import json
from pathlib import Path
from support import ActionTest, BINARY, SCRIPTS, VERSION
from test_baseline import function, coverage


class Scores(ActionTest):
    def setUp(self):
        super().setUp()
        self.env.update(CRAPTS_PR_NUMBER="42", CRAPTS_REVISION="head-revision", GITHUB_SHA="push-revision",
                        GITHUB_REPOSITORY="owner/repo", GITHUB_SERVER_URL="https://github.com", INPUT_COVERAGE="hits.info")

    def score(self, args=(), **env):
        (self.run_dir / "args.json").write_text(json.dumps(args))
        self.prepare(**env)
        self.stage("baseline", **env)
        result = self.stage("score", **env)
        out = dict(line.split("=", 1) for line in Path(self.env["GITHUB_OUTPUT"]).read_text().splitlines())
        report = Path(out["report"]).read_text()
        data = json.loads((self.run_dir / "gate.json").read_text())
        return out, report, data, result.stdout

    def baseline(self, names, **changes):
        (self.project / "old.info").write_text(coverage({name: 1 for name in names}))
        result = self.command([str(BINARY), *[name + ".ts" for name in names], "--coverage", "old.info",
                               "--format", "json", "--all-rows"])
        doc = json.loads(result.stdout)
        doc.update(changes)
        (self.project / "saved.json").write_text(json.dumps(doc))
        self.env["INPUT_BASELINE"] = "saved.json"

    def comparison(self, additions):
        base_files = {"fee.ts": function("fee", 8)}
        head = {name + ".ts": function(name, 6) for name in additions}
        self.env["INPUT_BASE"] = self.history(base_files, head)
        (self.project / "hits.info").write_text(coverage({name: 0 for name in ["fee", *additions]}))
        self.baseline(["fee"])

    def test_gate_population_with_display_limits(self):
        self.comparison(["fresh", "other"])
        for args in [["--top", "1"], ["--min", "200"], ["--top", "23", "--min", "71"]]:
            result, report, doc, _ = self.score(args)
            self.assertEqual([result[k] for k in ["crappy", "regressed", "changed", "status"]], ["3", "1", "2", "1"])
            self.assertTrue(report.startswith("<!-- crapts-report -->\n"))
            self.assertIsNone(doc["selection"])
            self.assertEqual(len(doc["entries"]), 3)
            calls = self.calls()[-3:]
            self.assertIn("--print-config", calls[0]["args"])
            self.assertIn("pr-comment", calls[1]["args"])
            self.assertIn("json", calls[2]["args"])
            self.assertIn("--all-rows", calls[2]["args"])
            for call in calls[1:]:
                self.assertIn("--baseline", call["args"])
                self.assertIn("--base", call["args"])
            self.assertIn(report, Path(self.env["GITHUB_STEP_SUMMARY"]).read_text())

    def test_gate_switches(self):
        self.comparison([])
        for regression, above, config, flags, status in [
                ("true", "false", "", ["--fail-regression", "--fail-changed"], "1"),
                ("false", "false", "", ["--fail-changed"], "0"),
                ("false", "true", "", ["--fail-changed", "--fail-above"], "1"),
                ("false", "false", "fail-above=true", ["--fail-changed"], "1")]:
            (self.project / ".crapts.toml").write_text(config)
            out, _, _, _ = self.score(INPUT_FAIL_REGRESSION=regression, INPUT_FAIL_ABOVE=above)
            actual = [flag for flag in self.calls()[-1]["args"] if flag.startswith("--fail-")]
            self.assertCountEqual(actual, flags)
            self.assertEqual([out[k] for k in ["crappy", "regressed", "changed", "status"]], ["1", "1", "0", status])

    def test_changed_verdicts(self):
        root = self.project
        def body(cc, name="fee", last="return n"):
            return f"export function {name}(n: number) {{\n" + "if (n) n--;\n" * (cc - 1) + f"{last};\n}}\n"
        def hook(cc):
            return "useEffect(() => { " + "if (n) n--; " * (cc - 1) + "});\n"
        for i, (old, new, measure, policy, fail, skipped, null, score) in enumerate([
                ("", body(12), "zero", "", 1, 0, 0, 156), ("", body(5), "zero", "", 0, 0, 0, 30),
                (body(10), body(12), "quarter", "", 1, 0, 0, 72.75),
                (body(12), body(12, last="return n+1"), "zero", "", 0, 0, 0, 156),
                (hook(5) + hook(15), hook(12), "zero", "", 1, 0, 0, 156),
                ("", body(12), "missing", "pessimistic", 1, 0, 1, 156),
                ("", body(12), "missing", "optimistic", 0, 0, 1, 12),
                ("", body(12), "missing", "skip", 0, 1, 0, None)]):
            self.project = root / str(i)
            self.project.mkdir()
            self.env["INPUT_BASE"] = self.history({"fee.ts": old} if old else {}, {"fee.ts": new})
            path = self.project / "hits.info"
            path.write_text("SF:other.ts\nDA:1,1\nend_of_record\n" if measure == "missing" else
                            "SF:fee.ts\n" + "".join(f"DA:{n},{int(measure == 'quarter' and 2 <= n <= 4)}\n" for n in range(1, len(new.splitlines()) + 1)) + "end_of_record\n")
            out, _, doc, log = self.score(["--missing", policy] if policy else [])
            self.assertIn("no base coverage and no baseline: only the diff gate ran", log)
            self.assertEqual((out["changed"], out["regressed"], out["status"]), (str(fail), "", str(fail)), i)
            counts = doc["changed"]["counts"]
            self.assertEqual((counts["missing_skipped"], counts["coverage_null"]), (skipped, null))
            if score is not None:
                self.assertEqual(doc["entries"][0]["crap"], score, i)
            if i == 4:
                self.assertEqual(counts["ambiguous"], 1)
            if i == 0:
                out, _, _, _ = self.score(INPUT_FAIL_CHANGED="false")
                self.assertEqual((out["crappy"], out["regressed"], out["changed"], out["status"]), ("1", "", "1", "0"))
                self.assertFalse(any(a.startswith("--fail-") for a in self.calls()[-1]["args"]))

    def test_quoted_arguments_reach_the_executable(self):
        (self.project / "fee.ts").write_text(function("fee", 2))
        (self.project / "hits.info").write_text(coverage({"fee": 1}))
        result = self.command(["node", str(SCRIPTS / "inputs.mjs")], INPUT_ARGS='"two words" "*.ts" ""')
        (self.run_dir / "args.json").write_text(result.stdout)
        self.prepare()
        self.stage("baseline")
        self.stage("score", 2)
        self.assertEqual(self.calls()[0]["args"][:3], ["two words", "*.ts", ""])

    def test_push_baseline(self):
        self.history({"fee.ts": function("fee", 2)}, {})
        (self.project / "hits.info").write_text(coverage({"fee": 0}))
        for writer in [VERSION, "0.0.9"]:
            self.baseline(["fee"], version=writer)
            out, report, _, _ = self.score(CRAPTS_PR_NUMBER="")
            self.assertEqual((out["regressed"], out["changed"], out["crappy"]), ("1", "", "0"))
            self.assertEqual(out["status"], "1" if writer == VERSION else "0")
            self.assertIn("fee", report)
            self.assertIn("markdown", self.calls()[-2]["args"])
            if writer != VERSION:
                self.assertTrue(report.startswith("<!-- crapts-report -->\n> **Note:**"))
                self.assertIn("regression gate skipped: baseline from crapts " + writer, report)

    def test_new_project_directory(self):
        self.env["INPUT_BASE"] = self.history({}, {"packages/shop/fee.ts": function("fee", 6)})
        self.project = self.project / "packages/shop"
        self.env.update(INPUT_COVERAGE="", INPUT_COVERAGE_FILE="hits.info", INPUT_COVERAGE_COMMAND="echo 'SF:fee.ts\nDA:1,0\nend_of_record' > hits.info; echo called >> trace")
        self.prepare()
        base = self.stage("baseline")
        self.assertIn("packages/shop", base.stdout.replace("\\", "/"))
        self.stage("score")
        self.assertEqual((self.project / "trace").read_text().splitlines(), ["called"])
        out = dict(line.split("=", 1) for line in Path(self.env["GITHUB_OUTPUT"]).read_text().splitlines())
        self.assertEqual((out["regressed"], out["changed"], out["status"]), ("", "1", "1"))

    def test_partial_baseline_stops_reporting(self):
        self.comparison(["fresh"])
        sliced = self.command([str(BINARY), "--format", "json", "--top", "5", "--coverage", "hits.info"]).stdout
        (self.project / "top5.json").write_text(sliced)
        self.prepare()
        self.stage("baseline", INPUT_BASELINE="top5.json")
        result = self.stage("score", 2)
        self.assertIn("top5.json", result.stderr)
        self.assertRegex(result.stderr.lower(), "not a complete baseline")
        self.assertFalse(Path(self.env["GITHUB_OUTPUT"]).exists())
        self.assertFalse(Path(self.env["GITHUB_STEP_SUMMARY"]).exists())
