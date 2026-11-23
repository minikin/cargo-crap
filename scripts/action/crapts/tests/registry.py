import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import sys
from urllib.parse import unquote
from support import BINARY, MANIFEST, WRAPPER, tarball


def serve(directory):
    directory.mkdir(parents=True, exist_ok=True)
    host = json.loads(subprocess.check_output(["node", "-e",
        "const r=require(process.argv[1]); console.log(JSON.stringify(r.resolveTarget(process.platform,process.arch,"
        "process.platform==='linux'?r.detectLibc():'none')))", str(WRAPPER / "lib/resolve.js")]))
    wrapper = MANIFEST
    files = {p.relative_to(WRAPPER).as_posix(): p.read_bytes() for folder in ["bin", "lib"] for p in (WRAPPER / folder).iterdir()}
    archives, docs = {}, {}
    class Registry(BaseHTTPRequestHandler):
        def do_GET(self):
            path = unquote(self.path.lstrip("/"))
            if path in archives:
                data = archives[path]
            elif path in docs:
                data = json.dumps(docs[path]).encode()
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Registry)
    url = "http://127.0.0.1:" + str(server.server_port)
    for manifest, package_files in [(wrapper, files), ({"name": host["name"], "version": wrapper["version"]},
                                                       {host["binary"]: BINARY.read_bytes()})]:
        name, version = manifest["name"], manifest["version"]
        archive = name.split("/")[1] + ".tgz"
        archives[archive] = tarball(manifest, package_files)
        docs[name] = {"name": name, "dist-tags": {"latest": version},
                      "versions": {version: {**manifest, "dist": {"tarball": url + "/" + archive}}}}
    (directory / "candidate wrapper.tgz").write_bytes(archives["crapts.tgz"])
    (directory / "registry-url").write_text(url)
    server.serve_forever()


if __name__ == "__main__":
    serve(Path(sys.argv[1]))
