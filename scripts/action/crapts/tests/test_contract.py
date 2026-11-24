import unittest

from support import ACTION, SCRIPTS


class ActionContract(unittest.TestCase):
    def test_only_the_owning_repository_can_override_the_comment_context(self):
        manifest = (ACTION / "action.yml").read_text()
        overrides = {
            "PR_NUMBER": "github.event.pull_request.number",
            "API_URL": "github.api_url",
        }
        expected = []
        for name in ["PR_NUMBER", "PR_NUMBER", "API_URL"]:
            expression = "(github.repository == 'minikin/cargo-crap' && env.CRAPTS_TEST_" + name + ")"
            expected.append("CRAPTS_" + name + ": ${{ " + expression + " || " + overrides[name] + " }}")
        self.assertCountEqual([line.strip() for line in manifest.splitlines() if "CRAPTS_TEST_" in line], expected)

        for path in SCRIPTS.rglob("*"):
            if not path.is_file() or "__pycache__" in path.parts:
                continue
            source = path.read_bytes()
            for name in overrides:
                self.assertNotIn(("CRAPTS_TEST_" + name).encode(), source, str(path))
