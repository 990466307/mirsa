#!/usr/bin/env python3
"""安装 MIRSA 的 MCP 适配层（macOS / Linux 通用）。

和 ``tools/mcp/setup.py`` 做同样的事，但不再假设 Linux：

* 依赖探查在 macOS 上用 ``otool -L``，在 Linux 上用 ``ldd``
* 共享库在 macOS 上是 ``libz3*.dylib``，在 Linux 上是 ``libz3.so``
* Homebrew 的 z3 / llvm 位置自动探测；``--z3-prefix`` 仍然可用
* Mach-O 的库查找变量是 ``DYLD_LIBRARY_PATH``（server.py 侧见 patch-server-dyld.py）

用法::

    python3 setup-macos.py                      # 在本脚本所在目录向上找仓库根
    python3 setup-macos.py --root ~/mirsa       # 显式指定仓库根
    python3 setup-macos.py --z3-prefix /opt/homebrew/opt/z3
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import venv
from pathlib import Path

IS_MAC = sys.platform == "darwin"
LIB_VAR = "DYLD_LIBRARY_PATH" if IS_MAC else "LD_LIBRARY_PATH"


def die(message: str) -> None:
    raise SystemExit(f"\n[setup-macos] 失败：{message}\n")


def repo_root(explicit: Path | None) -> Path:
    if explicit:
        root = explicit.expanduser().resolve()
        if not (root / "Cargo.toml").is_file() or not (root / "crates").is_dir():
            die(f"{root} 不像 MIRSA 仓库根（需要 Cargo.toml 与 crates/）")
        return root
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "Cargo.toml").is_file() and (parent / "crates").is_dir():
            return parent
    die("找不到仓库根目录，请用 --root 指定 MIRSA 仓库路径")


def brew_prefix() -> Path | None:
    brew = shutil.which("brew")
    if not brew:
        return None
    try:
        return Path(subprocess.check_output([brew, "--prefix"], text=True).strip())
    except subprocess.CalledProcessError:
        return None


def find_z3(explicit: Path | None) -> tuple[Path, Path]:
    """返回 (z3.h 路径, 含 libz3 的库目录)。"""
    bases: list[Path] = []
    if explicit:
        bases.append(explicit.expanduser())
    prefix = brew_prefix()
    if prefix:
        bases += [prefix / "opt/z3", prefix]
    bases += [Path("/opt/homebrew/opt/z3"), Path("/usr/local/opt/z3"),
              Path("/opt/homebrew"), Path("/usr/local"), Path("/usr")]
    for base in bases:
        base = base.expanduser().resolve()
        header = base / "include/z3.h"
        if not header.is_file():
            continue
        # 官方 macOS 预编译包把 libz3.dylib 放在 bin/ 而不是 lib/，
        # 所以两处都要找（Z3 4.12/4.13 的 arm64-osx 包就是 bin/）。
        for libdir in (base / "lib", base / "bin", base):
            if libdir.is_dir() and any(libdir.glob("libz3.*")):
                return header.resolve(), libdir.resolve()
    die("找不到 z3。请先 `brew install z3`，或下载 Z3 官方 macOS 包后用 "
        "--z3-prefix 指定（其下需有 include/z3.h 与含 libz3 的 lib/ 或 bin/ 目录）")


def find_llvm() -> Path | None:
    """返回含 libclang 的目录（bindgen 需要）。"""
    candidates: list[Path] = []
    prefix = brew_prefix()
    if prefix:
        candidates += [prefix / "opt/llvm/lib", prefix / "lib"]
    candidates += [Path("/opt/homebrew/opt/llvm/lib"), Path("/usr/local/opt/llvm/lib")]
    # 没装 Homebrew llvm 时，Xcode CLT 自带的 libclang 就够 bindgen 用。
    candidates += [
        Path("/Library/Developer/CommandLineTools/usr/lib"),
        Path("/Applications/Xcode.app/Contents/Developer/Toolchains/"
             "XcodeDefault.xctoolchain/usr/lib"),
    ]
    for cand in candidates:
        if cand.is_dir() and any(cand.glob("libclang*")):
            return cand
    return None


def clang_include_dirs(liqclang: Path | None) -> list[Path]:
    """收集 libclang 相关头文件目录，喂给 bindgen。"""
    if liqclang is None:
        return []
    found: list[Path] = []
    for cand in (liqclang.parent / "include", liqclang / "clang" / "include"):
        if cand.is_dir():
            found.append(cand)
    found += sorted(liqclang.glob("clang/*/include"))
    return found


def toolchain_paths(root: Path) -> tuple[Path, Path]:
    rustup = shutil.which("rustup") or str(Path.home() / ".cargo/bin/rustup")
    try:
        rustc = Path(subprocess.check_output(
            [rustup, "which", "rustc"], cwd=root, text=True).strip())
    except (OSError, subprocess.CalledProcessError) as error:
        die(f"取不到 rustc（rustup: {rustup}）：{error}\n"
            "请确认已安装 rustup，并已 `rustup toolchain install "
            "nightly-2025-02-14 --component rustc-dev,rust-src`")
    if not rustc.is_file():
        die(f"rustup which rustc 返回了不存在的路径：{rustc}")
    return rustc, rustc.parent.parent


def build(root: Path, rustc: Path, toolchain: Path, header: Path, libdir: Path,
          llvm: Path | None, sdk: dict[str, str]) -> Path:
    cargo = toolchain / "bin/cargo"
    if not cargo.is_file() and not cargo.with_suffix(".exe").is_file():
        die(f"工具链里没有 cargo：{cargo}")

    env = dict(os.environ, RUSTC=str(rustc), **sdk)
    # z3-sys 0.9.10 认这三个（已核对 crate 的 build.rs）
    env["Z3_SYS_Z3_HEADER"] = str(header)
    env["Z3_LIBRARY_PATH_OVERRIDE"] = str(libdir)
    if llvm:
        env["LIBCLANG_PATH"] = str(llvm)
    includes = clang_include_dirs(llvm)
    if includes:
        flags = " ".join(f"-I{path}" for path in includes)
        env["BINDGEN_EXTRA_CLANG_ARGS"] = (
            f"{env['BINDGEN_EXTRA_CLANG_ARGS']} {flags}".strip()
            if env.get("BINDGEN_EXTRA_CLANG_ARGS") else flags
        )

    libraries = [str(toolchain / "lib"), str(libdir)]
    if env.get(LIB_VAR):
        libraries.append(env[LIB_VAR])
    env[LIB_VAR] = os.pathsep.join(libraries)

    print(f"[setup-macos] 平台      : {sys.platform} ({platform.machine()})")
    print(f"[setup-macos] 仓库      : {root}")
    print(f"[setup-macos] 工具链    : {toolchain}")
    print(f"[setup-macos] z3.h      : {header}")
    print(f"[setup-macos] libz3 目录: {libdir}")
    if sdk:
        print(f"[setup-macos] SDKROOT   : {sdk['SDKROOT']}")
    print(f"[setup-macos] libclang  : {llvm or '<未探测到，bindgen 可能失败>'}")
    print("[setup-macos] 开始编译 worker（--features cli,z3，首次较慢）…")

    try:
        subprocess.run(
            [str(cargo), "build", "--locked", "--features", "cli,z3",
             "--bin", "mirsa-query"],
            cwd=root, env=env, check=True,
        )
    except subprocess.CalledProcessError:
        die("cargo build 失败。常见原因：\n"
            "  * 工具链没装 rustc-dev/rust-src\n"
            "  * 找不到 z3.h 或 libz3（看上面打印的路径）\n"
            "  * 找不到 libclang（`brew install llvm`）")

    binary = root / "target/debug/mirsa-query"
    if not binary.is_file():
        die(f"编译成功但找不到产物：{binary}")
    return binary


def linked_z3(binary: Path, libdir: Path) -> Path:
    """从二进制的动态依赖里找出实际链接的 libz3。

    macOS 上 z3-sys 用 ``-lz3`` 链接，二进制里记的是裸名 ``libz3.dylib``
    或 ``@rpath/libz3.dylib``（不是绝对路径），所以要回退到构建时用的
    ``libdir`` 里去找同名文件。
    """
    command = ["otool", "-L", str(binary)] if IS_MAC else ["ldd", str(binary)]
    try:
        output = subprocess.check_output(command, text=True, stderr=subprocess.STDOUT)
    except (OSError, subprocess.CalledProcessError) as error:
        die(f"探查动态依赖失败（{command[0]}）：{error}")
    for line in output.splitlines():
        text = line.strip()
        if "libz3" not in text:
            continue
        if IS_MAC:
            # otool -L 每行以路径开头，没有 "=>"
            candidate = text.split(" ", 1)[0]
        else:
            if "=>" not in text:
                continue
            candidate = text.split("=>", 1)[1].strip().split(" ")[0]
        path = Path(candidate)
        if path.is_file():
            return path
        name = path.name
        if name.startswith("libz3") and ("/" not in candidate or candidate.startswith("@")):
            for where in (libdir, libdir / "bin", Path("/opt/homebrew/lib"),
                          Path("/usr/local/lib"), Path("/usr/lib")):
                found = where / name
                if found.is_file():
                    return found
    die("二进制没有链接 libz3 —— 很可能 z3 feature 没生效（保证 --features cli,z3）。\n"
        f"依赖列表：\n{output}")


def find_python() -> Path:
    """找一个 >= 3.10 的解释器（mcp 2.3.0 的最低要求）。"""
    candidates: list[str] = []
    if os.environ.get("MIRSA_PYTHON"):
        candidates.append(os.environ["MIRSA_PYTHON"])
    if sys.version_info >= (3, 10):
        candidates.append(sys.executable)
    for name in ("python3.13", "python3.12", "python3.11", "python3.10", "python3"):
        found = shutil.which(name)
        if found:
            candidates.append(found)
    for candidate in candidates:
        try:
            version = subprocess.check_output(
                [candidate, "-c", "import sys; print('%d.%d' % sys.version_info[:2])"],
                text=True, stderr=subprocess.DEVNULL).strip()
        except (OSError, subprocess.CalledProcessError):
            continue
        major, minor = (int(part) for part in version.split("."))
        if (major, minor) >= (3, 10):
            return Path(candidate)
    die("找不到 Python >= 3.10（系统自带的 3.9 跑不了 mcp 2.3.0）。\n"
        "任选其一：`brew install python@3.12`；或用官方 standalone 版本，"
        "然后 `MIRSA_PYTHON=/path/to/python3.12 python3 setup-macos.py …`")


def sdk_env() -> dict[str, str]:
    """确认当前 SDK 能链接出最小程序；不行就换一个能用的，返回要注入的环境变量。

    某些 macOS + CommandLineTools 组合里，默认 SDK 的 .tbd 用了当前 ``ld``
    还不认识的架构（例如 ``arm64e.x1``），连 ``int main(){}`` 都链接失败。
    这不是 MIRSA 的问题，但会以「cargo build 失败」的形式冒出来。
    注意 SDKROOT 可能已经被 shell/IDE 预设成那个坏 SDK，所以要先验证再用。
    """
    if not IS_MAC:
        return {}

    current = os.environ.get("SDKROOT")
    probes: list[Path | None] = [Path(current) if current else None]
    sdk_root = Path("/Library/Developer/CommandLineTools/SDKs")
    if sdk_root.is_dir():
        probes += sorted((p for p in sdk_root.glob("MacOSX*.sdk") if p.is_dir()),
                         key=lambda p: p.name, reverse=True)
    xcrun = shutil.which("xcrun")
    if xcrun:
        try:
            probes.append(Path(subprocess.check_output(
                [xcrun, "--show-sdk-path"], text=True,
                stderr=subprocess.DEVNULL).strip()))
        except (OSError, subprocess.CalledProcessError):
            pass
    seen: set[str] = set()
    probes = [p for p in probes
              if p is None or (str(p) not in seen and not seen.add(str(p)))]

    source = "int main(void){return 0;}\n"
    for sdk in probes:
        env = dict(os.environ)
        if sdk is None:
            env.pop("SDKROOT", None)
        else:
            env["SDKROOT"] = str(sdk)
        with tempfile.TemporaryDirectory() as directory:
            c_file = Path(directory) / "probe.c"
            out = Path(directory) / "probe"
            c_file.write_text(source)
            try:
                subprocess.run(["cc", str(c_file), "-o", str(out)], env=env,
                               capture_output=True, check=True)
            except (OSError, subprocess.CalledProcessError):
                continue
        if sdk is not None and str(sdk) != current:
            print(f"[setup-macos] 当前 SDK 链接不通，改用 SDKROOT={sdk}")
            return {"SDKROOT": str(sdk)}
        return {}
    print("[setup-macos] 警告：所有候选 SDK 都链接不出最小 C 程序，"
          "请先修好 CommandLineTools（`xcode-select --install` 并重开终端）")
    return {}


def install_python(runtime: Path, root: Path, python: Path) -> None:
    venv = runtime / "venv"
    # 用解释器自带的保证 pip（python-build-standalone 带 wheel），不再从网络
    # 拉 pip 本体：那一步走 urllib，在没有 IPv6 路由的机器上会挂住。
    subprocess.run([str(python), "-m", "venv", str(venv)], check=True)
    bundled = venv / "bin" / "python"
    subprocess.run([str(bundled), "-m", "ensurepip", "--upgrade"], check=True)
    index = os.environ.get("PIP_INDEX_URL", "https://pypi.org/simple")
    subprocess.run(
        [str(bundled), "-m", "pip", "install",
         "--index-url", index, "--retries", "5", "--timeout", "60",
         "-r", str(root / "tools/mcp/requirements.txt")],
        check=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, help="MIRSA 仓库根目录")
    parser.add_argument("--z3-prefix", type=Path,
                        help="含 include/z3.h 与 lib/（或 bin/）下 libz3* 的前缀目录")
    parser.add_argument("--python", type=Path,
                        help="创建 venv 用的 Python >= 3.10（默认自动探测）")
    args = parser.parse_args()

    root = repo_root(args.root)
    runtime = root / "target/mcp"
    runtime.mkdir(parents=True, exist_ok=True)

    header, libdir = find_z3(args.z3_prefix)
    llvm = find_llvm()
    python = args.python.expanduser() if args.python else find_python()
    if not Path(python).is_file():
        die(f"--python 指向的解释器不存在：{python}")
    rustc, toolchain = toolchain_paths(root)
    sdk = sdk_env()

    binary = build(root, rustc, toolchain, header, libdir, llvm, sdk)
    z3 = linked_z3(binary, libdir)

    (runtime / "lib").mkdir(exist_ok=True)
    shutil.copy2(z3, runtime / "lib" / z3.name)

    # 正在服役的 worker 可能还被别的请求占用，先写临时名再原子替换
    staging = runtime / "mirsa-query.new"
    shutil.copy2(binary, staging)
    staging.replace(runtime / "mirsa-query")
    (runtime / "runtime.json").write_text(
        json.dumps({"rustc_lib": str(toolchain / "lib")}), encoding="utf-8")

    install_python(runtime, root, Path(python))
    print(f"[setup-macos] venv Python: {python}")

    print(f"[setup-macos] 已部署 libz3: {z3.name}")
    print(f"[setup-macos] runtime.json: {runtime / 'runtime.json'}")
    print(f"[setup-macos] 完成。启动器：{root / 'scripts/start-mcp.sh'}")
    print("[setup-macos] 下一步：按 docs/mcp.md 接入 MCP 服务")


if __name__ == "__main__":
    main()
