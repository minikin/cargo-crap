import argparse
import io
import json
import re
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from release_version import manifest_version

CORE = "crates/cargo-crap-core"
MANIFEST = CORE + "/Cargo.toml"
AGENT = "cargo-crap release (https://github.com/minikin/cargo-crap)"
BACKOFF = (0, 1, 2, 4, 8, 16)


def git(*args, cwd=None):
    return subprocess.check_output(["git", *args], text=True, cwd=cwd).strip()


def git_status(*args):
    return subprocess.run(
        ["git", *args], stdout=subprocess.DEVNULL, check=False
    ).returncode


def inherited_values(ref):
    member = tomllib.loads(git("show", f"{ref}:{MANIFEST}"))
    workspace = tomllib.loads(git("show", f"{ref}:Cargo.toml"))["workspace"]
    fields = {
        k: workspace["package"][k] for k, v in member["package"].items() if inherits(v)
    }
    deps = {}
    for tables in [member, *member.get("target", {}).values()]:
        for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
            for name, spec in tables.get(kind, {}).items():
                if inherits(spec):
                    deps[name] = workspace["dependencies"][name]
    lints = workspace["lints"] if inherits(member.get("lints")) else {}
    return fields, deps, lints


def inherits(value):
    return isinstance(value, dict) and value.get("workspace") is True


def request(url):
    return urlopen(Request(url, headers={"User-Agent": AGENT}), timeout=30)


class MissingTag(ValueError):
    pass


class Core:
    def __init__(self, api, remote):
        self.version = manifest_version(MANIFEST)
        self.tag = "cargo-crap-core-v" + self.version
        self.ref = "refs/tags/" + self.tag
        self.url = api.rstrip("/") + "/crates/cargo-crap-core/" + self.version
        self.remote = remote
        self.has_remote = remote in git("remote").splitlines()

    def registry_status(self):
        try:
            with request(self.url) as response:
                status = response.status
        except HTTPError as error:
            status = error.code
        if status not in (200, 404):
            raise ValueError(f"{self.url}: HTTP {status}")
        return status

    def tags(self):
        remote_tag = False
        if self.has_remote:
            status = git_status(
                "ls-remote", "--exit-code", "--refs", self.remote, self.ref
            )
            if status == 0:
                git("fetch", "--no-tags", self.remote, self.ref + ":" + self.ref)
                remote_tag = True
            elif status != 2:
                raise ValueError(f"cannot query {self.tag} on {self.remote}")
        local_tag = git_status("show-ref", "--verify", "--quiet", self.ref) == 0
        return local_tag, remote_tag

    def check(self):
        local, remote = self.tags()
        status = self.registry_status()
        if status == 404:
            if local or remote:
                raise ValueError(
                    f"{self.tag} conflicts with HTTP 404; refusing to upload"
                )
            return "unpublished"
        if not local or (self.has_remote and not remote):
            raise MissingTag(f"missing {self.tag}; use publish-core.sh --recover-tag")
        if git_status("diff", "--quiet", self.ref, "HEAD", "--", CORE):
            raise ValueError(
                f"{self.tag}: core tree changed; bump cargo-crap-core version"
            )
        if inherited_values(self.ref) != inherited_values("HEAD"):
            raise ValueError(
                f"{self.tag}: inherited manifest changed; bump cargo-crap-core version"
            )
        return "published"

    def push_tag(self, commit):
        if (
            git_status("show-ref", "--verify", "--quiet", self.ref) == 0
            and git("rev-parse", self.ref + "^{commit}") != commit
        ):
            raise ValueError(f"{self.tag} already points at a different commit")
        git("push", self.remote, commit + ":" + self.ref)
        return self.check()

    def publish(self):
        try:
            if self.check() == "published":
                return "published"
        except MissingTag:
            return self.await_competitor()
        self.require_remote()
        upload = subprocess.run(
            ["cargo", "publish", "--locked", "-p", "cargo-crap-core"], check=False
        )
        if upload.returncode:
            return self.await_competitor()
        for delay in BACKOFF:
            time.sleep(delay)
            if self.registry_status() == 200:
                try:
                    return self.push_tag(git("rev-parse", "HEAD"))
                except subprocess.CalledProcessError:
                    return self.await_competitor()
        raise ValueError(
            f"{self.tag}: upload visibility timeout; use --recover-tag if published"
        )

    def await_competitor(self):
        for delay in BACKOFF:
            time.sleep(delay)
            try:
                if self.check() == "published":
                    return "published"
            except MissingTag:
                continue
        raise ValueError(
            f"{self.tag}: publication incomplete; use --recover-tag if published"
        )

    def require_remote(self):
        if not self.has_remote:
            raise ValueError(f"cannot publish without configured remote {self.remote}")

    def recover(self):
        self.require_remote()
        try:
            if self.check() == "published":
                return "published"
            raise ValueError(f"{self.tag}: cannot recover an unpublished crate")
        except MissingTag:
            pass
        with request(self.url + "/download") as response:
            published = self.contents(io.BytesIO(response.read()))
        vcs = json.loads(published[".cargo_vcs_info.json"])
        commit = vcs["git"]["sha1"]
        if vcs["git"].get("dirty") or not re.fullmatch(
            "[a-f0-9]{40}|[a-f0-9]{64}", commit
        ):
            raise ValueError(f"{self.tag}: invalid source commit metadata")
        if git("cat-file", "-t", commit) != "commit":
            raise ValueError(f"{self.tag}: unknown source commit {commit}")
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp) / "source"
            git(
                "clone",
                "--quiet",
                "--no-checkout",
                "--no-local",
                git("rev-parse", "--show-toplevel"),
                str(checkout),
            )
            git("checkout", "--quiet", "--detach", commit, cwd=checkout)
            target = Path(tmp) / "target"
            subprocess.run(
                [
                    "cargo",
                    "package",
                    "--locked",
                    "-p",
                    "cargo-crap-core",
                    "--target-dir",
                    str(target),
                ],
                cwd=checkout,
                check=True,
            )
            path = target / "package" / f"cargo-crap-core-{self.version}.crate"
            with path.open("rb") as file:
                if self.contents(file) != published:
                    raise ValueError(
                        f"{self.tag}: packaged contents differ at {commit}"
                    )
        return self.push_tag(commit)

    def contents(self, file):
        prefix = f"cargo-crap-core-{self.version}/"
        files = {}
        with tarfile.open(fileobj=file, mode="r:gz") as archive:
            for entry in archive:
                if not entry.isfile() or not entry.name.startswith(prefix):
                    raise ValueError(f"unexpected crate entry {entry.name}")
                name = entry.name[len(prefix) :]
                if name in files:
                    raise ValueError(f"duplicate crate entry {name}")
                files[name] = archive.extractfile(entry).read()
        return files


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["check", "publish"])
    parser.add_argument("--api-base", default="https://crates.io/api/v1")
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--recover-tag", action="store_true")
    args = parser.parse_args()
    try:
        core = Core(args.api_base, args.remote)
        if args.operation == "check":
            print(core.check())
        elif args.recover_tag:
            print(core.recover())
        else:
            print(core.publish())
    except (
        KeyError,
        ValueError,
        OSError,
        tarfile.TarError,
        subprocess.CalledProcessError,
    ) as e:
        sys.exit(f"cargo-crap-core: {e}")
