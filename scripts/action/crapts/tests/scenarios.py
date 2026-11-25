"""Fixture repositories and a provider stub for the crapts Action scenarios.

Run as a script by the scenario CI job, and imported by test_triage.py and
test_score.py so both exercise the same projects.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
from support import write_history

# Five shapes the normalizer cannot fold together, each written twice. Two
# copies of one shape form one group.
SHAPES = [
    "let t = 0; for (let i = 0; i < n; i++) {{ if (i % 2) {{ t += i * 3 }} else {{ t -= i }} }} return t + n * 2",
    "const out: number[] = []; let i = n; while (i > 0) {{ switch (i % 3) {{ case 0: out.push(i); break; "
    "case 1: out.unshift(i); break; default: out.pop() }} i-- }} return out.length",
    "try {{ return [n, n + 1, n + 2].map(v => v * 2).filter(v => v > 3).reduce((a, b) => a + b, 0) }} "
    "catch (e) {{ console.error(e); throw new Error(String(e)) }}",
    "const o: Record<string, number> = {{}}; o.a = n > 1 ? n : -n; o.b = n > 2 ? o.a * 2 : o.a / 2; "
    "o.c = o.a && o.b ? o.a - o.b : 0; return Object.keys(o).length + o.c",
    "if (n <= 1) {{ return 1 }} const [head, ...rest] = String(n).split(''); "
    "return Number(head) * {name}(n - 1) + rest.length",
]
# Only the base holds a pair of this shape, so a base run that could pay asks about it.
LEGACY = ("let k = n; do {{ k = k > 10 ? Math.floor(k / 2) : k * 3 + 1; if (k === 4) {{ break }} }} "
          "while (k !== 1 && k > 0); return k === 4 ? n : -n")
VERDICT = {
    "model": "stub",
    "answers": {
        "duplication_kind": {"type": "choice", "choice": "same_logic", "confidence": 0.9},
        "worth_extracting": {"type": "score", "score": 2.0, "confidence": 0.8},
        "divergence_risk": {"type": "noul", "noul": 0.6},
    },
    "usage": {},
}
# ${VAR+key} prints a marker when the variable exists, never its value.
COVER = """set -eu
echo "${TYPESAFE_API_KEY+key}${OPENAI_API_KEY+key}." >> "${TRACE:-/dev/null}"
for file in *.ts; do
  printf 'SF:%s\\n' "$file"; for line in $(seq 1 LINES); do printf 'DA:%s,1\\n' "$line"; done; echo end_of_record
done > coverage.info
"""


def shape(index, name):
    body = LEGACY if index is None else SHAPES[index]
    return f"export function {name}(n: number): number {{ " + body.format(name=name) + " }\n"


def budget_history(project, config=True):
    project.mkdir(parents=True, exist_ok=True)
    common = {"legacyAlpha.ts": shape(None, "legacyAlpha"), "legacyBeta.ts": shape(None, "legacyBeta"),
              "cover.sh": COVER.replace("LINES", "1")}
    if config:
        common[".crapts.toml"] = ("[duplicates]\nenabled = true\n\n"
                                  "[duplicates.triage]\nenabled = true\nmax-requests = 2\n")
    head = {"legacyBeta.ts": None}
    for index in range(len(SHAPES)):
        for copy in ["one", "two"]:
            name = f"group{index}{copy.capitalize()}"
            head[name + ".ts"] = shape(index, name)
    return write_history(project, common, head)


def signals_history(project):
    def f(last):
        return f"export function f() {{\n  let x = 0\n  x += 1\n  {last}\n}}\n"
    project.mkdir(parents=True, exist_ok=True)
    return write_history(project, {".crapts.toml": "[signals]\nenabled = true\n", "f.ts": f("return x"),
                                   "cover.sh": COVER.replace("LINES", "5")}, {"f.ts": f("debugger; return x")})


def duplicates_history(project):
    project.mkdir(parents=True, exist_ok=True)
    return write_history(project, {"keep.ts": "export function keep(n: number) { return n }\n"},
                         {"one.ts": shape(0, "one"), "two.ts": shape(0, "two")})


def serve(log, ready):
    class Provider(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length") or 0)).decode("utf-8", "replace")
            with open(log, "a", encoding="utf-8") as out:
                out.write(json.dumps({"path": self.path, "body": body}) + "\n")
            data = json.dumps(VERDICT).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    ready(f"http://127.0.0.1:{server.server_port}")
    return server


def requests(log):
    return [json.loads(line) for line in Path(log).read_text().splitlines()] if Path(log).exists() else []


def groups(binary, project):
    result = subprocess.run([str(binary), "--duplicates", "--format", "json"], cwd=project,
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)["duplicate_groups"]


def main(operation, directory):
    directory = Path(directory)
    if operation == "budget-setup":
        budget_history(directory)
    elif operation == "signals-setup":
        signals_history(directory)
    elif operation == "duplicates-setup":
        duplicates_history(directory)
    elif operation == "triage-stub":
        serve(directory / "triage.log", lambda url: (directory / "triage-url").write_text(url)).serve_forever()
    elif operation == "budget-check":
        asked = requests(directory / "triage.log")
        assert len(asked) == 2, asked
        assert not any("legacy" in request["body"] for request in asked), asked
        assert set((directory / "budget-trace").read_text().split()) == {"."}
    elif operation == "duplicates-check":
        report = directory.read_text(encoding="utf-8")
        assert "`one`" in report and "`two`" in report, report
    else:
        raise SystemExit("unknown operation " + operation)


if __name__ == "__main__":
    main(*sys.argv[1:3])
