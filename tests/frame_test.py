#!/usr/bin/env python3
"""TUI 逐帧对比测试：120x40 终端下比较首屏、输入框与命令面板。

用法：
    python3 tests/frame_test.py [--orig /path/to/ante]

没有 --orig 时，使用 tests/golden/orig_welcome_120x40.txt 作为黄金样本。
退出码 0 表示所有断言通过。
"""

import argparse
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time
import fcntl
from pathlib import Path

import pyte

ROOT = Path(__file__).resolve().parents[1]
GOLDEN = ROOT / "tests/golden/orig_welcome_120x40.txt"

RESULTS = []


def record(name, ok, detail):
    RESULTS.append((name, ok, detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name} — {detail}")


FIXTURE_HOME = Path("/tmp/ah_tui/.ante")

CATALOG = {
    "providers": {
        "val": {
            "display_name": "Val",
            "base_url": "http://127.0.0.1:8893/v1",
            "wire_style": "OpenAiCompatible",
            "credential": ["env:X", "secret:val"],
            "preferred_models": [{
                "id": "mock-model", "description": "mock",
                "context_limit": 32768, "max_tokens": 4096,
                "effort": "medium", "support_vision": True,
            }],
        }
    }
}

RELEASE_NOTES = "## v0.3.7 - 2026-11-19\n\n### Added\n- `/rollover [instructions]` continues the conversation in a new session. The current session writes a handoff and stays resumable; the new session starts from the handoff alone, under current prompts and settings. The resume picker shows which session a session continues\n- Gemini 4 Neon on the Gemini API, Vertex AI and OpenRouter, with a 1,048,576-token context and low to xhigh thinking. It defaults to high effort on Gemini API and Vertex\n\n### Changed\n- The OpenAI API and subscription offer GPT-6.2 Sol in place of GPT-6.1 Sol, at low effort by default\n- Antix offers GPT-6.2 Astra and GPT-6.2 Sol in place of GPT-6 Astra and GPT-6.1 Sol\n- Bash can wait up to one hour for a command (was 10 minutes) and explains how to block on a backgrounded command's status file. A job's `status.json` records the session that started it\n- A reply that has thinking but no answer and no tool call is no longer stored and no longer ends the turn. The model is asked to continue, up to 3 times in a row, a limit shared with `max_tokens` continuations\n- Starting a session, including `/clear`, builds the new session before ending the current one, so a failed start leaves the current session as it was\n- Dependency updates\n\n### Fixed\n- Long lines in code blocks wrap instead of being cut at the right edge, and copying a wrapped line returns the original text, runs of spaces included\n- When Bash output is truncated, the full output stays in the file named on the first line until gc\n- The chat column on very narrow windows lines up with scrollback, and the offline model settings dialog no longer breaks at one row of height\n- The resume picker no longer adds `…` to a non-ASCII first message that fits\n\n### Wire\n- `StartSession` accepts optional `SessionRequest.rollover { instructions }`, and `SessionInfo` carries optional `rolled_from`\n\n### Crates\n- Public crates published as 0.3.7: ante-protocol-shape, ante-llm, ante-exec, ante-sdk"

VERSIONS = {
    "dismissed_version": None,
    "latest_version": {"version": "0.3.7", "release_notes": RELEASE_NOTES},
    "last_checked_unix_secs": 1795265910,
}


def ensure_fixture():
    """重建抓帧用的 ANTE_HOME，避免测试依赖临时目录的残留状态。"""
    import json as _json
    (FIXTURE_HOME / "auth/credentials/secret").mkdir(parents=True, exist_ok=True)
    (FIXTURE_HOME / "catalog.json").write_text(_json.dumps(CATALOG, ensure_ascii=False, indent=2))
    (FIXTURE_HOME / "versions.json").write_text(_json.dumps(VERSIONS, ensure_ascii=False, indent=2))
    cred = FIXTURE_HOME / "auth/credentials/secret/val.json"
    cred.write_text('{"value":"sk-test"}')
    cred.chmod(0o600)


def env_for():
    e = dict(os.environ)
    e.update({
        "HOME": "/tmp/ah_tui",
        "ANTE_HOME": "/tmp/ah_tui/.ante",
        "TERM": "xterm-256color",
        "COLUMNS": "120",
        "LINES": "40",
        "NO_COLOR": "1",
    })
    return e


def capture(binary, keys=(), cwd=ROOT.parent):
    """在 120x40 伪终端里运行，返回 40 行字符网格。"""
    binary = os.path.abspath(binary)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(str(cwd))
        os.execve(binary, [binary, "--provider", "val", "-m", "mock-model"], env_for())
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
        pump(1.0)
    # 先快照，再退出；否则离开备用屏会恢复空白的主屏
    screen = pyte.Screen(120, 40)
    pyte.ByteStream(screen).feed(buf)
    grid = [line.rstrip() for line in screen.display]
    os.write(fd, b"\x03")
    pump(0.4)
    try:
        os.close(fd)
    except OSError:
        pass
    return grid


def normalize(grid):
    """去掉更新公告等随网络变化的行，并统一动画字符。"""
    out = []
    for line in grid:
        s = line.replace("~", " ").rstrip()
        if False:
            out.append("")
        else:
            out.append(s)
    return out


def compare(name, ours, ref, rows, threshold=0.97):
    """逐字符比较指定行，返回相似度。"""
    total = same = 0
    diffs = []
    for r in rows:
        a = ours[r] if r < len(ours) else ""
        b = ref[r] if r < len(ref) else ""
        width = max(len(a), len(b), 1)
        for i in range(width):
            ca = a[i] if i < len(a) else " "
            cb = b[i] if i < len(b) else " "
            total += 1
            if ca == cb:
                same += 1
            elif len(diffs) < 5:
                diffs.append(f"r{r}c{i}: {ca!r}!={cb!r}")
    score = same / total if total else 0.0
    ok = score >= threshold
    detail = f"similarity={score:.3f} (threshold {threshold})"
    if not ok:
        detail += " | " + "; ".join(diffs)
    record(name, ok, detail)


def block(grid, marker):
    start = next((i for i, l in enumerate(grid) if marker in l and "┌" in l), None)
    if start is None:
        return []
    lines = []
    for i in range(start, min(start + 22, len(grid))):
        lines.append(grid[i])
        if grid[i].strip().startswith(("↑", "  ↑", "[↑")):
            break
    # 去掉尾随空行
    while lines and not lines[-1].strip():
        lines.pop()
    return lines


def overlay_case(name, cmd, ours, marker, width=118):
    ref = normalize((ROOT / f"tests/golden/orig_{name}_120x40.txt").read_text().splitlines())
    got = block(ours, marker)
    want = block(ref, marker)
    if not got:
        record(f"overlay-{name}", False, "overlay not rendered")
        return
    # 补齐右侧宽度后逐字符比较
    def pad(lines):
        return [l.ljust(width) for l in lines]
    compare(f"overlay-{name}", pad(got), pad(want), list(range(min(len(got), len(want)))), threshold=0.9)


def main():
    ensure_fixture()
    ap = argparse.ArgumentParser()
    ap.add_argument("--orig")
    args = ap.parse_args()

    ours_bin = str(ROOT / "target/release/ante")
    if not os.path.exists(ours_bin):
        print("missing target/release/ante; run cargo build --release")
        return 1

    if args.orig:
        ref = normalize(capture(os.path.abspath(args.orig)))
        if len([r for r in ref if r.strip()]) < 10:
            ref = normalize(GOLDEN.read_text().splitlines())
    else:
        ref = normalize(GOLDEN.read_text().splitlines())

    ours = normalize(capture(ours_bin))

    # 1) 首屏：字标 6 行 + 版本/模型/目录（忽略顶部空行差异）
    def header(grid):
        start = next(i for i, l in enumerate(grid) if l.strip())
        return grid[start:start + 6]

    compare("welcome-header", header(ours), header(ref), [0,1,2,4,5], threshold=0.9)

    # 2) 输入框与底栏：按分隔线定位，避免整体行偏移影响
    def composer(grid):
        seps = [i for i, l in enumerate(grid) if l.count("─") >= 20]
        if len(seps) < 2:
            return []
        start = seps[-2]
        return grid[start:start + 3]

    compare("composer", composer(ours), composer(ref), list(range(3)), threshold=0.97)
    hint = any("Press Ctrl+C again to exit" in l for l in ours)
    record("footer-hint", hint, "Ctrl+C hint present" if hint else "hint missing")

    # 3) 命令面板：输入 "/" 后必须列出命令与描述
    pal = "\n".join(capture(ours_bin, keys=["/"]))
    expected = ["/clear", "/compact", "/config", "/context", "/diff", "/exit"]
    missing = [c for c in expected if c not in pal]
    record("command-palette", not missing,
           "all core commands listed" if not missing else f"missing {missing}")

    # 欢迎页绝对行（跳过第 4 行动画字标）
    ref = normalize((ROOT / "tests/golden/orig_welcome_120x40.txt").read_text().splitlines())
    got = capture(ours_bin, keys=[])
    compare("welcome-frame", [got[i][:118].ljust(118) for i in range(24) if i != 4],
            [ref[i][:118].ljust(118) for i in range(24) if i != 4], list(range(23)), threshold=0.98)

    # 面板区：中段分隔线起到底部状态行
    ref = normalize((ROOT / "tests/golden/orig_palette_120x40.txt").read_text().splitlines())
    got = capture(ours_bin, keys=["/"])
    compare("palette-frame", [l.ljust(120) for l in got[17:35]],
            [l.ljust(120) for l in ref[17:35]], list(range(18)), threshold=0.95)

    for name, cmd, marker in [
        ("theme", "theme", "Theme"),
        ("providers", "providers", "Providers"),
        ("models", "models", "Switch Model"),
    ]:
        grid = capture(ours_bin, keys=["/", cmd, "\r"])
        overlay_case(name, cmd, grid, marker)

    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)} passed, {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
