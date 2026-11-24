import json
import os
from pathlib import Path
import subprocess
import sys
from support import VERSION, write_history
from test_baseline import coverage, function


def git(cwd, *args):
    return subprocess.check_output(["git", "-C", str(cwd), *args], text=True).strip()


def setup(directory):
    origin = directory / "origin"
    origin.mkdir(parents=True)
    project = "packages/shop/"
    base = write_history(origin, {
        project + "fee.ts": function("fee", 8),
        project + "lock": "base\n",
        project + "tests/fee.test.ts": "export {};\n",
        project + "scripts with spaces/install deps.sh": """set -eu
mkdir -p node_modules
cp lock node_modules/installed
echo "install $(git rev-parse HEAD) $(cat lock)" >> "$TRACE"
""",
        project + "scripts with spaces/run coverage.sh": """set -eu
test ! -e coverage.info
echo "cover $(git rev-parse HEAD) $(cat node_modules/installed)" >> "$TRACE"
hits=0; if test -e tests/fee.test.ts; then hits=1; fi
printf 'SF:fee.ts\\nDA:1,%s\\nend_of_record\\n' "$hits" > coverage.info
if test -e fresh.ts; then printf 'SF:fresh.ts\\nDA:1,0\\nend_of_record\\n' >> coverage.info; fi
"""}, {project + "lock": "head\n", project + "fresh.ts": function("fresh", 6), project + "tests/fee.test.ts": None})
    checkout = directory / "checkout"
    subprocess.run(["git", "clone", "--quiet", "--depth=1", "--branch", "pr", origin.as_uri(), str(checkout)], check=True)
    assert git(checkout, "rev-parse", "--is-shallow-repository") == "true"
    push = directory / "push"
    push.mkdir()
    (push / "shipping.ts").write_text(function("shipping", 2))
    (push / "hits.info").write_text(coverage({"shipping": 1}))
    (directory / "state.json").write_text(json.dumps({"base": base, "head": git(checkout, "rev-parse", "HEAD")}))
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        print("base=" + base, file=output)


def verify(directory, pr):
    state = json.loads((directory / "state.json").read_text())
    report = Path(os.environ["REPORT"]).read_text(encoding="utf-8")
    actual = [os.environ[k] for k in ["CRAPPY", "REGRESSED", "CHANGED", "OUTCOME"]]
    if pr:
        assert actual == ["2", "1", "1", "failure"], actual
        assert "fee" in report and "fresh" in report, report
        checkout = directory / "checkout"
        assert git(checkout, "rev-parse", "HEAD") == state["head"]
        assert len(git(checkout, "worktree", "list", "--porcelain").splitlines()) == 3
        assert not (Path(os.environ["REPORT"]).parent / "base-worktree").exists()
        expected = [f"{cmd} {state[role]} {role}" for role in ["head", "base"] for cmd in ["install", "cover"]]
        assert (directory / "trace").read_text().splitlines() == expected
    else:
        assert actual == ["0", "", "", "success"], actual
        assert "shipping" in report, report
        assert os.environ["LAUNCHER_VERSION"] == "crapts " + os.environ.get("VERSION", VERSION)


if __name__ == "__main__":
    directory = Path(sys.argv[2]).resolve()
    if sys.argv[1] == "setup":
        setup(directory)
    else:
        verify(directory, sys.argv[1] == "pr")
