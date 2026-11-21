#!/usr/bin/env python3
"""审批交互端到端：会话级授权（选项 2）与永久授权（选项 3）+ 泛化匹配。"""
import json, os, socket, subprocess, sys, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target/release/ante"
MOCK = ROOT / "tests/mock/srv_tool.py"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0)); return s.getsockname()[1]


def wait_port(port):
    end = time.time() + 5
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), 0.2): return
        except OSError: time.sleep(0.05)
    raise RuntimeError("mock did not start")


def make_home(port):
    home = Path(tempfile.mkdtemp(prefix="ante_appr_"))
    (home / "auth/credentials/secret").mkdir(parents=True)
    (home / "auth/credentials/secret/val.json").write_text('{"value":"sk-test"}')
    os.chmod(home / "auth/credentials/secret/val.json", 0o600)
    catalog = {"providers": {"val": {
        "display_name": "Val", "base_url": f"http://127.0.0.1:{port}/v1",
        "wire_style": "OpenAiCompatible", "credential": "secret:val",
        "preferred_models": [{"id": "mock", "description": "mock", "context_limit": 32768, "max_tokens": 4096}]}}}
    (home / "catalog.json").write_text(json.dumps(catalog))
    (home / "settings.json").write_text('{"permission_mode":"strict"}')
    return home


def run(home, port, lines):
    env = dict(os.environ, HOME=str(home), ANTE_HOME=str(home), ANTE_FORCE_REPL="1", COLUMNS="100")
    p = subprocess.run([str(BIN), "--provider", "val", "-m", "mock"], input="".join(lines),
                       capture_output=True, text=True, env=env, timeout=40)
    return p


def main():
    port = free_port()
    mock = subprocess.Popen([sys.executable, str(MOCK), str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    results = []
    try:
        wait_port(port)
        # 1) 选项 2：本会话放行，且泛化为 Bash(echo hi *)
        home = make_home(port)
        p = run(home, port, ["run echo hi\n", "2\n", "/exit\n"])
        out = p.stdout + p.stderr
        ok = p.returncode == 0 and "2 Yes, allow `Bash(echo hi *)` for this session" in out \
            and "Allowed for this session: Bash(echo hi *)" in out and "tool-said: status: exited" in p.stdout
        results.append(("approval-session-grant", ok, out[-400:]))
        # 2) 选项 3：写入 settings.permissions.allow
        home2 = make_home(port)
        p = run(home2, port, ["run echo hi\n", "3\n", "/exit\n"])
        settings = json.loads((home2 / "settings.json").read_text())
        allow = settings.get("permissions", {}).get("allow", [])
        ok = p.returncode == 0 and any(a.startswith("Bash(echo hi") for a in allow)
        results.append(("approval-persist-rule", ok, f"allow={allow}"))
        # 3) 同一 home 再跑：命中持久规则，不再弹审批
        p = run(home2, port, ["run echo hi\n", "/exit\n"])
        ok = p.returncode == 0 and "Do you want to proceed?" not in (p.stdout + p.stderr) and "tool-said: status: exited" in p.stdout
        results.append(("approval-rule-skips-prompt", ok, p.stdout[-280:]))
        # 4) 拒绝（选项 4）：不执行工具，模型仍拿到拒绝结果
        home3 = make_home(port)
        p = run(home3, port, ["run echo hi\n", "4\n", "/exit\n"])
        out = p.stdout + p.stderr
        ok = p.returncode == 0 and "denied" in p.stdout.lower() and "status: exited" not in p.stdout
        results.append(("approval-deny", ok, out[-320:]))
    finally:
        mock.terminate()
    failed = 0
    for name, ok, detail in results:
        print(f"[{'PASS' if ok else 'FAIL'}] {name}" + ("" if ok else f"\n{detail}"))
        failed += 0 if ok else 1
    print(f"\n{len(results)-failed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
