"""A stand-in HTTP API for the Action's scenario jobs.

Records every request, one JSON object per line, in the log file given.

  triage   answers every request with a 500, so a run that reaches it learns
           nothing and the test reads what was asked.
  verdicts answers every request with the same TypeSafe verdict, so a run
           caches what it asked and the test counts the requests.
  github   a pull request's issue comments, kept in memory: GET lists them,
           POST adds one, PATCH edits one. The token is the account
           fixture-bot: GET /user says so, and every comment it writes or
           starts with carries it as its `user`. `--existing BODY` starts with
           one such comment. `--foreign-bot BODY` puts one by another bot
           before it. `--anonymous` refuses GET /user and the GraphQL viewer
           query with a 403, as an installation token is refused. `--forbid`
           refuses every write with a 403, as a fork's read-only token is
           refused. `--reject-over N` refuses a body over N characters with
           a 422, as GitHub refuses one over its limit.

Usage: python3 stub.py <port> <log> triage
       python3 stub.py <port> <log> verdicts
       python3 stub.py <port> <log> github [--existing BODY] [--foreign-bot BODY]
                                           [--anonymous] [--forbid] [--reject-over N]
"""
import json
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

VERDICT = {
    "model": "stub",
    "answers": {
        "duplication_kind": {"type": "choice", "choice": "same_logic", "confidence": 0.9},
        "worth_extracting": {"type": "score", "score": 2.0, "confidence": 0.8},
        "divergence_risk": {"type": "noul", "noul": 0.6},
    },
    "usage": {},
}
COMMENTS = []
FORBID = False
ANONYMOUS = False
REJECT_OVER = None
USER = {"login": "fixture-bot", "type": "Bot"}
OTHER_BOT = {"login": "other-bot[bot]", "type": "Bot"}
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
        if MODE == "verdicts":
            return self.reply(200, VERDICT)
        path = self.path.split("?")[0]
        if path == "/user" or path == "/graphql":
            if ANONYMOUS:
                return self.reply(403, {"message": "Resource not accessible by integration"})
            if path == "/user":
                return self.reply(200, USER)
            return self.reply(200, {"data": {"viewer": {"login": USER["login"]}}})
        if self.command == "GET" and re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+/comments", path):
            return self.reply(200, COMMENTS)
        if FORBID:
            return self.reply(403, {"message": "Resource not accessible by integration"})
        if REJECT_OVER is not None and len(json.loads(body)["body"]) > REJECT_OVER:
            return self.reply(422, {"message": "Validation Failed"})
        if self.command == "POST" and re.fullmatch(r"/repos/[^/]+/[^/]+/issues/\d+/comments", path):
            comment = {"id": 100 + len(COMMENTS), "body": json.loads(body)["body"], "user": USER}
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
    if "--foreign-bot" in rest:
        COMMENTS.append({"id": 6, "body": rest[rest.index("--foreign-bot") + 1], "user": OTHER_BOT})
    if "--existing" in rest:
        COMMENTS.append({"id": 7, "body": rest[rest.index("--existing") + 1], "user": USER})
    FORBID = "--forbid" in rest
    ANONYMOUS = "--anonymous" in rest
    if "--reject-over" in rest:
        REJECT_OVER = int(rest[rest.index("--reject-over") + 1])
    HTTPServer(("127.0.0.1", int(sys.argv[1])), Stub).serve_forever()
