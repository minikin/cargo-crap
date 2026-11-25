import json
from pathlib import Path
from support import ActionTest, BINARY, SCRIPTS, VERSION
from test_baseline import function, coverage
import scenarios


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

    def generated(self, base_files, head_changes, lines, **env):
        """A PR whose base is scored by a real base run of the coverage command."""
        self.env["INPUT_BASE"] = self.history(base_files, head_changes)
        self.trace = self.temp / "commands"
        names = " ".join(sorted({Path(name).stem for name in {**base_files, **head_changes}}))
        # ${VAR+key} prints a marker, never the value.
        self.env.update(INPUT_COVERAGE="", INPUT_COVERAGE_FILE="coverage.info", TRACE=str(self.trace), **env,
                        INPUT_COVERAGE_COMMAND=f"""
echo "${{TYPESAFE_API_KEY+key}}${{typesAfe_api_key+key}}${{OPENAI_API_KEY+key}}${{OpenAI_Api_Key+key}}." >> "$TRACE"
for name in {names}; do
  printf 'SF:%s.ts\\n' "$name"; for line in $(seq 1 {lines}); do printf 'DA:%s,1\\n' "$line"; done; echo end_of_record
done > coverage.info
""")

    def test_only_the_report_run_carries_provider_keys(self):
        self.generated({"fee.ts": function("fee", 8)}, {"fresh.ts": function("fresh", 6)}, 1)
        poisoned = dict(TYPESAFE_BASE_URL="http://written-to-github-env.invalid", openai_base_url="http://x.invalid")
        (self.run_dir / "trusted.json").write_text(json.dumps(poisoned))
        for spelling in ["TYPESAFE_API_KEY", "typesAfe_api_key"]:
            with self.subTest(spelling=spelling):
                self.trace.unlink(missing_ok=True)
                before = len(self.calls())
                self.score(**{spelling: "not-a-key", "openAI_api_key": "not-a-key"}, **poisoned,
                           CRAPTS_TRUSTED_TYPESAFE_BASE_URL="http://install-time.invalid")
                calls = self.calls()[before:]
                self.assertEqual(len(calls), 4)
                self.assertIn("--all-rows", calls[0]["args"])
                self.assertNotIn("--base", calls[0]["args"])
                report = [call for call in calls if "pr-comment" in call["args"]]
                self.assertEqual(len(report), 1)
                self.assertCountEqual([name.upper() for name in report[0]["keys"]], ["TYPESAFE_API_KEY", "OPENAI_API_KEY"])
                self.assertEqual(report[0]["urls"], {"TYPESAFE_BASE_URL": "http://install-time.invalid"})
                for call in calls:
                    if call is not report[0]:
                        self.assertEqual(call["keys"], [], call["args"])
                self.assertEqual(self.trace.read_text().splitlines(), [".", "."])
        self.assertNotIn("not-a-key", Path(self.env["ARG_TRACE"]).read_text())

    def test_trusted_urls_are_read_case_insensitively(self):
        result = self.command(["node", str(SCRIPTS / "cli.mjs"), "trusted-urls"],
                              openai_base_url="http://a.invalid", TYPESAFE_API_KEY="not-a-key")
        outputs = dict(line.split("=", 1) for line in result.stdout.splitlines())
        self.assertEqual(outputs["OPENAI_BASE_URL"], "http://a.invalid")
        self.assertLessEqual(outputs.keys(), {"TYPESAFE_BASE_URL", "OPENAI_BASE_URL"})

    def test_a_signal_only_regression_fails_with_zero_counts(self):
        self.env.update(INPUT_BASE=scenarios.signals_history(self.project), INPUT_COVERAGE="",
                        INPUT_COVERAGE_COMMAND="bash cover.sh", INPUT_COVERAGE_FILE="coverage.info")
        out, report, doc, _ = self.score()
        self.assertEqual([out[k] for k in ["status", "crappy", "regressed", "changed", "signal-regressions"]],
                         ["1", "0", "0", "0", "1"])
        self.assertEqual(doc["summary"]["signal_regressions"], 1)
        self.assertIn("`f`", report)
        for signals, baseline, label in [(True, False, "unavailable"), (False, True, "off")]:
            with self.subTest(label):
                (self.project / ".crapts.toml").write_text("[signals]\nenabled = true\n" if signals else "")
                env = {} if baseline else {"INPUT_COVERAGE_COMMAND": "", "INPUT_COVERAGE": "coverage.info"}
                out, _, doc, _ = self.score(**env)
                self.assertEqual(out["signal-regressions"], "")
                if signals:
                    self.assertIsNone(doc["summary"]["signal_regressions"])
                else:
                    self.assertNotIn("signal_regressions", doc["summary"])

    def test_coverage_free_duplicate_scan(self):
        self.env.update(INPUT_COVERAGE="", INPUT_BASE=scenarios.duplicates_history(self.project))
        out, report, doc, _ = self.score(["--duplicates"])
        self.assertEqual([out[k] for k in ["status", "crappy", "regressed", "changed"]], ["0", "0", "", "0"])
        self.assertEqual(doc["coverage_kind"], "none")
        self.assertEqual(len(doc["duplicate_groups"]), 1)
        self.assertIn("No coverage supplied", report)
        scenarios.main("duplicates-check", out["report"])
        for call in self.calls():
            self.assertNotIn("--coverage", call["args"])
            self.assertIn("--base", call["args"])

    def test_comparison_flags_without_their_comparison_stay_dormant(self):
        self.history({"fee.ts": function("fee", 2), "--show-unchanged/odd.ts": function("odd", 2)}, {})
        (self.project / "hits.info").write_text(coverage({"fee": 0}))
        args = ["fee.ts", "--show-unchanged", "--fail-regression", "--fail-changed", "--", "--show-unchanged"]
        out, _, _, log = self.score(args, CRAPTS_PR_NUMBER="")
        self.assertEqual(out["status"], "0")
        for flag, needs in [("--show-unchanged", "a baseline"), ("--fail-regression", "a baseline"),
                            ("--fail-changed", "a base")]:
            self.assertIn(f"::notice::args: {flag} needs {needs}, and this run has none", log)
        for call in self.calls():
            # Only the positional path after -- survives.
            self.assertEqual(call["args"][call["args"].index("--"):], ["--", "--show-unchanged"])
            self.assertEqual(call["args"].count("--show-unchanged"), 1)
        self.baseline(["fee"])
        before = len(self.calls())
        _, _, _, log = self.score(["fee.ts", "--show-unchanged"], CRAPTS_PR_NUMBER="")
        self.assertNotIn("::notice::args:", log)
        for call in self.calls()[before:]:
            self.assertIn("--show-unchanged", call["args"])
