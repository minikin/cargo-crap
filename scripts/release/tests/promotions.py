from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import unittest
from urllib.parse import unquote
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[3]
RELEASE = ROOT / "scripts/release"
WRAPPER = Path("npm/crapts/package.json")
ACTION = Path("actions/crapts/action.yml")


class Promotions(unittest.TestCase):
    def test_remote_tag_and_all_npm_dist_tags_follow_release_eligibility(self):
        source = ROOT / ".github/workflows/release-crapts.yml"
        workflow = json.loads(subprocess.check_output(
            ["ruby", "-ryaml", "-rjson", "-e", "puts YAML.safe_load_file(ARGV[0]).to_json", str(source)]))
        jobs = workflow["jobs"]
        tag_job, npm_job = jobs["move-crapts-v1"], jobs["promote-npm"]
        wrapper = json.loads((ROOT / WRAPPER).read_text())
        names = [*wrapper["optionalDependencies"], wrapper["name"]]
        tags = {name: {"latest": "0.0.8"} for name in names}
        writes = []

        class Registry(BaseHTTPRequestHandler):
            def name(self):
                return unquote(self.path).removeprefix("/-/package/").partition("/dist-tags")[0]

            def do_GET(self):
                if self.name() not in tags:
                    self.send_error(404)
                    return
                body = json.dumps(tags[self.name()]).encode()
                self.send_response(200)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_PUT(self):
                if self.name() not in tags or not self.path.endswith("/dist-tags/latest"):
                    self.send_error(404)
                    return
                value = json.loads(self.rfile.read(int(self.headers["content-length"])))
                tags[self.name()]["latest"] = value
                writes.append((self.name(), value))
                self.send_response(204)
                self.end_headers()

            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Registry)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        url = "http://127.0.0.1:" + str(server.server_port)
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        directory = Path(scratch.name)
        repo, remote = directory / "work", directory / "origin.git"
        repo.mkdir()
        config = directory / "fixture.npmrc"
        config.write_text(f"registry={url}/\n@minikin:registry={url}/\n//127.0.0.1:{server.server_port}/:_authToken=fixture\n")
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
                   npm_config_registry=url, npm_config_userconfig=str(config),
                   npm_config_cache=str(directory / "cache"), npm_config_fetch_retries="0",
                   npm_config_update_notifier="false")
        for key in ["GITHUB_OUTPUT", "NPM_TOKEN", "NODE_AUTH_TOKEN"]:
            env.pop(key, None)

        def git(*args):
            return subprocess.check_output(["git", *args], cwd=repo, env=env, text=True).strip()

        git("init", "--bare", "-q", str(remote))
        git("init", "-q")
        git("remote", "add", "origin", str(remote))
        cases = [
            ("0.1.0", "success", "success", True, True),
            ("1.4.2", "success", "success", True, True),
            ("1.4.2-rc.1", "success", "success", False, False),
            ("2.0.0", "success", "success", False, True),
            ("1.4.2", "success", "failure", False, False),
            ("1.4.2", "failure", "skipped", False, False),
            ("1.4.2", "success", "skipped", False, False),
            ("0.2.0", "success", "success", True, True),
            ("0.1.5", "success", "success", True, True),
            ("1", "skipped", "skipped", False, False),
        ]
        stream = "\n".join(f"commit refs/heads/releases\ncommitter Fixture <fixture@example.invalid> {1000000000+i} +0000\ndata 1\nx\n"
                           for i in range(len(cases) + 1))
        subprocess.run(["git", "fast-import", "--quiet"], input=stream.encode(), cwd=repo, env=env, check=True)
        first, *commits = git("rev-list", "--reverse", "releases").splitlines()
        git("push", "-q", "origin", first + ":refs/tags/v1", first + ":refs/tags/crapts-v1")
        for path in [WRAPPER, ACTION, Path("crates/crapts/Cargo.toml"), Path("scripts/release/move-tag.sh")]:
            (repo / path).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(RELEASE / "move-tag.sh", repo / "scripts/release/move-tag.sh")
        pattern, = workflow["true"]["push"]["tags"]
        moved, latest = first, "0.0.8"
        for (version, npm, smoke, move, promote), commit in zip(cases, commits, strict=True):
            with self.subTest(version=version, npm=npm, smoke=smoke):
                writes.clear()
                ref = "crapts-v" + version
                if re.fullmatch(pattern.replace(".", r"\.").replace("*", ".*"), ref):
                    (repo / "crates/crapts/Cargo.toml").write_text(f'[package]\nversion="{version}"\n')
                    (repo / WRAPPER).write_text(json.dumps({**wrapper, "version": version,
                                               "optionalDependencies": dict.fromkeys(names[:-1], version)}))
                    (repo / ACTION).write_text(f'inputs:\n  version:\n    default: "{version}"\n')
                    checked = subprocess.check_output(["bash", str(RELEASE / "check-tag.sh"), "crapts", ref],
                                                      cwd=repo, env=env, text=True)
                    preflight = dict(line.split("=", 1) for line in checked.splitlines())
                    states = {"preflight": "success", "npm-smoke": npm, "crapts-action-smoke": smoke}
                    for job in [npm_job, tag_job]:
                        if any(states[name] != "success" for name in job["needs"]):
                            continue
                        condition = re.sub(r"needs\.preflight\.outputs\.(\w+)", lambda m: preflight[m[1]], job["if"])
                        if subprocess.run(["bash", "-c", "[[ " + condition + " ]]"], cwd=repo, env=env).returncode:
                            continue
                        command = job["steps"][-1]["run"]
                        subprocess.run(["bash", "-eo", "pipefail", "-c", command], cwd=repo,
                                       env={**env, "VERSION": preflight["version"], "GITHUB_SHA": commit}, check=True)
                if move:
                    moved = commit
                if promote:
                    latest = version
                self.assertCountEqual(writes, [(name, version) for name in names] if promote else [])
                for name in names:
                    with urlopen(url + "/-/package/" + name + "/dist-tags") as response:
                        self.assertEqual(json.load(response)["latest"], latest)
                self.assertEqual(git("ls-remote", "origin", "refs/tags/crapts-v1").split()[0], moved)
                self.assertEqual(git("ls-remote", "origin", "refs/tags/v1").split()[0], first)


if __name__ == "__main__":
    unittest.main()
