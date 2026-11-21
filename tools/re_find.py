#!/usr/bin/env python3
"""静态定位工具：字符串 -> 代码引用（RIP 相对 lea/mov），输出反汇编片段。

用法：
    python3 tools/re_find.py "Goal not yet met"          # 找字符串与引用点
    python3 tools/re_find.py --around 0x1234 --lines 80  # 反汇编指定地址
    python3 tools/re_find.py --funcs "goal"              # 列出附近函数边界（保守猜测）
"""
import argparse
import re
import sys
from pathlib import Path

from capstone import Cs, CS_ARCH_X86, CS_MODE_64
from elftools.elf.elffile import ELFFile

DEFAULT_BIN = Path(__file__).resolve().parents[2] / "work/ante/ante"


class Image:
    def __init__(self, path):
        self.path = path
        with open(path, "rb") as f:
            elf = ELFFile(f)
            self.sections = {}
            for sec in elf.iter_sections():
                if sec.header.sh_type == "SHT_NOBITS":
                    continue
                self.sections[sec.name] = {
                    "addr": sec.header.sh_addr,
                    "data": sec.data(),
                    "flags": sec.header.sh_flags,
                }
            self.text = self.sections[".text"]
            self.rodata = [s for n, s in self.sections.items() if n in (".rodata", ".data.rel.ro", ".data")]

    def read(self, addr, size):
        for s in [self.text] + self.rodata:
            start, data = s["addr"], s["data"]
            if start <= addr < start + len(data):
                off = addr - start
                return data[off:off + size]
        return b""

    def cstr(self, addr, limit=512):
        raw = self.read(addr, limit)
        return raw.split(b"\0", 1)[0].decode("utf-8", "replace")


def find_strings(img, needle, encoding="utf-8"):
    hits = []
    pat = needle.encode(encoding)
    for name, s in img.sections.items():
        data, base = s["data"], s["addr"]
        start = 0
        while True:
            i = data.find(pat, start)
            if i < 0:
                break
            hits.append((name, base + i, data[max(0, i - 120):i + len(pat) + 160]))
            start = i + 1
    return hits


def scan_xrefs(img, targets):
    """扫描 .text，用 op_str 里的 rip 相对表达式定位字符串引用（比 detail 模式快一个数量级）。"""
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    md.skipdata = True
    code, base = img.text["data"], img.text["addr"]
    ranges = [(saddr, saddr + 400, name) for name, saddr, _ in targets]
    out = []
    for ins in md.disasm(code, base):
        if "rip" not in ins.op_str:
            continue
        m = re.search(r"rip ([-+])\s*(0x[0-9a-f]+)", ins.op_str)
        if not m:
            continue
        disp = int(m.group(2), 16)
        addr = ins.address + ins.size + (disp if m.group(1) == "+" else -disp)
        for lo, hi, name in ranges:
            if lo <= addr < hi:
                out.append((ins.address, ins.mnemonic, ins.op_str, addr, name))
                break
    return out


def disasm(img, addr, lines=60):
    md = Cs(CS_ARCH_X86, CS_MODE_64)
    md.detail = False
    code = img.text["data"]
    start = addr - img.text["addr"]
    if start < 0 or start >= len(code):
        return []
    return list(md.disasm(code[start:start + lines * 16], addr))[:lines]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("needle", nargs="?")
    ap.add_argument("--bin", default=str(DEFAULT_BIN))
    ap.add_argument("--around", type=lambda x: int(x, 0))
    ap.add_argument("--lines", type=int, default=40)
    ap.add_argument("--max-xrefs", type=int, default=12)
    args = ap.parse_args()

    img = Image(args.bin)

    if args.around:
        for ins in disasm(img, args.around, args.lines):
            print(f"{ins.address:#x}: {ins.mnemonic:<8} {ins.op_str}")
        return

    if not args.needle:
        ap.error("needle or --around required")

    targets = find_strings(img, args.needle)
    if not targets:
        print(f"no string matching {args.needle!r}")
        sys.exit(1)
    for name, addr, ctx in targets[:5]:
        print(f"[str] {name} {addr:#x} …{ctx.decode('utf-8', 'replace')[:160]!r}")
    xrefs = scan_xrefs(img, targets)
    print(f"[xref] {len(xrefs)} hits")
    for addr, mn, ops, saddr, name in xrefs[: args.max_xrefs]:
        print(f"  {addr:#x}: {mn} {ops}   -> {saddr:#x} ({name})")
        for ins in disasm(img, addr - 12, 10):
            print(f"      {ins.address:#x}: {ins.mnemonic:<8} {ins.op_str}")


if __name__ == "__main__":
    main()
