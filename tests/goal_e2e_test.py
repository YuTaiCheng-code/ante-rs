#!/usr/bin/env python3
"""目标闭环端到端测试：判定器第一次 not met、第二次 met，验证自动续轮与收敛。"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target/release/ante"
MOCK = ROOT / "tests/mock/srv_goal.py"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_port(port, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), 0.2):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError(f"mock did not start on {port}")


def main():
    port = free_port()
    proc = subprocess.Popen([sys.executable, str(MOCK), str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    try:
        wait_port(port)
        home = Path(tempfile.mkdtemp(prefix="ante_goal_e2e_"))
        (home / "auth/credentials/secret").mkdir(parents=True)
        (home / "catalog.json").write_text(json.dumps({
            "providers": {"val": {
                "display_name": "Val",
                "base_url": f"http://127.0.0.1:{port}/v1",
                "wire_style": "OpenAiCompatible",
                "credential": "secret:val",
                "preferred_models": [{"id": "mock", "description": "mock", "context_limit": 32768, "max_tokens": 4096}],
            }}
        }))
        cred = home / "auth/credentials/secret/val.json"
        cred.write_text('{"value":"sk-test"}')
        cred.chmod(0o600)

        env = dict(os.environ, HOME=str(home), ANTE_HOME=str(home), ANTE_FORCE_REPL="1", COLUMNS="100")
        stdin = "/goal all tests pass\nplease work\n/exit\n"
        out = subprocess.run(
            [str(BIN), "--provider", "val", "-m", "mock"],
            input=stdin, env=env, capture_output=True, text=True, timeout=120,
        )
        text = out.stdout + out.stderr

        checks = [
            ("goal-set", "🎯 Goal set: all tests pass" in text),
            ("iteration-line", "🔁 Goal not yet met (iteration 1/10)" in text),
            ("reason-propagated", "no test output in the conversation yet" in text),
            ("met-line", "✅ Goal met: the test output shows all tests passing" in text),
            ("auto-continue", text.count("❖ Ante") >= 2),
        ]
        for name, ok in checks:
            print(f"[{'PASS' if ok else 'FAIL'}] {name}")
        if not all(ok for _, ok in checks):
            print(text[-1500:])
        failed = sum(1 for _, ok in checks if not ok)
        print(f"\n{len(checks) - failed} passed, {failed} failed")
        sys.exit(1 if failed else 0)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    main()
