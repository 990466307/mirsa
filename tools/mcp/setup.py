"""Build the query worker and install the MCP adapter into target/mcp."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
import venv

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "target/mcp"


def install_python() -> None:
    venv.EnvBuilder(with_pip=False, symlinks=True).create(RUNTIME / "venv")
    # Use a wheel directly, including on distributions without ensurepip.
    with urllib.request.urlopen("https://pypi.org/pypi/pip/25.3/json", timeout=30) as response:
        release = json.load(response)
    wheel = next(item for item in release["urls"] if item["filename"].endswith(".whl"))
    package = RUNTIME / wheel["filename"]
    if not package.exists():
        with urllib.request.urlopen(wheel["url"], timeout=60) as response:
            package.write_bytes(response.read())
    if hashlib.sha256(package.read_bytes()).hexdigest() != wheel["digests"]["sha256"]:
        raise RuntimeError("pip wheel checksum mismatch")
    subprocess.run(
        [str(RUNTIME / "venv/bin/python"), "-m", "pip", "install",
         "--index-url", "https://pypi.org/simple", "-r", str(ROOT / "tools/mcp/requirements.txt")],
        env=dict(os.environ, PYTHONPATH=str(package)), check=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--z3-prefix", type=Path, help="可选原生依赖前缀，包含 include/z3.h 和 lib 下的 Z3/Clang")
    args = parser.parse_args()
    RUNTIME.mkdir(parents=True, exist_ok=True)
    rustup = shutil.which("rustup") or str(Path.home() / ".cargo/bin/rustup")
    rustc = Path(subprocess.check_output([rustup, "which", "rustc"], cwd=ROOT, text=True).strip())
    toolchain = rustc.parent.parent
    libraries = [str(toolchain / "lib")]
    env = dict(os.environ, RUSTC=str(rustc))
    if args.z3_prefix:
        prefix = args.z3_prefix.resolve()
        library = next(prefix.glob("lib/**/libz3.so")).parent
        libraries.append(str(library))
        env.update(Z3_SYS_Z3_HEADER=str(prefix / "include/z3.h"),
                   Z3_LIBRARY_PATH_OVERRIDE=str(library), LIBCLANG_PATH=str(library))
        includes = sorted(prefix.glob("lib/llvm-*/lib/clang/*/include"))
        if includes:
            env["BINDGEN_EXTRA_CLANG_ARGS"] = f"-I{includes[-1]}"
    if env.get("LD_LIBRARY_PATH"):
        libraries.append(env["LD_LIBRARY_PATH"])
    env["LD_LIBRARY_PATH"] = os.pathsep.join(libraries)
    subprocess.run(
        [str(toolchain / "bin/cargo"), "build", "--locked", "--features", "cli,z3", "--bin", "mirsa-query"],
        cwd=ROOT, env=env, check=True,
    )
    binary = ROOT / "target/debug/mirsa-query"
    dependencies = subprocess.check_output(["ldd", str(binary)], env=env, text=True)
    z3 = next((line.split("=>", 1)[1].strip().split()[0]
               for line in dependencies.splitlines() if "libz3.so" in line and "=>" in line), None)
    if not z3 or not Path(z3).is_file():
        raise RuntimeError("无法定位 libz3，请用 --z3-prefix 指定原生依赖目录")
    (RUNTIME / "lib").mkdir(exist_ok=True)
    shutil.copy2(z3, RUNTIME / "lib" / Path(z3).name)
    # Existing requests may still be executing the deployed worker.
    staging = RUNTIME / "mirsa-query.new"
    shutil.copy2(binary, staging)
    staging.replace(RUNTIME / "mirsa-query")
    (RUNTIME / "runtime.json").write_text(json.dumps({"rustc_lib": str(toolchain / "lib")}))
    install_python()
    print(f"MIRSA MCP 已安装：{ROOT / 'scripts/start-mcp.sh'}")


if __name__ == "__main__":
    main()
