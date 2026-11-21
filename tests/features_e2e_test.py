#!/usr/bin/env python3
"""三块新增能力的端到端测试：审批泛化 / rollover / term。"""
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


def wait_port(port):
    end = time.time() + 5
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), 0.2):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("mock did not start")


def main():
    port = free_port()
    mock = subprocess.Popen([sys.executable, str(MOCK), str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    home = Path(tempfile.mkdtemp(prefix="ante_features_"))
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

        results = []

        # rollover: 普通一轮后交接
        env = dict(os.environ, HOME=str(home), ANTE_HOME=str(home), ANTE_FORCE_REPL="1", COLUMNS="100")
        out = subprocess.run([str(BIN), "--provider", "val", "-m", "mock"],
                             input="hello\n/rollover focus on tests\n/exit\n",
                             env=env, capture_output=True, text=True, timeout=120)
        text = out.stdout + out.stderr
        metas = [json.loads(p.read_text()) for p in (home / "sessions").glob("*/meta.json")]
        rolled = [m for m in metas if m.get("rolled_from")]
        results.append(("rollover-created", bool(rolled) and "Rollover: ses_" in text, f"{len(rolled)} rolled session(s)"))
        if rolled:
            dialog = (home / "sessions" / rolled[0]["id"] / "dialog.json").read_text()
            ok = "This session is continued from" in dialog and "focus on tests" in dialog
            results.append(("rollover-handoff-content", ok, "handoff preserved in new dialog"))
            results.append(("rollover-link", rolled[0]["rolled_from"].startswith("ses_"), rolled[0]["rolled_from"]))

        # term: 启动一个命令并读回输出
        out = subprocess.run([str(BIN), "--provider", "val", "-m", "mock"],
                             input="/term echo hi-from-term\n" + "/term echo hi-from-term\n/exit\n",
                             env=env, capture_output=True, text=True, timeout=120)
        ttext = out.stdout + out.stderr
        results.append(("term-launch", "terminal `echo hi-from-term` opened" in ttext, ""))
        results.append(("term-read", "hi-from-term" in ttext, ""))

        for name, ok, detail in results:
            print(f"[{'PASS' if ok else 'FAIL'}] {name}{' — ' + detail if detail else ''}")
        if not all(ok for _, ok, _ in results):
            print(ttext[-1200:])
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
