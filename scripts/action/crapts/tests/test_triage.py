import json
import os
import subprocess
import threading
from support import ActionTest, BINARY
import scenarios


class TriageBudget(ActionTest):
    def setUp(self):
        super().setUp()
        probe = self.temp / "probe"
        probe.mkdir()
        (probe / ".crapts.toml").write_text("[duplicates.triage]\nenabled = true\n")
        result = subprocess.run([str(BINARY), "--duplicates", "--format", "json"], cwd=probe, capture_output=True,
                                text=True, env={**self.env, "TYPESAFE_API_KEY": ""})
        if "built without the triage client" in result.stderr:
            if os.environ.get("CRAPTS_REQUIRE_TRIAGE") == "1":
                self.fail("CRAPTS_REQUIRE_TRIAGE=1, but " + str(BINARY) + " was built without the triage feature")
            self.skipTest("crapts was built without the triage feature")

    def test_one_budget_across_the_composite(self):
        log = self.temp / "triage.log"
        urls = []
        server = scenarios.serve(log, urls.append)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.project = self.project / "shop"
        base = scenarios.budget_history(self.project)
        self.assertEqual(len(scenarios.groups(BINARY, self.project)), 5)
        cache = self.temp / "cold cache"
        trace = self.temp / "project-commands"
        (self.run_dir / "trusted.json").write_text(json.dumps({"TYPESAFE_BASE_URL": urls[0]}))
        self.env.update(INPUT_BASE=base, INPUT_COVERAGE_COMMAND="bash cover.sh", INPUT_COVERAGE_FILE="coverage.info",
                        TRACE=str(trace), TYPESAFE_API_KEY="not-a-key", TYPESAFE_BASE_URL=urls[0], CRAPTS_PR_NUMBER="",
                        HOME=str(cache), XDG_CACHE_HOME=str(cache), LOCALAPPDATA=str(cache))
        counts = []
        for stage in ["prepare", "baseline", "score"]:
            result = self.prepare() if stage == "prepare" else self.stage(stage)
            counts.append(len(scenarios.requests(log)))
        self.assertEqual(counts, [0, 0, 2])
        asked = scenarios.requests(log)
        self.assertFalse(any("legacy" in request["body"] for request in asked), asked)
        self.assertNotIn("duplicate triage skipped", result.stderr)
        self.assertEqual(result.stderr.count("the triage budget (max-requests = 2) left 3 duplicate groups"), 1)
        self.assertEqual(set(trace.read_text().split()), {"."})
        paid = [call for call in self.calls() if call["keys"]]
        self.assertEqual(len(paid), 1)
        self.assertIn("markdown", paid[0]["args"])
