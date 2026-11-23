import json
import os
import re
import sys
import tomllib
from pathlib import Path

PLATFORMS = {
    "@minikin/crapts-" + suffix
    for suffix in (
        "darwin-arm64",
        "darwin-x64",
        "linux-x64-gnu",
        "linux-arm64-gnu",
        "linux-x64-musl",
        "win32-x64-msvc",
    )
}
NUMBER = r"(?:0|[1-9][0-9]*)"
SEMVER = re.compile(
    rf"({NUMBER})\.{NUMBER}\.{NUMBER}(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)


def version_parts(version):
    match = SEMVER.fullmatch(version)
    if not match or any(
        len(i) > 1 and i.isdigit() and i.startswith("0")
        for i in (match[2] or "").split(".")
    ):
        raise ValueError(f"invalid SemVer: {version}")
    return match[1], bool(match[2])


def manifest_version(path):
    return tomllib.loads(Path(path).read_text())["package"]["version"]


def wrapper_at(version):
    path = Path("npm/crapts/package.json")
    wrapper = json.loads(path.read_bytes())
    if wrapper["version"] != version or wrapper.get(
        "optionalDependencies"
    ) != dict.fromkeys(PLATFORMS, version):
        raise ValueError(f"{path} does not match {version}")
    return wrapper


def check_tag(product, tag):
    path, prefix = {
        "cargo-crap": ("Cargo.toml", "v"),
        "crapts": ("crates/crapts/Cargo.toml", "crapts-v"),
    }[product]
    if not tag.startswith(prefix):
        raise ValueError(f"{tag}: expected {prefix} prefix")
    version = tag[len(prefix) :]
    major, prerelease = version_parts(version)
    if manifest_version(path) != version:
        raise ValueError(f"{path} does not match {tag}")
    if product == "crapts":
        wrapper_at(version)
    output = (
        f"version={version}\nmajor={major}\nis_prerelease={str(prerelease).lower()}\n"
    )
    sys.stdout.write(output)
    if dest := os.getenv("GITHUB_OUTPUT"):
        with open(dest, "a") as f:
            f.write(output)


if __name__ == "__main__":
    try:
        check_tag(*sys.argv[1:])
    except (KeyError, TypeError, ValueError, OSError) as e:
        sys.exit(str(e))
