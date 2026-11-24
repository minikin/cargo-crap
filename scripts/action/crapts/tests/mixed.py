import json
from pathlib import Path
import re
import sys
from urllib.request import urlopen
from support import write_history
from test_baseline import coverage, function


def setup(project):
    project.mkdir(parents=True)
    write_history(project, {"rust/Cargo.toml": '[package]\nname="shop"\nversion="0.1.0"\nedition="2021"\n[workspace]\n',
                     "rust/src/lib.rs": "pub fn shipping(n: u32) -> u32 { n }\n"},
                    {"rust/src/lib.rs": "pub fn shipping(n: u32) -> u32 { if n > 1 { n } else { 0 } }\n",
                     "ts/fee.ts": function("fee", 6)})


def hits(project, count):
    (project / "rust/hits.info").write_text(f"SF:src/lib.rs\nDA:1,{count}\nend_of_record\n")
    (project / "ts/hits.info").write_text(coverage({"fee": count}))


def check(url, after):
    with urlopen(url + "/repos/minikin/cargo-crap/issues/42/comments") as response:
        comments = json.load(response)
    assert len(comments) == 2, comments
    for item, tool, name, score in zip(comments, ["cargo-crap", "crapts"], ["shipping", "fee"],
                                       [6, 42] if after else [2, 6]):
        assert item["body"].splitlines()[0] == "<!-- " + tool + "-report -->"
        row = next(line for line in item["body"].splitlines() if "`" + name + "`" in line)
        assert re.search(r"\b" + str(score) + r"\.0\b", row), row
    return [comment["id"] for comment in comments]


if __name__ == "__main__":
    operation, directory = sys.argv[1:3]
    project = Path(directory)
    if operation == "setup":
        setup(project)
        hits(project, 1)
    else:
        ids = check(sys.argv[3], operation == "check")
        snapshot = project.parent / "comment-ids.json"
        if operation == "update":
            snapshot.write_text(json.dumps(ids))
            hits(project, 0)
        else:
            assert ids == json.loads(snapshot.read_text())
