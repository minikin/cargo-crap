import argparse
import datetime
import json
import os
import re
import sys
import tempfile
import tomllib
from contextlib import ExitStack
from pathlib import Path

from release_version import ACTION, action_at, version_parts, wrapper_at

MANIFEST = Path("crates/crapts/Cargo.toml")
WRAPPER = Path("npm/crapts/package.json")
LOCK = Path("Cargo.lock")
LOG = Path("crates/crapts/CHANGELOG.md")
REPO = "https://github.com/minikin/cargo-crap"


def bump_table(text, header, old, version):
    sections = re.split(r"(?m)(?=^\[)", text)
    found = 0
    for i, section in enumerate(sections):
        if section.splitlines()[:1] != [header]:
            continue
        parsed = tomllib.loads(section)["package"]
        package = parsed[0] if isinstance(parsed, list) else parsed
        if package.get("name") != "crapts" or "source" in package:
            continue
        if package.get("version") != old:
            raise ValueError(f"{header} version differs from {MANIFEST}")
        sections[i], count = re.subn(
            r'(?m)^(version\s*=\s*)"' + re.escape(old) + '"',
            lambda m: m[1] + '"' + version + '"',
            section,
        )
        found += count
    if found != 1:
        raise ValueError(f"expected one local crapts version in {header}")
    return "".join(sections)


def changelog(text, version, date):
    pattern = r"(?ms)^## \[Unreleased\][ \t]*\n(.*?)(?=^## |^\[[^\]]+\]:|\Z)"
    section = re.search(pattern, text)
    if not section or not any(
        line.strip() and not line.lstrip().startswith("#")
        for line in section[1].splitlines()
    ):
        raise ValueError(f"{LOG} has nothing under [Unreleased] to release")
    if re.search(r"(?m)^## \[" + re.escape(version) + r"\]", text):
        raise ValueError(f"{LOG} already contains {version}")
    link = re.search(r"(?m)^\[Unreleased\]: (\S+)\s*$", text)
    tag = "crapts-v" + version
    previous = None
    if link:
        match = re.fullmatch(
            re.escape(REPO) + r"/compare/crapts-v([^/]+)\.\.\.HEAD", link[1]
        )
        if not match:
            raise ValueError(f"{LOG}: invalid Unreleased compare link")
        previous = match[1]
        version_parts(previous)
    notes = section[1].strip()
    body = f"## [Unreleased]\n\n## [{version}] - {date}\n\n{notes}\n\n"
    result = text[: section.start()] + body + text[section.end() :]
    release_link = (
        f"{REPO}/compare/crapts-v{previous}...{tag}"
        if previous
        else f"{REPO}/releases/tag/{tag}"
    )
    links = f"[Unreleased]: {REPO}/compare/{tag}...HEAD\n[{version}]: {release_link}\n"
    if link:
        return re.sub(
            r"(?m)^\[Unreleased\]: [^\n]*\n?", lambda _: links, result, count=1
        )
    return result.rstrip() + "\n\n" + links


def apply_changes(original, updated):
    with ExitStack() as cleanup:

        def stage(path, content):
            with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as temp:
                staged = Path(temp.name)
                cleanup.callback(staged.unlink, missing_ok=True)
                temp.write(content)
            staged.chmod(path.stat().st_mode)
            return staged

        ready = {p: stage(p, data) for p, data in updated.items()}
        backups = {p: stage(p, original[p]) for p in ready}
        written = []
        try:
            for path, staged in ready.items():
                if path.read_bytes() != original[path]:
                    raise ValueError(f"{path} changed during preparation")
                os.replace(staged, path)
                written.append(path)
            for path in written:
                if path.read_bytes() != updated[path]:
                    raise ValueError(f"{path}: edit verification failed")
        except BaseException:
            for path in reversed(written):
                os.replace(backups[path], path)
            raise


def prepare(version, date, prerelease):
    original = {p: p.read_bytes() for p in (MANIFEST, LOCK, WRAPPER, ACTION, LOG)}
    manifest = original[MANIFEST].decode()
    old = tomllib.loads(manifest)["package"]["version"]
    if old == version:
        raise ValueError(f"{MANIFEST} already says {version}")
    wrapper = wrapper_at(old)
    wrapper.update(
        version=version,
        optionalDependencies=dict.fromkeys(wrapper["optionalDependencies"], version),
    )
    updated = {
        ACTION: action_at(original[ACTION].decode(), old, version).encode(),
        MANIFEST: bump_table(manifest, "[package]", old, version).encode(),
        LOCK: bump_table(original[LOCK].decode(), "[[package]]", old, version).encode(),
        WRAPPER: (json.dumps(wrapper, indent=2) + "\n").encode(),
    }
    if not prerelease:
        updated[LOG] = changelog(original[LOG].decode(), version, date).encode()
    apply_changes(original, updated)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("date", nargs="?", default=datetime.date.today().isoformat())
    args = parser.parse_args()
    try:
        _, prerelease = version_parts(args.version)
        if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", args.date):
            raise ValueError("expected a date in YYYY-MM-DD format")
        datetime.date.fromisoformat(args.date)
    except ValueError as e:
        parser.error(str(e))
    try:
        prepare(args.version, args.date, prerelease)
    except (ValueError, OSError, KeyError) as e:
        sys.exit(str(e))
    print(
        f"Run just dev, open a PR, then tag the reviewed commit crapts-v{args.version}."
    )
