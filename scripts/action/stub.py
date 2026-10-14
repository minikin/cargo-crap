"""A stand-in HTTP API for the Action's scenario jobs.

Records every request, one JSON object per line, in the log file given, and
answers every one with a 500, so a run that reaches it learns nothing and
the test reads what was asked.

Usage: python3 stub.py <port> <log>
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


class Recorder(BaseHTTPRequestHandler):
    def handle_one(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode("utf-8", "replace")
        with open(LOG, "a", encoding="utf-8") as log:
            log.write(json.dumps({"method": self.command, "path": self.path, "body": body}) + "\n")
        self.send_response(500)
        self.end_headers()

    do_GET = do_POST = do_PATCH = handle_one

    def log_message(self, *_args) -> None:
        pass


if __name__ == "__main__":
    LOG = sys.argv[2]
    HTTPServer(("127.0.0.1", int(sys.argv[1])), Recorder).serve_forever()
