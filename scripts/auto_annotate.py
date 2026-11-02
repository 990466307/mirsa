#!/usr/bin/env python3
"""给未标注的样本自动生成 MIRSA 安全性质注解。

规则来自 std 里这些 unsafe API 的文档要求：

    x.get_unchecked(i)            -> i < len(x)
    x.split_at_unchecked(mid)     -> mid <= len(x)
    NonZeroT::new_unchecked(v)    -> v != 0
    NonNull::new_unchecked(p)     -> !is_null(p)
    CStr::from_ptr(p)             -> !is_null(p)
    copy_nonoverlapping(s, d, n)  -> n <= len(s) 且 n <= len(d)

用法::

    python3 scripts/auto_annotate.py /path/to/your-crate --analyze

标注写在目标目录的**副本**上（target/auto-annotate/...），原目录不动。
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKER = ROOT / "target/mcp/mirsa-query"

PATTERNS = [
    (re.compile(r"(\w+)\.get_unchecked(?:_mut)?\(\s*(\w+)\s*\)"), lambda m: f"{m.group(2)} < len({m.group(1)})"),
    (re.compile(r"(\w+)\.split_at_unchecked\(\s*(\w+)\s*\)"), lambda m: f"{m.group(2)} <= len({m.group(1)})"),
    (re.compile(r"NonZero\w*::new_unchecked\(\s*(\w+)\s*\)"), lambda m: f"{m.group(1)} != 0"),
    (re.compile(r"NonNull::new_unchecked\(\s*(\w+)\s*\)"), lambda m: f"!is_null({m.group(1)})"),
    (re.compile(r"CStr::from_ptr\(\s*(\w+)\s*\)"), lambda m: f"!is_null({m.group(1)})"),
]


def properties_for(line: str) -> list[str]:
    found: list[str] = []
    for pattern, make in PATTERNS:
        for match in pattern.finditer(line):
            found.append(make(match))
    size_of = {"u8":1,"i8":1,"u16":2,"i16":2,"u32":4,"i32":4,"u64":8,"i64":8,
               "u128":16,"i128":16,"usize":8,"isize":8,"f32":4,"f64":8}
    for match in re.finditer(r"try_from_bytes::<(\w+)>\(([^)]+)\)", line):
        ty, arg = match.group(1), match.group(2).strip().lstrip("&").strip()
        if ty in size_of and re.fullmatch(r"\w+", arg):
            found.append(f"len({arg}) == {size_of[ty]}")
    copy = re.search(r"copy_nonoverlapping\(\s*(\w+)\.as_ptr\(\),\s*(\w+)\.as_mut_ptr\(\),\s*(\w+)\s*\)", line)
    if copy:
        src, dst, count = copy.groups()
        found += [f"{count} <= len({src})", f"{count} <= len({dst})"]
    return found


def annotate(source: Path, target: Path) -> list[dict]:
    """在副本上按 API 要求 + 代码里已有的守卫生成注解。"""
    shutil.rmtree(target, ignore_errors=True)
    shutil.copytree(source, target)
    sites: list[dict] = []
    size_of = {"u8":1,"i8":1,"u16":2,"i16":2,"u32":4,"i32":4,"u64":8,"i64":8,
               "u128":16,"i128":16,"usize":8,"isize":8,"f32":4,"f64":8}

    for file in sorted(target.rglob("*.rs")):
        text = file.read_text()
        lines = text.splitlines()
        consts = {
            m.group(1): m.group(2)
            for m in re.finditer(r"const\s+(\w+)\s*:\s*usize\s*=\s*(\d+)", text)
        }
        slice_locals = {
            m.group(1): (m.group(2), m.group(3), m.group(4))
            for m in re.finditer(r"let\s+(\w+)\s*=\s*&(\w+)\[(\w+)\.\.(\w+)\];", text)
        }
        array_locals = {
            m.group(1): consts.get(m.group(2), m.group(2))
            for m in re.finditer(r"let\s+(\w+)\s*:\s*\[u8;\s*(\w+)\]", text)
        }

        inserts: dict[int, list[str]] = {}

        def add(line_index: int, prop: str) -> None:
            bucket = inserts.setdefault(line_index, [])
            if prop not in bucket:
                bucket.append(prop)

        # 1) try_from_bytes::<T>(arg)：优先用具体类型大小
        for index, line in enumerate(lines):
            for match in re.finditer(r"try_from_bytes::<(\w+)>\(([^)]+)\)", line):
                ty, arg = match.group(1), match.group(2).strip().lstrip("&").strip()
                if arg in slice_locals:
                    continue
                if ty in size_of and re.fullmatch(r"\w+", arg):
                    add(index, f"len({arg}) == {size_of[ty]}")
                elif arg in slice_locals:
                    base, _, end = slice_locals[arg]
                    add(index, f"{end} <= len({base})")
                elif arg in array_locals and array_locals[arg].isdigit():
                    add(index, f"len({arg}) == {array_locals[arg]}")

        # 2) 代码里已有的守卫：if EXPR > X.len() / if X.len() < N
        for index, line in enumerate(lines):
            guard = re.search(r"if\s+(.+?)\s*>\s*(\w+)\.len\(\)", line)
            prop = None
            if guard:
                prop = f"{guard.group(1).strip()} <= len({guard.group(2)})"
            else:
                guard = re.search(r"if\s+function_placeholder", line)
                guard = re.search(r"if\s+(\w+)\.len\(\)\s*<\s*(\w+)", line)
                if guard:
                    bound = consts.get(guard.group(2), guard.group(2))
                    prop = f"len({guard.group(1)}) >= {bound}"
            if prop:
                target_line = None
                for offset in range(index + 1, min(index + 10, len(lines))):
                    candidate = lines[offset]
                    if "unsafe" in candidate or "try_from_bytes" in candidate:
                        target_line = offset
                        break
                if target_line is None:
                    for offset in range(index + 1, min(index + 10, len(lines))):
                        if lines[offset].strip().startswith("let "):
                            target_line = offset
                            break
                if target_line is not None:
                    add(target_line, prop)

        out: list[str] = []
        for index, line in enumerate(lines):
            for prop in inserts.get(index, []):
                indent = line[: len(line) - len(line.lstrip())]
                out.append(f"{indent}#[mirsa::check({prop})]\n")
                sites.append({"file": str(file.relative_to(target)), "property": prop})
            out.append(line + "\n")
        file.write_text("".join(out))
    return sites


def analyze(target: Path) -> None:
    env = dict(os.environ)
    lib = glob.glob(os.path.expanduser("~/.rustup/toolchains/nightly-2025-02-14-*/lib"))[0]
    env["DYLD_LIBRARY_PATH"] = os.pathsep.join([lib, str(ROOT / "third-party/z3/lib")])
    root = target / "src/lib.rs"
    files = [root] if root.exists() else sorted((target / "src").glob("*.rs"))
    for file in files:
        reply = subprocess.run(
            [str(WORKER)],
            input=json.dumps({"file": str(file)}),
            capture_output=True,
            text=True,
            env=env,
        )
        payload = json.loads(reply.stdout or "{}")
        if payload.get("status") != "ok":
            print(f"{file.name}: 分析失败 {payload.get('kind')} {str(payload.get('message'))[:120]}")
            continue
        for item in payload.get("reports", []):
            output = (item.get("report") or {}).get("output") or {}
            result = output.get("result") or {}
            name = Path(str(item.get("file", file))).name
            print(f"{name}:{item.get('line'):<4} {item.get('query'):<26} {result.get('status')}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("--analyze", action="store_true")
    options = parser.parse_args()
    source = options.source.resolve()
    target = ROOT / "target/auto-annotate" / source.name
    sites = annotate(source, target)
    print(f"自动生成 {len(sites)} 条注解 → {target}")
    for site in sites:
        print(f"  {site['file']}: {site['property']}")
    if options.analyze:
        print()
        analyze(target)


if __name__ == "__main__":
    main()
