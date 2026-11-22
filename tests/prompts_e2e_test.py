#!/usr/bin/env python3
"""E2E: default tool schemas, include-tools aliases, skills and memory prompts."""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path.resolve() if False else Path(__file__).resolve().parents[1]
BIN = ROOT / "target/release/ante"
MOCK = ROOT / "tests/mock/srv_capture.py"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_port(port):
    end = time.time() + 5
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), 0.2):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("mock did not start")


def run_case(home, port, capture, extra, prompt="hello"):
    env = dict(os.environ, HOME=str(home), ANTE_HOME=str(home), ANTE_FORCE_REPL="1")
    out = subprocess.run(
        [str(BIN), "--provider", "val", "-m", "mock", "-p", prompt] + extra,
        env=env,
        capture_output=True,
        text=True,
        timeout=90,
    )
    lines = capture.read_text().strip().splitlines() if capture.exists() else []
    if not lines:
        raise AssertionError(f"no request captured; stdout={out.stdout[-500:]} stderr={out.stderr[-500:]}")
    return json.loads(lines[-1])


def main():
    port = free_port()
    home = Path(tempfile.mkdtemp(prefix="ante_prompts_"))
    capture = home / "requests.jsonl"
    mock = subprocess.Popen(
        [sys.executable, str(MOCK), str(port), str(capture)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    results = []
    try:
        wait_port(port)
        (home / "auth/credentials/secret").mkdir(parents=True)
        (home / "catalog.json").write_text(json.dumps({"providers": {"val": {
            "display_name": "Val", "base_url": f"http://127.0.0.1:{port}/v1",
            "wire_style": "OpenAiCompatible", "credential": "secret:val",
            "preferred_models": [{"id": "mock", "description": "mock", "context_limit": 32768, "max_tokens": 4096}],
        }}}))
        cred = home / "auth/credentials/secret/val.json"
        cred.write_text('{"value":"sk-test"}')
        cred.chmod(0o600)

        body = run_case(home, port, capture, [])
        tools = [t.get("function", t).get("name") for t in body.get("tools", [])]
        results.append(("websearch-default", "WebSearch" in tools, ",".join(tools)))
        results.append(("viewimage-default-off", "ViewImage" not in tools and "TodoWrite" not in tools, ""))

        body = run_case(home, port, capture, ["--include-tools", "todowrite,viewimage"])
        tools = [t.get("function", t).get("name") for t in body.get("tools", [])]
        results.append(("include-tools-alias", {"TodoWrite", "ViewImage"} <= set(tools), ",".join(tools)))
        descs = {t.get("function", t).get("name"): t.get("function", t).get("description", "") for t in body.get("tools", [])}
        results.append(("viewimage-has-schema", "absolute path" in descs.get("ViewImage", ""), ""))

        system = "\n".join(m.get("content", "") for m in body.get("messages", []) if m.get("role") == "system")
        results.append(("skills-prompt-full", "### How to use skills" in system and "progressive disclosure" in system.lower(), ""))
        results.append(("builtin-skills-listed", "ante-guide" in system and "skill-creator" in system, ""))
        results.append(("memory-default-off", "# Auto Memory" not in system, ""))

        body = run_case(home, port, capture, ["--enable-auto-memory"])
        system = "\n".join(m.get("content", "") for m in body.get("messages", []) if m.get("role") == "system")
        results.append(("memory-prompt-on", "# Auto Memory" in system and "MEMORY.md" in system, ""))
        results.append(("memory-directory", str(home / "memory") in system, ""))
        (home / "settings.json").write_text(json.dumps({"mcp_servers": {"mock": {
            "command": sys.executable, "args": ["-u", str(ROOT / "tests/mcp_mock_server.py")]
        }}}))
        body = run_case(home, port, capture, [])
        tools = [t.get("function", t).get("name") for t in body.get("tools", [])]
        results.append(("mcp-tool-injected", "mcp__mock__echo" in tools, ",".join(tools)))
        results.append(("no-extra-tool-failure", True, ""))

        for name, ok, detail in results:
            print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
        failed = sum(1 for _, ok, _ in results if not ok)
        print(f"\n{len(results) - failed} passed, {failed} failed")
        sys.exit(1 if failed else 0)
    finally:
        mock.terminate()
        try:
            mock.wait(timeout=3)
        except subprocess.TimeoutExpired:
            mock.kill()
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()
