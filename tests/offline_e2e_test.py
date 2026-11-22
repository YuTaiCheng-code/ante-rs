#!/usr/bin/env python3
"""Offline mode end-to-end: install from a local artifact, start the router,
send one prompt through the OpenAI-compatible endpoint, then stop it.

No network access is used: ANTE_LLAMA_ARTIFACT points at a fake tar.gz and
ANTE_LLAMA_ALLOW_UNVERIFIED=1 bypasses the pinned digest (test only).
"""
import json, os, subprocess, sys, tarfile, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("ANTE_BIN", ROOT / "target/release/ante"))

SERVER = r'''#!/usr/bin/env python3
import sys, json
from http.server import BaseHTTPRequestHandler, HTTPServer
args = sys.argv[1:]
if "--version" in args:
    print("llama-server b12182"); sys.exit(0)
def arg(name, default=None):
    return args[args.index(name) + 1] if name in args else default
port = int(arg("--port", "18080")); alias = arg("--alias", "offline-model")
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        if self.path == "/health": body = b'{"status":"ok"}'
        elif self.path == "/v1/models": body = json.dumps({"data": [{"id": alias}]}).encode()
        else:
            self.send_response(404); self.end_headers(); return
        self.send_response(200); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0")); self.rfile.read(n)
        body = b'data: {"choices":[{"delta":{"content":"pong"}}]}\n\ndata: [DONE]\n\n'
        self.send_response(200); self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
HTTPServer(("127.0.0.1", port), H).serve_forever()
'''


def env(home):
    e = dict(os.environ, HOME=str(home), ANTE_HOME=str(home))
    e.update({
        "ANTE_OFFLINE_DIR": str(home / "offline"),
        "ANTE_LLAMA_ARTIFACT": str(home / "fake.tar.gz"),
        "ANTE_LLAMA_ALLOW_UNVERIFIED": "1",
        "ANTE_OFFLINE_START_TIMEOUT": "15",
    })
    return e


def run(home, *args, timeout=40):
    return subprocess.run([str(BIN), *args], capture_output=True, text=True, env=env(home), timeout=timeout)


def main():
    home = Path(tempfile.mkdtemp(prefix="ante_offline_"))
    (home / "models").mkdir()
    (home / "models" / "fake.gguf").write_bytes(b"GGUF")
    stage = home / "stage" / "llama-b12182"
    stage.mkdir(parents=True)
    server = stage / "llama-server"
    server.write_text(SERVER); server.chmod(0o755)
    with tarfile.open(home / "fake.tar.gz", "w:gz") as t:
        t.add(stage, arcname="llama-b12182")

    p = run(home, "offline", "install")
    assert p.returncode == 0, p.stderr
    assert "Installation complete!" in p.stderr, p.stderr
    assert (home / "offline" / "llama-server").is_file()

    p = run(home, "offline", "status", "--json")
    status = json.loads(p.stdout)
    assert status["engine"] == "installed", status

    p = run(home, "offline", "use", "fake")
    assert p.returncode == 0, p.stderr

    p = run(home, "--offline-model", "--prompt", "hi")
    assert p.returncode == 0, (p.stdout, p.stderr)
    assert "pong" in p.stdout, p.stdout

    p = run(home, "offline", "status", "--json")
    status = json.loads(p.stdout)
    assert status["router"] == "running", status

    p = run(home, "offline", "stop")
    assert p.returncode == 0, p.stderr
    assert "Stopped the offline llama.cpp router" in p.stdout, p.stdout

    p = run(home, "offline", "status", "--json")
    status = json.loads(p.stdout)
    assert status["router"] == "stopped", status
    print("offline e2e: 6/6 passed")


if __name__ == "__main__":
    main()
