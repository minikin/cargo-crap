import io
import json
import os
from pathlib import Path
import subprocess
import shutil
import tarfile
import tempfile
import unittest

os.environ["GIT_CONFIG_GLOBAL"] = os.devnull
os.environ["GIT_CONFIG_NOSYSTEM"] = "1"

ROOT = Path(__file__).resolve().parents[4]
SCRIPTS = ROOT / "scripts/action/crapts"
ACTION = ROOT / "actions/crapts"
WRAPPER = ROOT / "npm/crapts"
MANIFEST = json.loads((WRAPPER / "package.json").read_text())
VERSION = MANIFEST["version"]
BINARY = Path(os.environ.get("CRAPTS_BINARY", ROOT / "target/debug/crapts")).resolve()
if os.name == "nt":
    BINARY = BINARY.with_suffix(".exe")


def tarball(manifest, files):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:gz") as archive:
        for name, body in {"package.json": json.dumps(manifest).encode(), **files}.items():
            member = tarfile.TarInfo("package/" + name)
            member.size = len(body)
            member.mode = 0o755
            archive.addfile(member, io.BytesIO(body))
    return data.getvalue()


class ActionTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.temp = Path(scratch.name).resolve()
        self.project = self.temp / "project with spaces"
        self.project.mkdir()
        self.run_dir = self.temp / "run"
        self.run_dir.mkdir()
        (self.run_dir / "args.json").write_text("[]")
        self.env = dict(os.environ, RUNNER_TEMP=str(self.temp), GITHUB_ACTION_PATH=str(ACTION),
                        GITHUB_PATH=str(self.temp / "path"), GITHUB_ENV=str(self.temp / "env"),
                        GITHUB_OUTPUT=str(self.temp / "outputs"), GITHUB_STEP_SUMMARY=str(self.temp / "summary"),
                        CRAPTS_TEMP=str(self.run_dir), CRAPTS_BINARY=str(BINARY), ARG_TRACE=str(self.temp / "args.log"),
                        INPUT_BASELINE="", INPUT_BASE="", INPUT_COVERAGE="", INPUT_COVERAGE_COMMAND="",
                        INPUT_INSTALL_COMMAND="", INPUT_COVERAGE_FILE="coverage/coverage-final.json", INPUT_ARGS="", INPUT_FAIL_ABOVE="false", INPUT_FAIL_CHANGED="true",
                        INPUT_FAIL_REGRESSION="true", INPUT_COMMENT="true", INPUT_COMMENT_MARKER="<!-- crapts-report -->",
                        INPUT_VERSION=VERSION, INPUT_PACKAGE="", npm_config_cache=str(self.temp / "cache"),
                        npm_config_fetch_retries="0")

    def command(self, args, status=0, **env):
        result = subprocess.run(args, cwd=self.project, env={**self.env, **env}, capture_output=True, text=True)
        self.assertEqual(result.returncode, status, result.stdout + result.stderr)
        return result

    def install(self, status=0, **env):
        return self.command(["bash", str(SCRIPTS / "install.sh")], status, **env)

    def prepare(self, status=0, **env):
        return self.command(["node", str(SCRIPTS / "prepare.mjs"), self.env.get("CRAPTS_TEST_BASH") or shutil.which("bash")],
                            status, **env)

    def prepared(self):
        return json.loads((self.run_dir / "prepared.json").read_text())

    def git(self, *args):
        return self.command(["git", *args]).stdout.strip()

    def stage(self, name, status=0, **env):
        return self.command(["node", "--import", (SCRIPTS / "tests/trace.mjs").as_uri(), str(SCRIPTS / (name + ".mjs")),
                             self.env.get("CRAPTS_TEST_BASH") or shutil.which("bash")], status, **env)

    def calls(self):
        path = Path(self.env["ARG_TRACE"])
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def history(self, base_files, head_changes, *, base_changes=None):
        return write_history(self.project, base_files, head_changes, base_changes=base_changes)


def write_history(project, base_files, head_changes, *, base_changes=None):
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=project, text=True).strip()

    git("init", "-q")
    commits = []
    for mark, (branch, parent, files) in enumerate([("common", None, base_files), ("base", 1, base_changes or {}),
                                                   ("pr", 1, head_changes)], start=1):
        commits.extend([f"commit refs/heads/{branch}", f"mark :{mark}",
                        f"committer Fixture <fixture@example.invalid> {1000000000 + mark} +0000",
                        "data 1", str(mark)])
        if parent:
            commits.append(f"from :{parent}")
        for name, body in files.items():
            if body is None:
                commits.append("D " + name)
            else:
                commits.extend(["M 100644 inline " + name, "data " + str(len(body.encode())), body])
        commits.append("")
    subprocess.run(["git", "fast-import", "--quiet"], cwd=project, input="\n".join(commits).encode(), check=True)
    git("checkout", "-q", "pr")
    return git("rev-parse", "base")
