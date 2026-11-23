from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
from urllib.parse import parse_qs, urlparse
from support import ActionTest, BINARY, SCRIPTS
from test_baseline import function, coverage


class Delivery(ActionTest):
    def setUp(self):
        super().setUp()
        self.requests, self.comments, self.refusal = [], [], None
        owner = self
        class API(BaseHTTPRequestHandler):
            def handle_request(self):
                parsed = urlparse(self.path)
                query = parse_qs(parsed.query)
                body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"null")
                owner.requests.append((self.command, parsed.path, query, body))
                collection = "/repos/minikin/cargo-crap/issues/42/comments"
                status, reply = 200, {}
                if self.headers.get("Authorization") != "Bearer fixture-token":
                    status = 401
                elif self.command == "GET" and parsed.path == collection:
                    page = int(query["page"][0])
                    assert query["per_page"] == ["100"]
                    reply = owner.comments[100 * (page - 1):100 * page]
                elif owner.refusal:
                    status, reply = owner.refusal, {"message": "write refused"}
                elif self.command == "POST" and parsed.path == collection:
                    owner.comments.append({"id": len(owner.comments) + 1, **body})
                    status = 201
                elif self.command == "PATCH" and parsed.path.startswith("/repos/minikin/cargo-crap/issues/comments/"):
                    owner.comments[int(parsed.path.split("/")[-1]) - 1].update(body)
                else:
                    status = 404
                data = json.dumps(reply).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            do_GET = do_POST = do_PATCH = handle_request
            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), API)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.env.update(CRAPTS_PR_NUMBER="42", CRAPTS_API_URL=f"http://127.0.0.1:{server.server_port}",
                        CRAPTS_TOKEN="fixture-token", GITHUB_REPOSITORY="minikin/cargo-crap",
                        GITHUB_SERVER_URL="https://github.com", CRAPTS_REVISION="pr-head",
                        GITHUB_SHA="merge-commit", INPUT_COVERAGE="hits.info",
                        REPORT=str(self.run_dir / "report.md"))

    def outputs(self):
        return dict(line.split("=", 1) for line in Path(self.env["GITHUB_OUTPUT"]).read_text().splitlines())

    def deliver(self, **env):
        self.prepare(**env)
        self.stage("baseline", **env)
        self.stage("score", **env)
        self.stage("comment", **env)
        out = self.outputs()
        return self.command(["bash", "-c", 'exit "$STATUS"'], int(out["status"]), STATUS=out["status"]).returncode

    def test_saved_baseline_and_git_base_in_one_comment(self):
        base = self.history({"fee.ts": function("fee", 8)}, {name + ".ts": function(name, 6) for name in ["one", "two"]})
        (self.project / "old.info").write_text(coverage({"fee": 1}))
        saved = self.command([str(BINARY), "fee.ts", "--format", "json", "--all-rows", "--coverage", "old.info"]).stdout
        (self.project / "saved.json").write_text(saved)
        (self.project / "hits.info").write_text(coverage({"fee": 0, "one": 0, "two": 0}))
        self.assertEqual(self.deliver(INPUT_BASE=base, INPUT_BASELINE="saved.json", INPUT_COVERAGE_COMMAND="exit 7"), 1)
        out = self.outputs()
        self.assertEqual([out[k] for k in ["crappy", "regressed", "changed"]], ["3", "1", "2"])
        report = Path(out["report"]).read_text()
        for name in ["fee", "one", "two"]:
            self.assertIn(name, report)
        self.assertEqual(self.comments, [{"id": 1, "body": report}])
        self.assertEqual([r[0] for r in self.requests], ["GET", "POST"])
        for call in self.calls()[-2:]:
            self.assertIn("--base", call["args"])
            self.assertIn("--baseline", call["args"])

    def test_config_gate_and_read_only_fork(self):
        self.history({"fee.ts": function("fee", 6)}, {})
        (self.project / "hits.info").write_text(coverage({"fee": 0}))
        for fail in [False, True]:
            (self.project / ".crapts.toml").write_text("fail-above=" + str(fail).lower())
            self.prepare()
            self.stage("baseline")
            self.stage("score")
            out = self.outputs()
            self.assertEqual((out["crappy"], out["regressed"], out["changed"], out["status"]), ("1", "", "", str(int(fail))))
            report = Path(out["report"]).read_text()
            self.assertIn(report, Path(self.env["GITHUB_STEP_SUMMARY"]).read_text())
            self.refusal = None
            self.stage("comment")
            self.assertEqual(self.comments[0]["body"], report)
            for status in [403, 404]:
                self.refusal = status
                for existing in [False, True]:
                    self.comments[:] = [{"id": 1, "body": report}] if existing else []
                    result = self.stage("comment")
                    self.assertEqual(result.stdout.count("::warning::"), 1)
                    self.assertRegex(result.stdout, f"{status}.*job summary")
                    self.assertEqual(self.requests[-1][0], "PATCH" if existing else "POST")
                    self.assertEqual(out, self.outputs())
                    self.command(["bash", "-c", 'exit "$STATUS"'], int(fail), STATUS=out["status"])
        (self.project / ".crapts.toml").write_text("fail-above=false\n")
        self.stage("score")
        self.assertEqual(self.outputs()["status"], "0")
        self.assertIn(report.splitlines()[0], Path(self.env["GITHUB_STEP_SUMMARY"]).read_text())
        self.refusal = 500
        result = self.stage("comment", 1)
        self.assertIn("500", result.stderr)

    def test_links_settings_and_disabled_comment(self):
        self.history({}, {"packages/shop/src/fee.ts": function("fee", 6)})
        self.project = self.project / "packages/shop"
        (self.project / "hits.info").write_text("SF:src/fee.ts\nDA:1,0\nend_of_record\n")
        (self.project / ".crapts.toml").write_text('threshold=29\nexclude=["vendor/**"]\nallow=["unused"]\n')
        for pr, comment, revision in [("", "true", "push-commit"), ("42", "true", "pr-head"), ("42", "false", "pr-head")]:
            self.requests.clear()
            self.deliver(CRAPTS_PR_NUMBER=pr, INPUT_COMMENT=comment, CRAPTS_REVISION=revision)
            report = Path(self.outputs()["report"]).read_text()
            self.assertIn("/blob/" + revision + "/packages/shop/src/fee.ts", report)
            settings = json.loads((self.run_dir / "settings.json").read_text())
            self.assertIn(".crapts.toml", json.dumps(settings))
            for key in ["threshold", "fail_above", "allow", "exclude"]:
                self.assertIn(key, settings["settings"])
            self.assertEqual([r[0] for r in self.requests], ["GET", "POST"] if pr and comment == "true" else [])

    def test_pagination_and_renderer_budget(self):
        self.history({}, {"many.ts": "".join(function(f"f{i:02}", 6) for i in range(30))})
        (self.project / "hits.info").write_text("SF:many.ts\n" + "".join(f"DA:{i},0\n" for i in range(1, 31)) + "end_of_record\n")
        self.prepare()
        self.stage("baseline")

        def render(length, marker):
            self.stage("score", CRAPTS_REVISION="r" * length, INPUT_COMMENT_MARKER=marker)
            return Path(self.env["REPORT"]).read_bytes().decode()

        def units(s):
            return len(s.encode("utf-16-le")) // 2

        marker = "m" * 60
        short = units(render(80, marker))
        step = units(render(81, marker)) - short
        length, extra = divmod(65536 - short, step)
        self.assertLess(extra + 1, 60)
        wide = "\U0001f600"
        for mode, existing in [("short", True), ("exact", True), ("over", True), ("short", False)]:
            mark = self.env["INPUT_COMMENT_MARKER"] if mode == "short" else wide * extra + "m" * (60 - extra)
            if mode == "over":
                mark = wide + mark[1:] if not extra else mark[:-1] + wide
            report = render(80 if mode == "short" else length + 80, mark)
            if mode == "exact":
                self.assertEqual(units(report), 65536)
            self.assertLessEqual(units(report), 65536)
            self.comments[:] = [{"id": i, "body": "unrelated"} for i in range(1, 151)]
            self.comments[0]["body"], self.comments[1]["body"] = mark + "x\n", "quoted\n" + mark
            if existing:
                self.comments[119]["body"] = mark + "\nold"
            self.requests.clear()
            self.stage("comment", INPUT_COMMENT_MARKER=mark)
            self.assertEqual([r[0] for r in self.requests], ["GET", "GET", "PATCH" if existing else "POST"])
            self.assertEqual([r[2]["page"][0] for r in self.requests[:2]], ["1", "2"])
            if existing:
                self.assertTrue(self.requests[-1][1].endswith("/120"))
            self.assertEqual(len(self.comments), 150 if existing else 151)
            self.assertEqual(self.requests[-1][3]["body"], report)
            self.assertTrue(report.startswith(mark + "\n"))
            self.assertEqual(report.splitlines().count(mark), 1)
