"""A stand-in HTTP API for the Action's scenario jobs.

Records every request, one JSON object per line, in the log file given.

  triage   answers every request with a 500, so a run that reaches it learns
           nothing and the test reads what was asked.
  github   a pull request's issue comments, kept in memory: GET lists them,
           POST adds one, PATCH edits one. `--existing BODY` starts with one
           comment, and `--forbid` refuses every write with a 403, as a
           fork's read-only token is refused.

Usage: python3 stub.py <port> <log> triage
       python3 stub.py <port> <log> github [--existing BODY] [--forbid]
"""
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

COMMENTS = []
FORBID = False
MODE = "triage"


class Stub(BaseHTTPRequestHandler):
    def reply(self, status: int, payload=None) -> None:
        data = json.dumps(payload).encode() if payload is not None else b""
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_one(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode("utf-8", "replace")
        with open(LOG, "a", encoding="utf-8") as log:
            log.write(json.dumps({"method": self.command, "path": self.path, "body": body}) + "\n")
        if MODE == "triage":
            return self.reply(500)
        path = self.path.split("?")[0]
        if self.command == "GET" and re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+/comments", path):
            return self.reply(200, COMMENTS)
        if FORBID:
            return self.reply(403, {"message": "Resource not accessible by integration"})
        if self.command == "POST" and re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+/comments", path):
            comment = {"id": 100 + len(COMMENTS), "body": json.loads(body)["body"]}
            COMMENTS.append(comment)
            return self.reply(201, comment)
        found = re.fullmatch(r"/repos/[^/]+/[^/]+/issues/comments/(\d+)", path)
        if self.command == "PATCH" and found:
            for comment in COMMENTS:
                if comment["id"] == int(found.group(1)):
                    comment["body"] = json.loads(body)["body"]
                    return self.reply(200, comment)
        return self.reply(404, {"message": "Not Found"})

    do_GET = do_POST = do_PATCH = handle_one

    def log_message(self, *_args) -> None:
        pass


if __name__ == "__main__":
    LOG = sys.argv[2]
    MODE = sys.argv[3] if len(sys.argv) > 3 else "triage"
    rest = sys.argv[4:]
    if "--existing" in rest:
        COMMENTS.append({"id": 7, "body": rest[rest.index("--existing") + 1]})
    FORBID = "--forbid" in rest
    HTTPServer(("127.0.0.1", int(sys.argv[1])), Stub).serve_forever()
