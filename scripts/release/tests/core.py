import io
import json
import os
import subprocess
import tarfile
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parents[1]
CORE = "crates/cargo-crap-core"
TAG = "cargo-crap-core-v0.2.0"
PACKAGES = json.loads((ROOT / "npm/crapts/package.json").read_text())[
    "optionalDependencies"
]


class CoreRelease(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.base = Path(tmp.name)
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.remote = self.base / "origin.git"
        self.status = 404
        self.archive = b""
        self.uploads = 0
        case = self

        class Registry(BaseHTTPRequestHandler):
            def do_GET(self):
                if not self.headers.get("User-Agent") or not self.path.startswith(
                    "/crates/cargo-crap-core/0.2.0"
                ):
                    self.send_error(400)
                    return
                self.send_response(case.status)
                self.end_headers()
                self.wfile.write(
                    case.archive if self.path.endswith("/download") else b"{}"
                )

            def do_PUT(self):
                case.uploads += 1
                case.status = 200
                self.send_response(200)
                self.end_headers()

            def log_message(self, *_):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Registry)
        threading.Thread(
            target=server.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True
        ).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.api = f"http://127.0.0.1:{server.server_port}"
        self.env = os.environ.copy()
        self.env["CARGO_NET_OFFLINE"] = "true"
        self.env["TEST_API"] = self.api
        self.env["REAL_CARGO"] = subprocess.check_output(
            ["rustup", "which", "cargo"], text=True
        ).strip()
        self.env["PATH"] = str(self.base) + os.pathsep + self.env["PATH"]
        stub = self.base / "cargo"
        stub.write_text(
            '#!/bin/sh\nif [ "$1" = publish ]; then\n'
            '  curl -fsS -X PUT "$TEST_API/upload" || exit 1\n'
            '  if [ -n "${RACE_COMMIT:-}" ]; then git push origin "$RACE_COMMIT:refs/tags/'
            + TAG
            + '" && exit 1; fi\n'
            '  exit 0\nfi\nexec "$REAL_CARGO" "$@"\n'
        )
        stub.chmod(0o755)
        self.git("init", "--quiet", "--bare", str(self.remote))
        self.git("init", "--quiet", "-b", "main")
        self.git("remote", "add", "origin", str(self.remote))
        author = subprocess.check_output(
            ["git", "-C", str(ROOT), "log", "-1", "--format=%an%n%ae"], text=True
        ).splitlines()
        self.git("config", "user.name", author[0])
        self.git("config", "user.email", author[1])
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        self.write(
            "Cargo.toml",
            '[workspace]\nmembers=["crates/cargo-crap-core"]\nresolver="2"\n'
            '[workspace.package]\nlicense="MIT"\n[workspace.lints.rust]\nunsafe_code="warn"\n'
            '[workspace.dependencies]\nanyhow="1"\nsyn="2"\n',
        )
        self.write(
            CORE + "/Cargo.toml",
            '[package]\nname="cargo-crap-core"\nversion="0.2.0"\nedition="2021"\n'
            "license.workspace=true\n[dependencies]\nanyhow.workspace=true\n[lints]\nworkspace=true\n",
        )
        self.write(CORE + "/src/lib.rs", "pub const VALUE: usize = 7;\n")
        self.write(".gitignore", "/target/\n")
        self.cmd(["cargo", "generate-lockfile", "--offline"])
        self.commit()
        self.initial = self.git("rev-parse", "HEAD").strip()
        self.git("push", "-q", "origin", "main")
        self.git(
            "--git-dir=" + str(self.remote), "symbolic-ref", "HEAD", "refs/heads/main"
        )

    def cmd(self, args, check=True):
        result = subprocess.run(
            args,
            check=False,
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
        )
        if check and result.returncode:
            self.fail(f"{args}: {result.stderr}")
        return result

    def git(self, *args):
        return self.cmd(["git", *args]).stdout

    def write(self, path, data):
        dest = self.repo / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(data)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-qm", "Release fixture")

    def helper(self, name, *args, code=0, message=None):
        res = self.cmd(["bash", str(SCRIPTS / (name + ".sh")), *args], False)
        self.assertEqual(res.returncode, code, (res.stdout, res.stderr))
        if message:
            self.assertIn(message, res.stderr if code else res.stdout)
        return res.stdout.strip()

    def check(self, **kw):
        return self.helper("check-core", "--api-base", self.api, **kw)

    def publish(self, *args, **kw):
        return self.helper("publish-core", "--api-base", self.api, *args, **kw)

    def tag(self):
        self.git("tag", TAG)
        self.git("push", "-q", "origin", TAG)

    def test_core_release_check_detects_unpublished_versions_tag_conflicts_and_fails_closed_on_errors(
        self,
    ):
        self.check(message="unpublished")
        self.status = 200
        self.check(code=1, message="--recover-tag")
        self.status = 404
        self.tag()
        self.check(code=1, message=TAG)
        self.status = 503
        self.check(code=1, message="503")
        self.assertEqual(self.uploads, 0)

    def test_core_diff_check_stops_a_release_when_published_core_has_local_or_inherited_manifest_edits(
        self,
    ):
        self.tag()
        self.status = 200
        for path, before, after in [
            (CORE + "/src/lib.rs", "7", "8"),
            ("Cargo.toml", 'anyhow="1"', 'anyhow="1.0"'),
            ("Cargo.toml", 'license="MIT"', 'license="ISC"'),
            ("Cargo.toml", '"warn"', '"deny"'),
        ]:
            with self.subTest(path=path, before=before):
                current = (self.repo / path).read_text()
                self.write(path, current.replace(before, after))
                self.commit()
                self.check(code=1, message="bump")
                self.write(path, current)
                self.commit()

    def test_core_diff_check_passes_when_the_published_core_matches_its_tag_even_in_a_shallow_checkout(
        self,
    ):
        self.tag()
        self.status = 200
        self.write(
            "Cargo.toml",
            (self.repo / "Cargo.toml").read_text().replace('syn="2"', 'syn="1"'),
        )
        self.write("src/main.rs", "fn main() {}")
        self.commit()
        self.git("push", "-q", "origin", "main")
        shallow = self.base / "shallow"
        self.git("clone", "--no-tags", "--depth=1", self.remote.as_uri(), str(shallow))
        self.repo = shallow
        self.assertEqual(self.git("tag"), "")
        self.check(message="published")
        self.assertEqual(self.git("rev-parse", TAG + "^{commit}").strip(), self.initial)

    def test_core_publication_handles_interruption_recovery_and_competing_releases_safely(
        self,
    ):
        self.cmd(["cargo", "package", "--locked", "--offline", "-p", "cargo-crap-core"])
        archive = next((self.repo / "target/package").glob("*.crate")).read_bytes()
        self.status = 200
        self.check(code=1, message="--recover-tag")
        for name, replacement in [
            ("src/lib.rs", b"bad"),
            (".cargo_vcs_info.json", b'{"git":{"sha1":"' + b"f" * 40 + b'"}}'),
            (".cargo_vcs_info.json", None),
        ]:
            dest = io.BytesIO()
            with (
                tarfile.open(fileobj=io.BytesIO(archive)) as old,
                tarfile.open(fileobj=dest, mode="w:gz") as new,
            ):
                for member in old:
                    data = old.extractfile(member).read()
                    if member.name.split("/", 1)[1] == name:
                        if replacement is None:
                            continue
                        data = replacement
                    member.size = len(data)
                    new.addfile(member, io.BytesIO(data))
            self.archive = dest.getvalue()
            self.publish("--recover-tag", code=1)
            self.assertEqual(self.git("ls-remote", "--tags", "origin"), "")
        self.archive = archive
        self.write("frontend", "release")
        self.commit()
        self.publish("--recover-tag")
        self.assertEqual(self.git("rev-parse", TAG + "^{commit}").strip(), self.initial)
        self.publish()
        self.assertEqual(self.uploads, 0)

    def test_competing_release_rechecks_core_tree_and_inherited_fields_after_upload_race(
        self,
    ):
        self.env["RACE_COMMIT"] = self.initial
        self.write("frontend", "release")
        self.commit()
        self.publish()
        self.assertEqual(self.uploads, 1)
        self.publish()
        self.assertEqual(self.uploads, 1)

    def test_competing_release_with_different_core_fails_closed(self):
        self.env["RACE_COMMIT"] = self.initial
        self.write(CORE + "/src/lib.rs", "pub fn different() {}")
        self.commit()
        self.publish(code=1, message="bump")

    def test_competing_upload_waits_for_tag_publication_without_reupload(self):
        self.status = 200
        worker = threading.Timer(0.5, self.tag)
        worker.start()
        self.addCleanup(worker.join)
        self.publish()
        self.assertEqual(self.uploads, 0)

    def test_unpublished_core_uploads_once_and_tags_head(self):
        self.publish()
        self.check(message="published")
        self.publish()
        self.assertEqual(self.uploads, 1)
        self.assertIn(self.initial, self.git("ls-remote", "--tags", "origin"))

    def test_release_tag_check_enforces_exact_version_agreement_and_classifies_prerelease_and_major_version(
        self,
    ):
        self.write("Cargo.toml", '[package]\nversion="0.9.0"\n')
        self.helper("check-tag", "cargo-crap", "v0.9.0", message="version=0.9.0")
        self.helper("check-tag", "cargo-crap", "v0.9.1", code=1, message="Cargo.toml")
        for version in ["0.1.0-rc.1", "0.1.0", "1.0.0", "2.0.0", "1.0.0-alpha-1"]:
            self.write("crates/crapts/Cargo.toml", f'[package]\nversion="{version}"\n')
            pkg = {
                "version": version,
                "optionalDependencies": dict.fromkeys(PACKAGES, version),
            }
            self.write("npm/crapts/package.json", json.dumps(pkg))
            output = self.base / "outputs"
            output.write_text("")
            self.env["GITHUB_OUTPUT"] = str(output)
            result = self.helper("check-tag", "crapts", "crapts-v" + version)
            self.assertEqual(
                result.splitlines(),
                [
                    f"version={version}",
                    "major=" + version.split(".")[0],
                    "is_prerelease=" + str("-" in version).lower(),
                ],
            )
            self.assertEqual(output.read_text().strip(), result)
        for tag in [
            "crapts-v1",
            "v1.0.0",
            "crapts-v00.1.0",
            "crapts-v1.0.0+x",
            "crapts-v1.0.0-01",
            "crapts-v1.0.0-a..b",
        ]:
            self.helper("check-tag", "crapts", tag, code=1)
        self.helper(
            "check-tag",
            "crapts",
            "crapts-v1.2.0",
            code=1,
            message="crates/crapts/Cargo.toml",
        )
        pkg["optionalDependencies"].pop(next(iter(PACKAGES)))
        self.write("npm/crapts/package.json", json.dumps(pkg))
        self.helper(
            "check-tag",
            "crapts",
            "crapts-v" + version,
            code=1,
            message="npm/crapts/package.json",
        )


if __name__ == "__main__":
    unittest.main()
