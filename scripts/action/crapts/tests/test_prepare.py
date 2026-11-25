import json
from pathlib import Path
import re
import subprocess
from support import ActionTest, BINARY


class HeadCoverage(ActionTest):
    def test_existing_lcov(self):
        self.project = self.project / "packages/shop"
        self.project.mkdir(parents=True)
        (self.project / "fee.ts").write_text("export function fee(b: boolean) { return b ? 1 : 0 }\n")
        (self.project / "hits.info").write_text("SF:fee.ts\nDA:1,1\nend_of_record\n")
        self.prepare(INPUT_COVERAGE="hits.info", INPUT_COVERAGE_COMMAND="exit 7", INPUT_INSTALL_COMMAND="exit 5")
        coverage = Path(self.prepared()["coverage"])
        self.assertTrue(coverage.is_relative_to(self.run_dir))
        report = json.loads(self.command([str(BINARY), "--coverage", str(coverage), "--format", "json"]).stdout)
        self.assertEqual(report["entries"][0]["crap"], 2)

    def test_fresh_istanbul(self):
        output = self.project / "out/head.json"
        output.parent.mkdir()
        output.write_text("stale")
        (self.project / "fee.ts").write_text("export function fee(b: boolean) { return b ? 1 : 0 }\n")
        location = {"start": {"line": 1, "column": 34}, "end": {"line": 1, "column": 50}}
        generated = {"fee.ts": {"path": "fee.ts", "statementMap": {"0": location}, "s": {"0": 1}}}
        (self.project / "fresh.json").write_text(json.dumps(generated))
        self.prepare(INPUT_COVERAGE_FILE="out/head.json", INPUT_INSTALL_COMMAND="echo install > trace",
                     INPUT_COVERAGE_COMMAND="""test ! -e out/head.json && echo coverage >> trace && cp fresh.json out/head.json""")
        self.assertEqual((self.project / "trace").read_text().splitlines(), ["install", "coverage"])
        self.assertEqual(Path(self.prepared()["coverage"]).read_bytes(), output.read_bytes())
        result = self.command([str(BINARY), "--coverage", self.prepared()["coverage"], "--format", "json"])
        self.assertEqual(json.loads(result.stdout)["entries"][0]["crap"], 2)

    def test_head_errors(self):
        for env, expected in [({}, "set coverage or coverage-command"),
                              ({"INPUT_COVERAGE": "missing.info", "INPUT_COVERAGE_COMMAND": "exit 7"}, "coverage.*missing.info"),
                              ({"INPUT_COVERAGE_COMMAND": "exit 7"}, "head.*coverage command.*status 7"),
                              ({"INPUT_COVERAGE_COMMAND": "true", "INPUT_COVERAGE_FILE": "out/new.json"},
                               "coverage-file.*out/new.json")]:
            with self.subTest(env=env):
                self.assertRegex(self.prepare(2, **env).stderr, expected)
                self.assertFalse((self.run_dir / "prepared.json").exists())

    def test_duplicates_alone_needs_no_coverage(self):
        for args, config, status in [(["--duplicates"], "", 0), (["--top", "2", "--duplicates"], "", 0),
                                     (["--", "--duplicates"], "", 2), ([], "[duplicates]\nenabled = true\n", 2)]:
            with self.subTest(args=args, config=config):
                (self.project / ".crapts.toml").write_text(config)
                (self.run_dir / "args.json").write_text(json.dumps(args))
                (self.run_dir / "prepared.json").unlink(missing_ok=True)
                result = self.prepare(status, INPUT_COVERAGE_FILE="never.json", INPUT_INSTALL_COMMAND="touch ran")
                self.assertFalse((self.project / "ran").exists())
                if status:
                    self.assertIn("set coverage or coverage-command", result.stderr)
                    self.assertFalse((self.run_dir / "prepared.json").exists())
                else:
                    self.assertEqual(self.prepared(), {"base": "", "coverage": ""})


def remote_at(path, depth, merge=False, disjoint=False):
    subprocess.run(["git", "init", "--bare", "-q", str(path)], check=True)
    seq, mark = [], 0

    def commit(branch, parent=None, extra="", second=None):
        nonlocal mark
        mark += 1
        seq.extend([f"commit refs/heads/{branch}", f"mark :{mark}",
                    f"committer Fixture <fixture@example.invalid> {1000000000 + mark} +0000",
                    "data 1", "c"])
        if parent:
            seq.append(f"from :{parent}")
        if second:
            seq.append(f"merge :{second}")
        if extra:
            seq.append(extra)
        seq.append("")
        return mark

    common = commit("ancestor")
    base = common
    head = None if disjoint else common
    for _ in range(depth):
        base = commit("base", base)
        head = commit("pr", head)
    source = "export function fee(n: number) { " + "if (n) n--; " * 5 + "return n; }\n"
    head = commit("pr", head, f"M 100644 inline fee.ts\ndata {len(source)}\n{source}")
    if merge:
        commit("merge", base, f"M 100644 inline fee.ts\ndata {len(source)}\n{source}", second=head)
    subprocess.run(["git", "-C", str(path), "fast-import", "--quiet"], input="\n".join(seq).encode(), check=True)


class History(ActionTest):
    def checkout(self, distance, merge, full, disjoint=False):
        remote = self.temp / "remote.git"
        remote_at(remote, distance, merge, disjoint)
        ref = "merge" if merge else "pr"
        base = self.git("-C", str(remote), "rev-parse", "refs/heads/base")
        self.project = self.project / "checkout"
        subprocess.run(["git", "clone", "-q", "-b", ref, *([] if full else ["--depth=1"]),
                        remote.as_uri(), str(self.project)], check=True)
        if not full and not merge:
            self.git("fetch", "-q", "--depth=1", "origin", base)
        (self.project / "hits.info").write_text("SF:fee.ts\nDA:1,0\nend_of_record\n")
        return remote, ref, base

    def verify_history(self, distance, fetches, merge=False, full=False):
        remote, ref, base = self.checkout(distance, merge, full)
        trace = self.temp / "git.trace"
        self.prepare(INPUT_BASE=base, INPUT_COVERAGE="hits.info", GIT_TRACE=str(trace))
        actual = [line.split("built-in: git fetch ")[1] for line in trace.read_text().splitlines()
                  if "built-in: git fetch " in line]
        self.assertEqual([re.search(r"--(depth=1|deepen=\d+|unshallow)", f)[1] for f in actual], fetches)
        for fetch in actual:
            self.assertIn(base, fetch)
            if "--depth=1 " not in fetch:
                self.assertIn(self.git("rev-parse", "HEAD"), fetch)
        self.assertEqual(self.prepared()["base"], base)
        self.assertEqual(self.git("merge-base", base, "HEAD"), self.git("-C", str(remote), "merge-base", base, ref))
        full_tree = self.temp / "full"
        self.git("clone", "-q", "-b", ref, str(remote), str(full_tree))

        def count(cwd):
            result = subprocess.run([str(BINARY), "--base", base, "--format", "json", "--coverage",
                                     self.prepared()["coverage"]], cwd=cwd, capture_output=True, text=True, check=True)
            return json.loads(result.stdout)["summary"]["changed_failures"]
        self.assertEqual(count(full_tree), 1)
        self.assertEqual(count(self.project), 1)

    def test_merge_checkout(self):
        self.verify_history(2, ["depth=1", "deepen=50"], merge=True)

    def test_medium_history(self):
        self.verify_history(300, ["deepen=50", "deepen=200", "deepen=1000"])

    def test_deep_history(self):
        self.verify_history(1300, ["deepen=50", "deepen=200", "deepen=1000", "unshallow"])

    def test_full_history(self):
        self.verify_history(2, [], full=True)

    def history_error(self, base, diagnostic):
        result = self.prepare(2, INPUT_BASE=base, INPUT_COVERAGE_COMMAND="touch ran", INPUT_INSTALL_COMMAND="touch ran")
        self.assertIn(base, result.stderr)
        self.assertIn(diagnostic, result.stderr)
        self.assertFalse((self.project / "ran").exists())
        self.assertFalse((self.run_dir / "prepared.json").exists())

    def test_disjoint(self):
        _, _, base = self.checkout(2, False, False, True)
        self.history_error(base, "no merge base between " + base + " and HEAD after unshallowing: the base is not in this repository's history")

    def test_missing_tracking_ref(self):
        _, _, base = self.checkout(2, True, False)
        self.prepare(INPUT_BASE="origin/base", INPUT_COVERAGE="hits.info")
        self.assertEqual(self.prepared()["base"], base)

    def test_missing(self):
        self.checkout(2, True, False)
        self.history_error("0" * 40, "origin")

    def test_failed_deepen(self):
        _, _, base = self.checkout(2, False, False)
        self.git("remote", "set-url", "origin", str(self.temp / "absent"))
        self.history_error(base, "fetch")
