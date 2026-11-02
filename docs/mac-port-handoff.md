> 说明：这是 macOS 移植当时的历史记录，文中路径（`mac-port/setup-macos.py`、`tools/mcp/setup.py`、`dsh-bundle/`）后来已调整为
> `scripts/setup.py`、`dsh-plugin/`。当前用法以 [mcp.md](mcp.md) 为准。

# MIRSA → macOS 交接说明

这份包的目的：把一套**已经在 Linux/WSL 上跑通并验证过**的 MIRSA MCP 接入层交到 macOS 上，
让接手的 AI 只需要处理 4 个平台差异点，而不用从头摸索架构和语义。

---

## 0. 阅读顺序

1. 本文件第 2 节（架构）——搞清三个进程的关系
2. 第 3 节（**macOS 阻塞点**）——这是唯一需要你改代码的地方，共 4 处
3. 第 4 节（安装步骤）
4. 第 5 节（验证）——有**已知正确答案**，改完必须复现

> **重要前提**：第 2/3/5 节里标「已验证」的结论来自 Linux(WSL) 上的真实运行；
> macOS 相关的判断来自**源码审阅**，没有在 macOS 上实跑过。
> 请把第 3 节的每一条都当成「高度可疑、需实跑确认」而不是既成事实。

---

## 1. 包里有什么

```
src/                          # 从跑通的部署中原样取出（权威版本）
  tools/mcp/server.py         # MCP 服务，导出 3 个工具（核心，~610 行）
  tools/mcp/setup.py          # 原版安装脚本 —— Linux 专用，见第 3 节
  tools/mcp/test_mcp.py       # 工具发现 + 真实查询的自测
  tools/mcp/requirements.txt  # mcp==2.3.0
  tools/mcp/dsh.cordis.yml    # DSH 接入配置（原版，用 wsl.exe 启动）
  tools/mcp/README.md
  tools/mcp/DSH接入.md         # 原部署文档（WSL 视角，术语需替换）
  scripts/start-mcp.sh        # 启动器（已可移植，无需改）
  examples/inputs/*.rs        # 演示输入
  rust-toolchain.toml         # 工具链锁定
  Cargo.toml, Cargo.lock      # 工作区定义与锁定依赖

setup-macos.py                # ← 跨平台安装脚本（darwin 已处理），见第 4 节
patch-server-dyld.py          # ← server.py 的 DYLD 补丁（幂等）
dsh.cordis.yml                # ← macOS 版 MCP 配置
vuln_demo_macos.py            # ← 端到端复现脚本（对着已知答案跑）
HANDOFF-macos.md              # 本文件
```

**包里没有**：
- Rust 源码的 `crates/`、`src/`、`tests/`（体积大）。请从上游取：
  `git clone https://github.com/990466307/mirsa.git && git checkout 4b266c7`
  已验证的版本 = `4b266c7c6b9506ba1cfd879e2df0f62f7db4a197`（tag 内容「release: prepare MIRSA 0.4.0」）。
- 原 Linux worker 二进制（`mirsa-query`，281MB，x86_64-linux）。**Mac 必须自己重编**。

---

## 2. 架构（已验证）

```
MCP 客户端（DSH / Claude Code / …）
        │  stdio (JSON-RPC)
        ▼
scripts/start-mcp.sh          # 检查 3 个安装产物存在，然后 exec
        ▼
tools/mcp/server.py           # Python，导出 3 个工具
        │  每个请求 spawn 一次子进程，stdin 送 JSON，stdout 收 JSON
        ▼
target/mcp/mirsa-query        # Rust 二进制：rustc driver，真正的分析器
```

`start-mcp.sh` **不需要改**：它全靠 `BASH_SOURCE` 推导 `$mirsa_root`，无绝对路径、无平台假设。
它只要求这三个产物存在：

| 产物 | 作用 |
|---|---|
| `target/mcp/venv/bin/python` | 装了 `mcp==2.3.0` 的虚拟环境 |
| `target/mcp/mirsa-query` | 分析器 worker |
| `target/mcp/runtime.json` | `{"rustc_lib": "<toolchain>/lib"}`，worker 找 rustc 动态库用 |

worker 的接口是**一次请求一个 JSON**，所以没有常驻端口，也没有并发问题。

### 三个工具（对外契约，已验证）

| 工具 | 必填 | 说明 |
|---|---|---|
| `mirsa_read_lines` | `file`, `start` | 读带行号的源码（`count` 可选，默认 80，上限 400） |
| `mirsa_variables` | `line` | 列出该行**真实可解析**的变量名（`file` 或 `code` 二选一） |
| `mirsa_query` | `line`, `dsl` | 查性质；`file`/`code` 二选一，`bindings` 可省（同名默认） |

`mirsa_query` 检测到文件里已有 `// mirsa: mirsa_line_entry` 标注会复用；没有就在**内存里**插入标注，
**不改原文件**。返回值里 `location.annotated_statement` 是最终被分析的语句，可用来核对行号有没有插错位置。

---

## 3. macOS 阻塞点（必须改，共 4 处）

原部署在 Linux 上完成，代码里写死了 4 个 Linux 约定。逐条给出位置和原因。

### 3.1 `tools/mcp/setup.py:48` —— 只找 `libz3.so`

```python
library = next(prefix.glob("lib/**/libz3.so")).parent
```

macOS 的 Homebrew z3 产出的是 **`libz3.dylib`**（通常 `libz3.4.x.dylib` + 符号链接）。
`glob("lib/**/libz3.so")` 会抛 `StopIteration`。

→ 用 `setup-macos.py` 替代，它按平台选后缀。

### 3.2 `tools/mcp/setup.py:63,65` —— 用 `ldd` 且只认 `libz3.so`

```python
dependencies = subprocess.check_output(["ldd", str(binary)], env=env, text=True)
z3 = next((line.split("=>", 1)[1].strip().split()[0]
           for line in dependencies.splitlines() if "libz3.so" in line and "=>" in line), None)
if not z3 or not Path(z3).is_file():
    raise RuntimeError("无法定位 libz3，请用 --z3-prefix 指定原生依赖目录")
```

**macOS 上没有 `ldd`**（会 `FileNotFoundError`），等价物是 `otool -L`，且输出格式不同：

```
	/opt/homebrew/opt/z3/lib/libz3.dylib (compatibility version 4.15.0, ...)
```

注意：**没有 `=>`**，路径是行首第一个 token。照抄原解析逻辑会拿不到路径，
最终落到那句「无法定位 libz3」的报错上——这个报错很有迷惑性，会让人以为是 z3 没装。

### 3.3 `tools/mcp/server.py:68-73` —— worker 环境只设 `LD_LIBRARY_PATH`

```python
def worker_env() -> dict[str, str]:
    config = json.loads((RUNTIME / "runtime.json").read_text())
    libraries = [str(RUNTIME / "lib"), config["rustc_lib"]]
    if os.environ.get("LD_LIBRARY_PATH"):
        libraries.append(os.environ["LD_LIBRARY_PATH"])
    return dict(os.environ, LD_LIBRARY_PATH=os.pathsep.join(libraries))
```

macOS 的动态加载器**根本不看 `LD_LIBRARY_PATH`**，它看 `DYLD_LIBRARY_PATH`。
不改的话 worker 可能找不到 `libz3.dylib` 或 `librustc_driver-*.dylib`。

→ 跑 `python3 patch-server-dyld.py`（幂等，会顺手补上 `import sys`）。

### 3.4 `tools/mcp/dsh.cordis.yml` —— 启动命令是 `wsl.exe`

```yaml
command: wsl.exe
args: ['-d', 'Ubuntu-D', '-u', 'wentao', '--exec', '/bin/bash', '/home/wentao/mirsa/scripts/start-mcp.sh']
```

→ 用本包的 `dsh.cordis.yml`（已改成 `/bin/bash <绝对路径>/scripts/start-mcp.sh`）。

### 3.5 附带：`start-mcp.sh` 无 `LD_LIBRARY_PATH`

它只 `exec` python，不设任何库路径——这没问题，因为**库路径由 `server.py` 的 `worker_env()`
在 spawn worker 时注入**（3.3 就是改这里）。别误以为要改 `start-mcp.sh`。

### 3.6 可选（不影响功能）

`server.py` 的工具描述里写着「路径使用 **WSL** 绝对路径」「它读的是 **WSL** 路径」等字样。
这些字符串会直接进 AI 的 tool description，在 macOS 上可能诱导它去找 `/home/wentao/...` 这种不存在的路径。
建议全局替换成「本机绝对路径」。纯文案，不影响行为。

---

## 4. 安装步骤

前置：Xcode CLT（`xcode-select --install`）、rustup、Homebrew。

```bash
# 1) 取仓库并回到已验证的提交
git clone https://github.com/990466307/mirsa.git ~/mirsa
cd ~/mirsa && git checkout 4b266c7c6b9506ba1cfd879e2df0f62f7db4a197

# 2) 工具链：nightly-2025-02-14 + rustc-dev + rust-src（rustc driver 必需）
rustup toolchain install nightly-2025-02-14 --component rustc-dev,rust-src

# 3) 原生依赖：z3（求解器）+ llvm（bindgen 要 libclang）
brew install z3 llvm

# 4) 覆盖 MCP 层为本包的权威版本
cp -R src/tools/mcp/.    tools/mcp/
cp -R src/scripts/.      scripts/
cp -R src/examples/inputs/. examples/inputs/

# 5) 打 DYLD 补丁
python3 patch-server-dyld.py tools/mcp/server.py

# 6) 安装（编译 worker + 建 venv + 写 runtime.json）
python3 setup-macos.py --root ~/mirsa
```

`setup-macos.py` 会做这些事，并在每步失败时给出可读的原因：
自动定位 Homebrew 的 `z3.h` / `libz3.dylib` / `libclang.dylib` → 设
`Z3_SYS_Z3_HEADER`、`Z3_LIBRARY_PATH_OVERRIDE`、`LIBCLANG_PATH` →
`cargo build --locked --features cli,z3 --bin mirsa-query` →
用 `otool -L` 定位实际链接的 libz3 → 复制到 `target/mcp/lib` → 写 `runtime.json` → 建 venv 装依赖。

> **`--features cli,z3` 绝不能省 `z3`。**
> `crates/analysis` 里 `QueryConfig::use_solver = cfg!(feature = "z3")`，
> 而 `crates/solver` 在未启用 `z3` 时让 `solve()` 恒返回 `Unknown`。
> 症状是：编译成功、运行无报错、**所有性质都判成「未证明」**——静默失效，极难察觉。

---

## 5. 验证（有已知正确答案）

### 5.1 工具自测

```bash
~/mirsa/target/mcp/venv/bin/python ~/mirsa/tools/mcp/test_mcp.py
```

检查工具发现、真实查询、错误路径。**必须在第 4 节完成后跑**。

### 5.2 端到端复现（这是关键验收）

```bash
~/mirsa/target/mcp/venv/bin/python vuln_demo_macos.py
```

对 `demo/vulnerable_demo.rs`（21 行，3 处 `unsafe`）查询各自的越界前置条件，
**正确答案来自本次 Linux 上的真实运行**，Mac 上必须逐字一致：

| 行 | 函数 | 查询性质 | 期望结论 |
|---|---|---|---|
| 4 | `definitely_out_of_bounds` | `$index < len($table)` | **确定违反** `definite_violation=true`，`$index=[5,5]`，`len($table)=[3,3]` |
| 9 | `bounded_by_constant` | `3 < len($data)` | **已证明** `proved=true` |
| 17 | `correctly_guarded` | `$index < len($data)` | **已证明** `proved=true` |

如果第 4 行报「未证明」而不是「确定违反」，**首先怀疑 3.3（DYLD）或 `z3` feature 没启用**，
而不是怀疑分析器——那正是 z3 缺失时的典型表现。

---

## 6. 语义要点（这些坑已经踩过，别重踩）

### DSL 写法

- **末尾不加分号**
- 位置写法只有**开头一个**引号：`at 'mirsa_line_entry`（不是 `at 'mirsa_line_entry'`）
- 字面量不加类型后缀：`$i % 2` ✓，`$i % 2u64` ✗（类型不匹配）。转换写 `e as u64`
- 一行只能一条查询；`facts` 只接受**一个**参数，`facts($x)` ✓，`facts()` ✗
- 完整文法：仓库内 `crates/dsl/grammar.ebnf`

### 结论怎么读（三者互斥，不要混）

| 字段 | 含义 | 能不能当漏洞 |
|---|---|---|
| `proved=true` | 该性质成立 | 性质成立，非漏洞 |
| `definite_violation=true` | 存在必然违反的可达路径 | **是漏洞**，附 `evidence` 给出各变量具体取值 |
| `unreachable=true` | 查询点不可达 | 不构成证明 |
| 以上都不是 + `reason` 有值 | **未证明** | **不能当安全，也不能当漏洞** |

`reason` 的取值描述的是**分析为什么给不出结论**，与代码有无缺陷无关：
`BudgetExceeded` / `InsufficientInformation` / `SolverUnknown` / `Unsupported` /
`MixedPaths` / `BackendDisabled`（后者=z3 未启用，见第 4 节）。

`evidence` 只在 `definite_violation=true` 时出现，是「漏洞为什么成立」；
`reason` + `unproven_witness` 只在未证明时出现，是「分析为什么没结论」。两者回答不同问题。

### 分析模型的边界（必须如实汇报）

1. **只看标注所在的函数体**，不看调用方实参。加一个具体调用方不会改变函数内结论。
2. **无符号整数减法下溢没有建模**。涉及 `len - 1` 的守卫即使 `proved`，也只在算术前提成立时有效。
3. **证明一条性质 ≠ 整个函数安全**。例如 `get_unchecked` 只证了下标在界内，
   不代表指针有效性、对齐、生命周期都成立。
4. 修复成 `get()`/`[]` 这类安全 API 后再查，**没有需要证明的前置条件**，
   此时的「未证明」无意义，不能据此说修复无效。

---

## 7. 接回 DSH（可选）

改好 `dsh.cordis.yml` 里的绝对路径后：

```bash
dsh web --patch /path/to/dsh.cordis.yml
```

`toolCallTimeoutMs` 原配置是 `120000`（首次编译单个 `.rs` 较慢，别调太小）。
MCP 工具列表只在重连/重启时同步，改完配置要新建会话。

---

## 8. 排错

| 症状 | 先查 |
|---|---|
| 「MIRSA MCP 尚未安装，请先运行 tools/mcp/setup.py」 | 三个产物缺一，见第 2 节表格 |
| 「无法定位 libz3」 | 3.1/3.2：`otool -L` 与 `.dylib` |
| 所有查询都是「未证明」 | z3 feature 或 DYLD，见 5.2 提示 |
| `mirsa-query` 启动即崩、找不到 `librustc_driver-*.dylib` | `runtime.json` 的 `rustc_lib` + 3.3 DYLD |
| 编译期找不到 `z3.h` | `Z3_SYS_Z3_HEADER` |
| 编译期 bindgen 找不到 libclang | `LIBCLANG_PATH`（指向 `libclang.dylib` 所在目录） |
| Apple Silicon vs Intel | Homebrew 前缀不同：`/opt/homebrew` vs `/usr/local`，脚本已自动探测，但 `Cargo.lock` 里的 `z3-sys` 只锁版本、不锁架构，正常 |

`z3-sys` 版本 `0.9.10`，它读的环境变量就是这三个（已核对 crate 源码）：
`Z3_SYS_Z3_HEADER`、`Z3_LIBRARY_PATH_OVERRIDE`、`LIBCLANG_PATH`。
