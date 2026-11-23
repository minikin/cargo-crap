from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import threading
from urllib.parse import unquote
from support import ACTION, ActionTest, BINARY, MANIFEST, SCRIPTS, VERSION, WRAPPER, tarball


class Install(ActionTest):
    def test_contract(self):
        text = (ACTION / "action.yml").read_text()
        inputs = dict(re.findall(r'^  ([\w-]+):\n    description: [^\n]+\n    default: (.+)$',
                                 text.split("outputs:")[0], re.M))
        expected = dict(version=VERSION, package="", args="", coverage="", baseline="", comment="true",
                        base="${{ github.event.pull_request.base.sha }}")
        expected.update({"working-directory": ".", "coverage-file": "coverage/coverage-final.json",
                         "coverage-command": "", "install-command": "", "fail-changed": "true",
                         "fail-regression": "true", "fail-above": "false", "comment-marker": "<!-- crapts-report -->",
                         "github-token": "${{ github.token }}"})
        self.assertEqual({key: value.strip('"') for key, value in inputs.items()}, expected)
        self.assertEqual(re.findall(r'^  ([\w-]+):$', text.split("outputs:")[1].split("runs:")[0], re.M),
                         ["crappy", "regressed", "changed", "report"])

    def test_args(self):
        (self.project / "a.ts").touch()
        result = self.command(["node", str(SCRIPTS / "inputs.mjs")], INPUT_ARGS='"two words" "*.ts" ""')
        self.assertEqual(json.loads(result.stdout), ["two words", "*.ts", ""])
        for value, message in [("'unfinished", "quotes"), ("--format=json", "--format"),
                               ("--output other.md", "--output"), ("--base other", "--base"),
                               ("--baseline=old", "--baseline"), ("--comment-marker custom", "--comment-marker"),
                               ("--commit-ref=main", "--commit-ref"), ("--all-rows", "--all-rows")]:
            result = self.install(2, INPUT_ARGS=value)
            self.assertIn("args", result.stderr)
            self.assertIn(message, result.stderr)
        for value in ["", "a\rb", "a\nb"]:
            self.assertIn("one nonempty line", self.install(2, INPUT_COMMENT_MARKER=value).stderr)
        self.assertRegex(self.install(2, INPUT_FAIL_CHANGED="yes").stderr, "fail-changed.*true or false.*yes")
        self.assertIn("packs/missing.tgz", self.install(2, INPUT_PACKAGE="packs/missing.tgz").stderr)

    def test_missing_runner_temp(self):
        del self.env["RUNNER_TEMP"]
        self.assertIn("RUNNER_TEMP is required", self.install(2).stderr)

    def test_old_node(self):
        shim = self.temp / "node"
        shim.write_bytes(b'#!/usr/bin/env bash\necho v22.0.0\n')
        shim.chmod(0o755)
        result = self.install(2, PATH=str(self.temp) + os.pathsep + self.env["PATH"])
        self.assertRegex(result.stderr, "actions/setup-node.*node-version.*24")

    def test_registry_and_tarball(self):
        host = json.loads(self.command(["node", "-e",
            "const r=require(process.argv[1]); console.log(JSON.stringify(r.resolveTarget(process.platform,process.arch,"
            "process.platform==='linux'?r.detectLibc():'none')))", str(WRAPPER / "lib/resolve.js")]).stdout)
        archives, metadata, requests = {}, {}, []

        class Registry(BaseHTTPRequestHandler):
            def do_GET(self):
                path = unquote(self.path[1:])
                requests.append(path)
                if path in archives:
                    body = archives[path]
                elif path in metadata:
                    body = json.dumps(metadata[path]).encode()
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Registry)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        registry = "http://127.0.0.1:" + str(server.server_port)
        self.env["npm_config_registry"] = registry
        files = {p.relative_to(WRAPPER).as_posix(): p.read_bytes() for sub in ("lib", "bin")
                 for p in (WRAPPER / sub).iterdir()}
        for version in {VERSION, "0.1.2", "1.0.1", "0.2.0-rc.1", "0.2.0"}:
            platform = {"name": host["name"], "version": version}
            wrapper = {**MANIFEST, "version": version, "optionalDependencies": {host["name"]: version},
                       "scripts": {"postinstall": "node -e \"process.exit(7)\""}}
            wrapper_files = files.copy()
            if version != VERSION:
                wrapper_files["bin/crapts.js"] = f'#!/usr/bin/env node\nconsole.log("crapts {version}")\n'.encode()
            for pkg, content in [(platform, {host["binary"]: BINARY.read_bytes()}), (wrapper, wrapper_files)]:
                filename = pkg["name"].split("/")[1] + "-" + version + ".tgz"
                archives[filename] = tarball(pkg, content)
                doc = metadata.setdefault(pkg["name"], {"name": pkg["name"], "versions": {}, "dist-tags": {"latest": "0.2.0"}})
                doc["versions"][version] = {**pkg, "dist": {"tarball": registry + "/" + filename}}
        originals = {"package.json": '{"private":true}', "package-lock.json": '{"lockfileVersion":3}'}
        for name, body in originals.items():
            (self.project / name).write_text(body)
        for selection, expected, local in [(VERSION, VERSION, False), ("0.1.2", "0.1.2", False),
                                            ("latest", "1.0.1", False), ("9.9.9", "0.2.0-rc.1", True),
                                            ("9.9.9", VERSION, True)]:
            package = ""
            if local:
                package = "packs/test build.tgz"
                path = self.project / package
                path.parent.mkdir(exist_ok=True)
                path.write_bytes(archives["crapts-" + expected + ".tgz"])
            if selection == "latest":
                metadata[MANIFEST["name"]]["dist-tags"]["latest"] = "1.0.1"
            requests.clear()
            self.install(INPUT_VERSION=selection, INPUT_PACKAGE=package)
            location = Path(self.env["GITHUB_PATH"]).read_text().splitlines()[-1]
            self.assertTrue(Path(location).is_relative_to(self.temp))
            outputs = dict(line.split("=", 1) for line in Path(self.env["GITHUB_OUTPUT"]).read_text().splitlines())
            self.assertTrue(Path(outputs["binary"]).is_file())
            if expected == VERSION:
                self.assertEqual(self.command([outputs["binary"], "--version"]).stdout.strip(), "crapts " + VERSION)
            result = self.command(["bash", "-c", 'cd "$1" && export PATH="$PWD:$PATH" && crapts --version', "_", location])
            self.assertEqual(result.stdout.strip(), "crapts " + expected)
            if not local:
                self.assertIn("crapts-" + expected + ".tgz", requests)
        wrong = {**MANIFEST, "version": "0.1.2", "optionalDependencies": {host["name"]: "0.1.2"}}
        files["bin/crapts.js"] = b'#!/usr/bin/env node\nconsole.log("crapts 0.1.1")\n'
        archives["wrong.tgz"] = tarball(wrong, files)
        metadata[MANIFEST["name"]]["versions"]["0.1.2"]["dist"]["tarball"] = registry + "/wrong.tgz"
        result = self.install(2, INPUT_VERSION="0.1.2", npm_config_cache=str(self.temp / "wrong-cache"))
        self.assertIn("expected 0.1.2", result.stderr)
        self.assertIn("actual 0.1.1", result.stderr)
        for name, body in originals.items():
            self.assertEqual((self.project / name).read_text(), body)
