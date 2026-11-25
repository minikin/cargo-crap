import json
import re
import subprocess
import unittest

from support import ACTION, ROOT, SCRIPTS


def exported(name):
    source = f'import("{(SCRIPTS / "cli.mjs").as_uri()}").then(m => console.log(JSON.stringify(m.{name})))'
    return json.loads(subprocess.check_output(["node", "-e", source], text=True))


class ActionContract(unittest.TestCase):
    def test_only_the_owning_repository_can_override_the_comment_context(self):
        manifest = (ACTION / "action.yml").read_text()
        overrides = {
            "PR_NUMBER": "github.event.pull_request.number",
            "API_URL": "github.api_url",
        }
        expected = []
        for name in ["PR_NUMBER", "PR_NUMBER", "API_URL"]:
            expression = ("(github.repository == 'minikin/cargo-crap' && inputs.github-token == 'fixture-token'"
                          " && env.CRAPTS_TEST_" + name + ")")
            expected.append("CRAPTS_" + name + ": ${{ " + expression + " || " + overrides[name] + " }}")
        self.assertCountEqual([line.strip() for line in manifest.splitlines() if "env.CRAPTS_TEST_" in line], expected)
        self.assertIn("CRAPTS_DEFAULT_TOKEN: ${{ inputs.github-token == github.token }}",
                      [line.strip() for line in manifest.split("Comment on the pull request")[1].splitlines()])

        for path in SCRIPTS.rglob("*"):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            source = path.read_bytes()
            for name in overrides:
                self.assertNotIn(("CRAPTS_TEST_" + name).encode(), source, str(path))

    def test_every_provider_variable_is_withheld_or_pinned(self):
        providers = ROOT / "crates/cargo-crap-core/src/duplicates/triage/provider"
        found = {"API_KEY_VAR": set(), "BASE_URL_VAR": set()}
        for path in providers.rglob("*.rs"):
            for kind, value in re.findall(r'pub const (API_KEY_VAR|BASE_URL_VAR): &str = "(\w+)"', path.read_text()):
                found[kind].add(value)
        self.assertEqual(len(found["API_KEY_VAR"]), 2, found)
        self.assertEqual(set(exported("PROVIDER_KEYS")), found["API_KEY_VAR"])
        self.assertEqual(set(exported("PROVIDER_URLS")), found["BASE_URL_VAR"])
