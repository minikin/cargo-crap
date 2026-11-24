import json
import os
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parents[1]
CRATE = Path("crates/crapts")
CORE = Path("crates/cargo-crap-core")
MANIFEST = CRATE / "Cargo.toml"
CHANGELOG = CRATE / "CHANGELOG.md"
WRAPPER = Path("npm/crapts/package.json")
LOCK = Path("Cargo.lock")
ACTION = Path("actions/crapts/action.yml")
CHANGED = [MANIFEST, CHANGELOG, WRAPPER, LOCK, ACTION]
ISOLATED = [
    Path("Cargo.toml"),
    Path("CHANGELOG.md"),
    Path("action.yml"),
    CORE / "Cargo.toml",
    CORE / "CHANGELOG.md",
]


class PrepareRelease(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        for path in CHANGED + ISOLATED:
            (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / path, self.repo / path)
        self.original_action = self.read(ACTION)
        self.old = tomllib.loads(self.read(MANIFEST).decode())["package"]["version"]
        self.dependencies = set(json.loads(self.read(WRAPPER))["optionalDependencies"])
        self.next = "0.2.0" if self.old != "0.2.0" else "0.3.0"
        self.changes = "## [Unreleased]\n\n### Added\n\n- Check reports.\n"
        self.write(CHANGELOG, "# Changelog\n\n" + self.changes)
        self.original = self.snapshot(ISOLATED)

    def write(self, path, text):
        (self.repo / path).write_text(text)

    def read(self, path):
        return (self.repo / path).read_bytes()

    def snapshot(self, paths):
        return {path: self.read(path) for path in paths}

    def prep(self, *args, code=0):
        res = subprocess.run(
            ["bash", str(SCRIPTS / "prep-crapts.sh"), *args],
            cwd=self.repo,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(res.returncode, code, (args, res.stdout, res.stderr))
        if code:
            self.assertTrue(res.stderr)
        else:
            for item in ["just dev", "PR", "crapts-v" + args[0]]:
                self.assertIn(item, res.stdout)

    def versions(self, version, before):
        self.assertEqual(
            tomllib.loads(self.read(MANIFEST).decode())["package"]["version"], version
        )
        old_pin = f'    default: "{self.old}"'.encode()
        new_pin = f'    default: "{version}"'.encode()
        self.assertEqual(self.original_action.count(old_pin), 1)
        self.assertEqual(self.read(ACTION), self.original_action.replace(old_pin, new_pin, 1))
        npm = json.loads(self.read(WRAPPER))
        self.assertEqual(npm["version"], version)
        self.assertEqual(
            npm["optionalDependencies"], dict.fromkeys(self.dependencies, version)
        )
        parsed = tomllib.loads(before.decode())
        (pkg,) = [
            pkg
            for pkg in parsed["package"]
            if pkg["name"] == "crapts" and "source" not in pkg
        ]
        pkg["version"] = version
        self.assertEqual(tomllib.loads(self.read(LOCK).decode()), parsed)
        self.assertEqual(self.snapshot(ISOLATED), self.original)

    def test_release_prep_bumps_crapts_manifests_lock_and_changelog_without_touching_cargo_crap_or_core(
        self,
    ):
        lock = self.read(LOCK)
        self.prep(self.next, "2031-02-03")
        self.versions(self.next, lock)
        first = (
            f"https://github.com/minikin/cargo-crap/releases/tag/crapts-v{self.next}"
        )
        log = self.read(CHANGELOG).decode()
        self.assertIn(f"## [Unreleased]\n\n## [{self.next}] - 2031-02-03", log)
        self.assertIn(f"[{self.next}]: {first}", log)
        self.assertIn(f"/compare/crapts-v{self.next}...HEAD", log)
        self.write(CHANGELOG, log.replace("## [Unreleased]", self.changes.rstrip()))
        self.prep("9.0.0-rc.1", "2032-02-29")
        self.prep("9.0.0", "2032-03-01")
        final = self.read(CHANGELOG).decode()
        self.assertIn(f"/compare/crapts-v{self.next}...crapts-v9.0.0", final)
        self.assertIn(f"[{self.next}]: {first}", final)
        self.assertNotIn("rc.1", final)

    def test_release_prep_supports_prerelease_rehearsals_without_draining_unreleased_and_keeps_locked_builds_working(
        self,
    ):
        for directory in [Path("src"), CORE / "src", CRATE / "src"]:
            shutil.copytree(ROOT / directory, self.repo / directory)
        env = dict(
            os.environ,
            CARGO_NET_OFFLINE="true",
            CARGO_TARGET_DIR=str(self.repo / "target"),
        )
        changelog = self.read(CHANGELOG)
        lock = self.read(LOCK)
        for version in [self.next + "-rc.1", self.next]:
            self.prep(version, "2031-02-03")
            self.versions(version, lock)
            if version.endswith("-rc.1"):
                self.assertEqual(self.read(CHANGELOG), changelog)
            build = subprocess.run(
                ["cargo", "build", "--locked", "-p", "crapts"],
                cwd=self.repo,
                env=env,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(build.returncode, 0, build.stderr)
            tool = self.repo / "target/debug/crapts"
            self.assertEqual(
                subprocess.check_output([tool, "--version"], text=True).strip(),
                "crapts " + version,
            )

    def test_release_prep_rejects_invalid_arguments_and_empty_stable_changelogs_without_modifying_files(
        self,
    ):
        original = self.snapshot(CHANGED)
        for args in [
            [],
            ["v0.2.0"],
            ["crapts-v0.2.0"],
            ["01.2.3"],
            ["1.2.03"],
            ["0.1.0-rc..1"],
            ["0.1.0-rc.01"],
            ["0.1.0+test"],
            [self.next, "03/02/2031"],
            [self.next, "2031-02-29"],
            [self.next, "2031-02-03", "extra"],
        ]:
            self.prep(*args, code=2)
            self.assertEqual(self.snapshot(CHANGED), original)
        self.prep(self.old, code=1)
        self.assertEqual(self.snapshot(CHANGED), original)
        self.write(CHANGELOG, "# Changelog\n\n## [Unreleased]\n\n### Added\n")
        before = self.snapshot(CHANGED)
        self.prep(self.next, code=1)
        self.assertEqual(self.snapshot(CHANGED), before)
        self.write(
            LOCK,
            self.read(LOCK).decode().replace('name = "crapts"', 'name = "missing"'),
        )
        before = self.snapshot(CHANGED)
        self.prep(self.next + "-rc.2", code=1)
        self.assertEqual(self.snapshot(CHANGED), before)


if __name__ == "__main__":
    unittest.main()
