#!/usr/bin/env python3
"""让 tools/mcp/server.py 在 macOS 上找得到 worker 的共享库。

Linux 的动态加载器看 ``LD_LIBRARY_PATH``，macOS 看 ``DYLD_LIBRARY_PATH``。
server.py 的 ``worker_env()`` 只设置前者，所以 macOS 上 worker 可能找不到
``libz3.dylib`` 和 ``librustc_driver-*.dylib``。

本脚本做两件事，且**可重复执行**（已是目标状态时直接跳过）：

1. 确认 ``import sys`` 存在（补丁要用 ``sys.platform``）
2. 把 ``worker_env()`` 的返回值改成按平台选择变量名

用法::

    python3 patch-server-dyld.py                 # 默认改 ./tools/mcp/server.py
    python3 patch-server-dyld.py ~/mirsa/tools/mcp/server.py
"""

from __future__ import annotations

import ast
import sys
from pathlib import Path

OLD = '''    if os.environ.get("LD_LIBRARY_PATH"):
        libraries.append(os.environ["LD_LIBRARY_PATH"])
    return dict(os.environ, LD_LIBRARY_PATH=os.pathsep.join(libraries))'''

NEW = '''    # Linux 用 LD_LIBRARY_PATH，macOS 用 DYLD_LIBRARY_PATH：设置当前平台认的那个，
    # worker 才能找到 libz3 与 rustc driver 的动态库。
    var = "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
    if os.environ.get(var):
        libraries.append(os.environ[var])
    return dict(os.environ, **{var: os.pathsep.join(libraries)})'''

MARKER = 'if sys.platform == "darwin" else "LD_LIBRARY_PATH"'


def ensure_import_sys(source: str) -> tuple[str, bool]:
    if "\nimport sys\n" in source or source.startswith("import sys\n"):
        return source, False
    if "\nimport os\n" in source:
        return source.replace("\nimport os\n", "\nimport os\nimport sys\n", 1), True
    # 兜底：插在第一个 import 之前
    lines = source.splitlines(keepends=True)
    for index, line in enumerate(lines):
        if line.startswith(("import ", "from ")):
            lines.insert(index, "import sys\n")
            return "".join(lines), True
    return "import sys\n" + source, True


def main() -> None:
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("tools/mcp/server.py")
    if not target.is_file():
        raise SystemExit(f"找不到文件：{target}\n用法：python3 patch-server-dyld.py [server.py 路径]")

    source = target.read_text(encoding="utf-8")

    if MARKER in source:
        print(f"[patch] 已经是目标状态，跳过：{target}")
        return

    if "LD_LIBRARY_PATH=os.pathsep.join(libraries)" not in source:
        raise SystemExit(
            f"[patch] 在 {target} 里没找到预期的 worker_env() 写法。\n"
            "上游可能改过这段；请手工确认这三行：\n"
            "    if os.environ.get(\"LD_LIBRARY_PATH\"):\n"
            "        libraries.append(os.environ[\"LD_LIBRARY_PATH\"])\n"
            "    return dict(os.environ, LD_LIBRARY_PATH=os.pathsep.join(libraries))"
        )

    if OLD not in source:
        raise SystemExit(
            f"[patch] 找到了关键行但上下文不匹配（缩进/空白可能被改过）：{target}"
        )

    patched = source.replace(OLD, NEW, 1)
    patched, added = ensure_import_sys(patched)

    try:
        ast.parse(patched)
    except SyntaxError as error:
        raise SystemExit(f"[patch] 打补丁后语法错误，已放弃写入：{error}") from error

    backup = target.with_suffix(target.suffix + ".pre-dyld")
    if not backup.exists():
        backup.write_text(source, encoding="utf-8")
    target.write_text(patched, encoding="utf-8")

    print(f"[patch] 已打补丁：{target}")
    print(f"[patch] 补 import sys: {'是' if added else '本来就有'}")
    print(f"[patch] 原文件备份：{backup}")


if __name__ == "__main__":
    main()
