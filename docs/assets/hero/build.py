#!/usr/bin/env python3
"""Fill index.html with real cargo-crap output and write hero.html beside it.

    build.py <runs dir>

<runs dir> is what render.sh collects: human.ansi, pr-comment.md (with its
duplicate section), github.txt, shields.json, and the shop's sources before
and after the pull request in base/ and pr/. The PR comment is split at its
duplicate section into two panels. The PR comment goes
through GitHub's own Markdown renderer (`gh api /markdown`), and the badge is
rendered by scripts/badges/render.py, so the build needs `gh` and the network.
"""

import base64
import difflib
import importlib.util
import html
import json
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path

HERE = Path(__file__).resolve().parent
SGR = re.compile(r"\x1b\[([0-9;]*)m")
# The codes cargo-crap's human and summary renderers emit.
COLORS = {"31": "red", "33": "yellow", "38;5;9": "red", "38;5;10": "green", "38;5;11": "yellow"}


def ansi_to_html(text):
    """Turn SGR-coloured terminal text into spans, escaping everything else."""
    out, classes, pos = [], set(), 0

    def emit(chunk):
        if not chunk:
            return
        chunk = html.escape(chunk)
        out.append(f'<span class="{" ".join(sorted(classes))}">{chunk}</span>' if classes else chunk)

    for m in SGR.finditer(text):
        emit(text[pos:m.start()])
        pos = m.end()
        code = m.group(1)
        if code in ("", "0"):
            classes.clear()
        elif code == "39":
            classes -= {"red", "green", "yellow"}
        elif code == "1":
            classes.add("bold")
        elif code == "2":
            classes.add("dim")
        elif code == "31;1":
            classes |= {"red", "bold"}
        elif code in COLORS:
            classes -= {"red", "green", "yellow"}
            classes.add(COLORS[code])
    emit(text[pos:])
    return "".join(out)


def mark_row(rendered, name):
    """Wrap every terminal line naming `name` so the page can highlight it."""
    lines = rendered.split("\n")
    return "\n".join(f'<span class="travel">{line}</span>' if name in line else line for line in lines)


def gfm(markdown):
    body = json.dumps({"text": markdown, "mode": "gfm", "context": "minikin/cargo-crap"})
    return subprocess.run(
        ["gh", "api", "/markdown", "--input", "-"], input=body, capture_output=True, text=True, check=True
    ).stdout


def diff_panel(base, pr, start):
    """The annotated line, then the changed part of its function, as diff rows."""
    old, new = base.splitlines(), pr.splitlines()
    added = set()
    for tag, _, _, j1, j2 in difflib.SequenceMatcher(None, old, new).get_opcodes():
        if tag in ("insert", "replace"):
            added.update(range(j1, j2))

    def row(i):
        kind = "add" if i in added else "ctx"
        sign = "+" if kind == "add" else " "
        return (
            f'<div class="dl {kind}"><span class="ln">{i + 1}</span>'
            f'<span class="sg">{sign}</span><code>{html.escape(new[i]) or " "}</code></div>'
        )

    head = start - 1
    end = next(i for i in range(head, len(new)) if new[i] == "}")
    first = min(i for i in added if head < i <= end)
    hunk = [row(i) for i in range(max(head + 1, first - 1), end + 1)]
    skipped = first - 1 > head + 1
    divider = '<div class="dl hunk"><span class="ln"></span><span class="sg"></span><code>\u22ef</code></div>'
    return row(head), ([divider] if skipped else []) + hunk


def annotation(github_lines, name):
    for line in github_lines:
        if f"::{name} " in line:
            props, message = line[len("::warning "):].split("::", 1)
            fields = dict(part.split("=", 1) for part in props.split(","))
            return fields, urllib.parse.unquote(message)
    raise SystemExit(f"no annotation for {name}")


def badge(shields):
    """The CRAP pill the README shows, from the run's `--format shields`
    output, rendered by the same script as the README's badges. The pill
    follows prefers-color-scheme, which a screenshot cannot pick, so the page
    gets a light and a dark copy as images and shows the one for its theme."""
    spec = importlib.util.spec_from_file_location("render", HERE.parents[2] / "scripts" / "badges" / "render.py")
    render = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(render)
    svg = render.crap_badge(shields)
    dark_rules = re.search(r"@media \(prefers-color-scheme:dark\)\{(.*?\})\}", svg)
    light = svg[: dark_rules.start()] + svg[dark_rules.end():]
    dark = svg[: dark_rules.start()] + dark_rules.group(1) + svg[dark_rules.end():]
    image = lambda doc, theme: (
        f'<img class="pill-{theme}" alt="" src="data:image/svg+xml;base64,'
        f'{base64.b64encode(doc.encode()).decode()}">'
    )
    return image(light, "light") + image(dark, "dark")


def main(runs):
    runs = Path(runs)
    focus = "shipping_fee"
    human = mark_row(ansi_to_html(runs.joinpath("human.ansi").read_text()), focus)
    comment_md = runs.joinpath("pr-comment.md").read_text()
    split = re.search(r"^## \d+ duplicate candidates?$", comment_md, re.M)
    if split is None:
        raise SystemExit("the PR comment has no duplicate section")
    comment = gfm(comment_md[: split.start()]).replace("<details>", "<details open>")
    comment = re.sub(
        r"<tr>(?=(?:(?!</tr>).)*<code[^>]*>" + focus + "</code>)", '<tr class="travel">', comment, flags=re.S
    )
    duplicates = gfm(comment_md[split.start():])
    github_lines = runs.joinpath("github.txt").read_text().splitlines()
    fields, message = annotation(github_lines, focus)
    start = int(fields["line"])
    path = fields["file"].removeprefix("./")
    first_row, rows = diff_panel(
        runs.joinpath("base", path).read_text(), runs.joinpath("pr", path).read_text(), start
    )
    note = (
        '<div class="annot"><div class="annot-head"><span class="warn-icon">!</span>'
        f'<b>Check warning</b><span class="annot-title">{html.escape(fields["title"])}</span></div>'
        f'<div class="annot-msg">{html.escape(message)}</div></div>'
    )
    diff = first_row + note + "".join(rows)

    page = HERE.joinpath("index.html").read_text()
    for key, value in {
        "HUMAN": human,
        "DUPLICATES": duplicates,
        "COMMENT": comment,
        "DIFF": diff,
        "FILE": html.escape(fields["file"].removeprefix("./")),
        "BADGE": badge(json.loads(runs.joinpath("shields.json").read_text())),
    }.items():
        page = page.replace("{{" + key + "}}", value)
    HERE.joinpath("hero.html").write_text(page)


if __name__ == "__main__":
    main(sys.argv[1])
