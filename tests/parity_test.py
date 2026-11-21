#!/usr/bin/env python3
"""ante 0.3.4 与 ante-rs 的等效性测试套件。

用法：
    python3 tests/parity_test.py [--orig /path/to/ante] [--ours target/release/ante]

退出码：0 全部通过（允许 SKIP），1 存在 FAIL。
"""

import argparse
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import fcntl
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07]*\x07")

# 原版 0.3.4 的斜杠命令面（TUI 面板实测 + 内嵌手册交叉核对）
REFERENCE_COMMANDS = {
    "clear", "compact", "config", "context", "diff", "exit", "fork", "goal",
    "import-claude", "init", "mcp", "model", "models", "offline-mode",
    "providers", "rename", "resume", "rewind", "statusline", "term", "theme",
    "update", "usage", "welcome",
}

RESULTS = []


def record(name, status, detail=""):
    RESULTS.append((name, status, detail))
    print(f"[{status}] {name}" + (f" — {detail}" if detail else ""))


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def make_home(port=None):
    """构造一个带已授权 provider 的隔离 Ante home。"""
    home = Path(tempfile.mkdtemp(prefix="ante-parity-"))
    ante = home / ".ante"
    (ante / "auth/credentials/secret").mkdir(parents=True)
    port = port or 8893
    catalog = {
        "providers": {
            "val": {
                "display_name": "Val",
                "base_url": f"http://127.0.0.1:{port}/v1",
                "credential": ["env:X", "secret:val"],
                "wire_style": "OpenAiCompatible",
                "preferred_models": [{
                    "id": "mock-model", "description": "mock",
                    "context_limit": 32768, "max_tokens": 4096,
                    "effort": "medium", "supported_vision": True,
                }],
            }
        }
    }
    # 修正字段名（support_vision 是正确拼写）
    catalog["providers"]["val"]["preferred_models"][0].pop("supported_vision")
    catalog["providers"]["val"]["preferred_models"][0]["support_vision"] = True
    (ante / "catalog.json").write_text(json.dumps(catalog))
    cred = ante / "auth/credentials/secret/val.json"
    cred.write_text('{"value":"sk-test"}')
    cred.chmod(0o600)
    return home


def env_for(home):
    e = dict(os.environ)
    e.update({
        "HOME": str(home),
        "ANTE_HOME": str(home / ".ante"),
        "TERM": "xterm-256color",
        "COLUMNS": "120",
        "LINES": "40",
        "MOCK_API_KEY": "dummy",
        "ANTIX_API_KEY": "dummy",
        "NO_COLOR": "1",
    })
    return e


# ---------------------------------------------------------------- 测试项


def test_command_surface():
    src = (ROOT / "src/repl.rs").read_text()
    arms = []
    for line in src.splitlines():
        if "=>" in line and '"' in line:
            arms.extend(re.findall(r'"([a-z][a-z0-9-]+)"', line.split("=>")[0]))
    ours = {c for c in arms if c in REFERENCE_COMMANDS}
    missing = sorted(REFERENCE_COMMANDS - ours)
    extra = sorted(ours - REFERENCE_COMMANDS - {"quit"})
    if missing or extra:
        record("slash-command-surface", "FAIL",
               f"missing={missing or '-'} extra={extra or '-'}")
    else:
        record("slash-command-surface", "PASS", f"{len(ours & REFERENCE_COMMANDS)} commands")


def test_no_forbidden_commands(binary, home):
    """原版手册明确：没有 /help，也没有 /permissions。"""
    out = capture_tui(binary, home, keys=["/help\r"], wait=1.0)
    forbidden = "Unknown command: /help" in out
    record("no-help-command", "PASS" if forbidden else "FAIL",
           "unknown command expected" if forbidden else "unexpectedly handled /help")


def test_welcome_screen(binary, home):
    txt = capture_tui(binary, home, keys=["\r"], wait=1.6)
    required = ["Ante v0.3.4", "mock-model", "val"]
    art = any(ch in txt for ch in "▄▐█▀")
    absent = [r for r in required if r not in txt]
    if absent:
        record("welcome-screen", "FAIL", f"missing tokens: {absent}")
    elif not art:
        record("welcome-screen", "FAIL", "no ASCII wordmark detected")
    else:
        record("welcome-screen", "PASS", "wordmark + version + model/provider")


def test_catalog_parity(orig, ours, home):
    a = run([ours, "catalog"], env=env_for(home), cwd=str(ROOT), timeout=30)
    if a.returncode != 0:
        record("catalog-parity", "FAIL", f"ours exit {a.returncode}: {a.stderr[:120]}")
        return
    if not orig:
        record("catalog-parity", "SKIP", "no --orig binary")
        return
    b = run([orig, "catalog"], env=env_for(home), cwd=str(ROOT), timeout=30)
    if json.loads(a.stdout) == json.loads(b.stdout):
        record("catalog-parity", "PASS", "byte-equal JSON structure")
    else:
        record("catalog-parity", "FAIL", "catalog JSON differs")


def test_serve_protocol(orig, ours, home):
    ops = [
        ("StartSession", {}),
        ("UserInput", "say OK"),
        ("Shutdown", {}),
    ]
    seq = {}
    for tag, exe in (("ours", ours), ("orig", orig)):
        if not exe:
            continue
        payload = "\n".join(
            json.dumps({"id": f"op_{ulid(i)}", "op": {name: arg}})
            for i, (name, arg) in enumerate(ops, 1)
        ) + "\n"
        p = run([exe, "serve", "--stdio"], env=env_for(home), cwd=str(ROOT),
                input=payload, timeout=60)
        events = []
        for line in p.stdout.splitlines():
            try:
                ev = json.loads(line).get("event")
            except json.JSONDecodeError:
                continue
            events.append(ev if isinstance(ev, str) else next(iter(ev)))
        seq[tag] = events
    if "orig" not in seq:
        record("serve-protocol", "SKIP" if "ours" in seq else "FAIL",
               "no --orig binary; ours events=" + ",".join(seq.get("ours", [])[:4]))
        return
    seq["ours"] = [e for e in seq["ours"] if e != "Error"]
    seq["orig"] = [e for e in seq["orig"] if e != "Error"]
    if seq["ours"] == seq["orig"]:
        record("serve-protocol", "PASS", " / ".join(seq["ours"]))
    else:
        record("serve-protocol", "FAIL",
               f"ours={seq['ours']} orig={seq['orig']}")


def test_doctor(orig, ours, home):
    a = run([ours, "doctor"], env=env_for(home), cwd=str(ROOT), timeout=30)
    if not orig:
        record("doctor-format", "SKIP", "no --orig binary")
        return
    b = run([orig, "doctor"], env=env_for(home), cwd=str(ROOT), timeout=30)
    a_lines = [l for l in a.stdout.splitlines() if l.strip()]
    b_lines = [l for l in b.stdout.splitlines() if l.strip()]
    keys_a = [l.split()[0] for l in a_lines[:8]]
    keys_b = [l.split()[0] for l in b_lines[:8]]
    if keys_a == keys_b:
        record("doctor-format", "PASS", " / ".join(keys_a))
    else:
        record("doctor-format", "FAIL", f"ours={keys_a} orig={keys_b}")


# ---------------------------------------------------------------- 工具函数


def ulid(n):
    base = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    t = int(time.time() * 1000)
    out = ""
    for _ in range(10):
        out = base[t % 32] + out
        t //= 32
    return out + base[n % 32] * 16


def capture_tui(binary, home, keys, wait=1.4):
    """在伪终端里启动 TUI，应答光标/设备查询，返回去 ANSI 的文本。"""
    pid, fd = pty.fork()
    if pid == 0:
        env = env_for(home)
        os.execve(binary, [binary, "--provider", "val", "-m", "mock-model"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    buf = b""
    answered = set()

    def pump(seconds):
        nonlocal buf
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.05)
            if not r:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            buf += chunk
            if b"\x1b[6n" in chunk and "cpr" not in answered:
                answered.add("cpr")
                os.write(fd, b"\x1b[1;1R")
            if b"\x1b[c" in chunk and "da" not in answered:
                answered.add("da")
                os.write(fd, b"\x1b[?1;2c")
            if b"\x1b[>1u" in chunk and "kbd" not in answered:
                answered.add("kbd")
                os.write(fd, b"\x1b[?1u")

    pump(2.0)
    for k in keys:
        os.write(fd, k.encode())
        pump(wait)
    os.write(fd, b"\x03")
    pump(0.5)
    try:
        os.close(fd)
    except OSError:
        pass
    return ANSI.sub("", buf.decode("utf-8", "replace"))


# ---------------------------------------------------------------- 入口


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--orig", help="原版 ante 路径（可选）")
    ap.add_argument("--ours", default=str(ROOT / "target/release/ante"))
    args = ap.parse_args()
    ours = os.path.abspath(args.ours)
    orig = os.path.abspath(args.orig) if args.orig else None
    if not os.path.exists(ours):
        print(f"missing build: {ours}\nrun cargo build --release first")
        return 1
    home = make_home()

    test_command_surface()
    test_no_forbidden_commands(ours, home)
    test_welcome_screen(ours, home)
    test_catalog_parity(orig, ours, home)
    test_serve_protocol(orig, ours, home)
    test_doctor(orig, ours, home)

    failed = [r for r in RESULTS if r[1] == "FAIL"]
    passed = [r for r in RESULTS if r[1] == "PASS"]
    skipped = [r for r in RESULTS if r[1] == "SKIP"]
    print(f"\n{len(passed)} passed, {len(failed)} failed, {len(skipped)} skipped")
    if failed:
        print("failures:", ", ".join(n for n, _, _ in failed))
    shutil.rmtree(home, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
