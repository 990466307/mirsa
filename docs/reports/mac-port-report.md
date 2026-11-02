# MIRSA macOS 移植报告（真机实测）

> 历史记录：本文保留当时的路径和结果；旧演示输入已移除，当前测试结构与运行方式见 [tests/README.md](../../tests/README.md)。

日期：2026-10-23
机器：macOS 26.6 (Darwin 25.6.0)，Apple M5 / arm64
仓库：`990466307/mirsa` @ `4b266c7c6b9506ba1cfd879e2df0f62f7db4a197`（0.4.0）
结论：**移植成功。第 5.2 节的 3 条已知答案逐字复现。**

```
   行  性质                       实际         期望         结果
   4  $index < len($table)     确定违反       确定违反       ✓
   9  3 < len($data)           已证明        已证明        ✓
  17  $index < len($data)      已证明        已证明        ✓
```

第 4 行的 `evidence` 也与 Linux 一致：`$index = [5, 5]`、`Length($table) = [3, 3]`。

原始输出见 `verification/vuln_demo.log`。

---

## 1. 本次安装的位置

DSH 的沙箱只允许写工作区，所以没有装到 `~/mirsa`，而是：

```
/Users/pwt/Documents/deepseek-harness/default-workspace/mirsa/
├── repo/                       # MIRSA 仓库（4b266c7）+ 编译产物
│   ├── target/mcp/mirsa-query  # worker
│   ├── target/mcp/runtime.json
│   ├── target/mcp/lib/libz3.dylib
│   └── target/mcp/venv/        # Python 3.12 + mcp 2.3.0
├── deps/
│   ├── z3/                     # Z3 4.12.6 arm64（include/ + lib/libz3.dylib）
│   └── python/                 # CPython 3.12.15 standalone
└── mac-port/
    ├── dsh.cordis.yml          # 已填真实绝对路径
    ├── setup-macos.py          # 见第 3 节，已修
    └── verification/           # 验证日志
```

工具链：rustup `nightly-2025-02-14-aarch64-apple-darwin`（`rustc-dev`/`rust-src` 本来就已安装）。

---

## 2. 交接文档里的 4 个阻塞点，真机结果

| # | 交接文档的判断 | 真机结果 |
|---|---|---|
| 3.1 `libz3.so` vs `libz3.dylib` | 会 `StopIteration` | 属实。用 `setup-macos.py` 修掉 |
| 3.2 `ldd` / `=>` 解析 | macOS 没有 `ldd` | 属实；`otool -L` 分支本身可用，但**还有第二个坑**（见 3.2′） |
| 3.3 只设 `LD_LIBRARY_PATH` | worker 找不到 dylib | 属实。已打 `patch-server-dyld.py`，`worker_env()` 现按平台选变量 |
| 3.4 `wsl.exe` 启动 | macOS 没有 wsl.exe | 属实。配置改成本机 `bash -c exec <绝对路径>` |

3.3 是否真的生效，用第 5.2 节反证：如果 DYLD 或 z3 feature 没生效，第 4 行会变成「未证明」。
实测是「确定违反」，说明 `libz3.dylib` 与 `librustc_driver-*.dylib` 都被 worker 加载到了。

`start-mcp.sh` **未做任何修改**（3.5 的判断正确）。

3.6 的「WSL 路径」文案已全部替换为本机绝对路径的措辞（`server.py` 里 5 处）。

---

## 3. 真机上额外发现的问题（交接文档没提）

### 3.1′ Z3 官方 macOS 包把 dylib 放在 `bin/`，不在 `lib/`

`setup-macos.py` 原来只在 `lib/` 与包根找 `libz3.*`。官方 `z3-4.12.6-arm64-osx-11.0.zip`
的布局是 `bin/libz3.dylib` + `include/*.h`，所以按文档的 `--z3-prefix` 用法会报「找不到 z3」。
已修：`find_z3()` 同时看 `lib/`、`bin/`、包根，并把路径 `resolve()`（原来用相对路径当参数时，
`Z3_SYS_Z3_HEADER=deps/z3/include/z3.h` 会在 `cwd=repo` 下失效）。

### 3.2′ `otool -L` 里 libz3 是**裸名**，不是绝对路径

z3-sys 用 `-lz3` 链接，二进制记录的是 `libz3.dylib`（或 `@rpath/...`）：

```
$ otool -L target/debug/mirsa-query
	@rpath/librustc_driver-5eb1c8a19f485edd.dylib
	libz3.dylib
```

交接文档 3.2 只说了「行首第一个 token」，但如果照抄「`Path(candidate).is_file()`」就会失败，
报的却是「二进制没有链接 libz3 —— z3 feature 没生效」，极具误导性。
已修：`linked_z3(binary, libdir)` 在候选不是绝对文件时回退到构建用 `libdir` 里找同名文件。
（运行时仍然靠 `DYLD_LIBRARY_PATH` 指向 `target/mcp/lib`，这块不变。）

### 3.3′ 系统 Python 3.9 装不了 `mcp==2.3.0`

`mcp 2.3.0` 的 `Requires-Python` 是 `>=3.10`，而这台机器只有 `/usr/bin/python3` = 3.9.6，
又没装 Homebrew。原 `setup-macos.py` 用当前进程的解释器建 venv，会直接失败。
处理方式：下载 python-build-standalone 的 CPython 3.12.15（arm64）到 `deps/python`，
`setup-macos.py` 新增 `--python` 参数与 `find_python()`（>=3.10 才接受）。

### 3.4′ 本机 CLT 的 SDK 27.0 与新 `ld` 不匹配（环境问题，但会伪装成编译失败）

默认 SDK `MacOSX27.0.sdk` 的 `.tbd` 里有 `arm64e.x1` 架构，本机 `ld`（ld-1267）不认识，
**连 `int main(){}` 都链接失败**。而且 shell 环境里 `SDKROOT` 已被预设成这个坏 SDK，
所以 `cargo build` 第一层就挂，报错完全看不出和 MIRSA 有关。
（`MacOSX26.sdk` / `26.5` / `15.4` 都正常。）

`setup-macos.py` 新增 `sdk_env()`：先探测当前 `SDKROOT`，链接不通就按版本从新到旧找一个能用的，
以 `SDKROOT=...` 注入 cargo 的构建环境。本次自动选中 `MacOSX26.sdk`。

> 这是机器级问题，建议之后跑一次 `xcode-select --install` 或更新 CLT，让 `ld` 与 SDK 对齐；
> 修好后 `sdk_env()` 会自动回到默认 SDK，无需改配置。

### 3.5′ `test_mcp.py` 的 0.3 秒超时窗口在 macOS 上太短

`test_timeout_and_cancellation_stop_worker` 用一个假 worker 写 pid 再 sleep，超时窗口 0.3s。
macOS 上启动一个脚本进程本身就要 ~0.4s（沙箱/门禁），pid 还没写就被杀，测试必然报
`FileNotFoundError: .../pid`。只把**这个测试自己**的窗口在 darwin 上放宽到 2.0s，
产品的 `QUERY_SECONDS`（90s）没有改动。

---

## 4. 验证

### 4.1 工具自测

```
$ repo/target/mcp/venv/bin/python tools/mcp/test_mcp.py
Ran 5 tests in 6.0s
OK                  # WSL 转接测试已在第二轮删除，没有 skip
```

覆盖：工具发现（2 个工具/必填参数：`mirsa_variables`、`mirsa_query`）、
两种 stdio 模式、粘贴代码路径、文件未被改动、超时与取消时 worker 被杀。

### 4.2 端到端（关键验收）

```
$ MIRSA_ROOT=<repo> repo/target/mcp/venv/bin/python mac-port/vuln_demo_macos.py
...
全部符合预期：MIRSA 在 macOS 上工作正常。
```

### 4.3 按 DSH 实际启动方式再跑一遍

用 `dsh.cordis.yml` 里同样的 `bash -c exec <start-mcp.sh>` 拉起服务：

```
tools: ['mirsa_query', 'mirsa_variables']
proved: True | stmt: let value = unsafe { *s.get_unchecked(i) };
```

---

## 5. 改动的文件清单

| 文件 | 改动 |
|---|---|
| `repo/tools/mcp/server.py` | 打 DYLD 补丁（`worker_env()` 按平台选变量 + `import sys`）；路径接口改成 macOS 原生（见第 6 节）；WSL/Windows 文案全部清除 |
| `repo/tools/mcp/test_mcp.py` | 超时/取消测试的窗口在 darwin 上 0.3s → 2.0s；删掉 WSL 转接测试（macOS 用不上） |
| `repo/tools/mcp/setup-macos.py` | 新增（从 `mac-port/` 复制进来，让仓库自包含） |
| `repo/tools/mcp/setup.py` | 在 macOS 上直接退出并提示改用 `setup-macos.py`，避免误跑 Linux 脚本 |
| `repo/tools/mcp/README.md` | 安装/验证段落改成 macOS + `setup-macos.py` + Python ≥ 3.10；路径说明改为本机绝对路径 |
| `repo/tools/mcp/DSH接入.md` | 从 WSL 视角改写成 macOS 本机 + `start-mcp.sh` |
| `repo/tools/mcp/dsh.cordis.yml` | 从 `wsl.exe` 改成本机 `bash -c exec` |
| `mac-port/setup-macos.py` | `find_z3` 支持 `bin/` 与绝对路径；`find_llvm` 支持系统 CLT libclang；`linked_z3` 支持裸名回退；新增 `find_python`/`--python`；新增 `sdk_env()` 自动挑可用 SDK |
| `mac-port/dsh.cordis.yml` | 改成真实安装路径的 `bash -c exec` |
| `mac-port/vuln_demo_macos.py` | 未改（用 `MIRSA_ROOT` 指定仓库根即可，不必是 `~/mirsa`） |

`repo/tools/mcp/server.py.pre-dyld` 是补丁脚本自动留的备份，可删。

---

## 6. 工具接口改成 macOS 原生（第二轮）

macOS 上 agent 自己就能读本机文件，不需要「Windows 读不到 WSL 文件」那套转接，所以把接口层面的
WSL/Windows 残留全部拆了：

* `to_wsl_path()` → `to_local_path()`：只做「去引号 / 展开 `~` / 相对路径按 server cwd 解析」，
  不再翻译 `D:\...` 和 `\\wsl$\...`。找不到文件时的报错也改成「请给本机绝对路径」。
* **直接删掉 `mirsa_read_lines`**：它在 Linux/WSL 时代存在的唯一理由是 agent 读不到 WSL 文件；
  macOS 上 agent 直接用本机读文件工具即可，少一个会和原生读文件重复的工具。
  工具面现在是 2 个：`mirsa_variables`、`mirsa_query`。
* `test_mcp.py` 删掉 `test_windows_wsl_bridge`（原来在 macOS 上永远 skip），
  现在 5 个测试全跑、全过，没有 skip。
* 仓库内 `README.md` / `DSH接入.md` / `dsh.cordis.yml` 一起改成 macOS 视角，
  `setup.py` 在 darwin 上直接退出并指向 `setup-macos.py`。

实测路径行为：

```
relative  -> ok: <repo>/examples/inputs/query_target.rs
tilde     -> 找不到这个文件：~/mirsa_none.rs（解析为 /Users/pwt/mirsa_none.rs）；请给本机绝对路径
missing   -> 找不到这个文件：/tmp/nope.rs（解析为 /tmp/nope.rs）；请给本机绝对路径
```

改完重跑：`test_mcp.py` 5/5 通过，`vuln_demo_macos.py` 仍是 3/3 逐字一致。

---

## 7. 接回 DSH

```bash
dsh web --patch /Users/pwt/Documents/deepseek-harness/default-workspace/mirsa/mac-port/dsh.cordis.yml
```

- 配置里的 `toolCallTimeoutMs` 保持 120000（单个 `.rs` 首次分析要现场编译）。
- MCP 工具列表只在重连/重启时同步，**改完要新建会话**才看得到 `mirsa_query` / `mirsa_variables` 这 2 个工具。
- 当前这个 GUI（`http://127.0.0.1:52460`）不是用 `--patch` 起的，所以本次没有替你重启；
  需要重启时用上面的命令，或把这段 `insert` 合并进你平时的启动配置。

## 8. 如果一定要放到 `~/mirsa`

工作区被沙箱限制，写不了 `~/mirsa`。需要时（在你授权的 shell 里）执行：

```bash
mv /Users/pwt/Documents/deepseek-harness/default-workspace/mirsa/repo ~/mirsa
mkdir -p ~/mirsa/target/mcp
```

…并把 `deps/z3`、`deps/python` 一起搬走，同时改两处：
`setup-macos.py --z3-prefix`/`--python` 的路径，以及 `dsh.cordis.yml` 里的启动器路径。
**更省事的做法是保持现状**：所有产物路径都可以搬，但搬完必须重跑一遍第 4.2 节。

---

## 9. 把流程固化成 skill（精简版）

`mirsa-unsafe-check` 已安装并生效（skill 目录会热更新，本会话已重新加载）：

- 生效位置：`~/.dsh/skills/mirsa-unsafe-check/SKILL.md`（用户级，所有会话可见）
- 仓库副本：`mac-port/skills/mirsa-unsafe-check/SKILL.md`（改完重新复制过去）

只固定两件事：

1. **工具流程**：自己读文件拿行号 → `mirsa_variables` 核实变量名（强制）→ 查该函数在标准库文档里的
   Safety 要求、挑一条写成 DSL → `mirsa_query`（`file` 与 `code` 二选一，`line` 从 1 起，查语句执行前状态）。
2. **DSL grammar**：`crates/dsl/grammar.ebnf` 原样收进 skill，查 DSL 写法不用再翻仓库。

结论字段怎么读、汇报格式、模型边界这些**没有写进 skill**——它们本来就随工具返回和仓库文档走，
skill 里再写一遍只会重复。

按这个流程跑过一次 NonZero 例子：

```
mirsa_variables(code, line=5, candidates=["x"]) -> visible=["x"]
mirsa_query(code, line=5, dsl="$x != 0 at 'mirsa_line_entry") -> proved=true
```

---

## 10. 打包成 DSH 组合包

`dsh-bundle/` 是一个可安装的本地组合包（`@local/dsh-mirsa`）：

```
package.json          dsh.bundle.patch -> cordis.patch.yml
cordis.patch.yml      insert mcp-mirsa + skill-mirsa-filesystem
skills/mirsa-unsafe-check/SKILL.md
locale/{zh,en}.json   meta.title / meta.description
icon.svg
README.zh.md          安装与迁移说明
```

- MCP 部分沿用官方 MCP 模板：配置型行，插 `@deepseek-ai/dsh-mcp-client`（stdio）。
- skill 部分不依赖 `~/.dsh/skills`：单独插一个 `@deepseek-ai/dsh-skill-filesystem`，
  `includeDefaultRoots: false` + `customSkillDirs` 指向包内 `skills/`。
- 组合验证：用临时 `DSH_HOME` 跑 `dsh --profile web --dump-config --patch dsh-bundle/cordis.patch.yml`，
  两行都正确合成。
- 安装走 Web「插件」页或 `plugin_manager.install_bundle`；desktop profile 的 `dsh plugin` 命令被禁用。
- 迁移时要先删 profile patch 里手写的 mcp-mirsa 段，再删 `~/.dsh/skills` 副本，避免重复 id / 同名 skill。

### 10.1 安装结果（已执行）

```
plugin_manager install_bundle  target = mirsa/dsh-bundle
→ application: applied, warnings: [], pnpm 314ms
→ profile 依赖: @local/dsh-mirsa -> link:.../mirsa/dsh-bundle
→ bundles: [dsh-base, dsh-web-app, @local/dsh-mirsa]
```

迁移同时完成：

- 删除了 `cordis.patch.yml` 里手写的 `mcp-mirsa`（改由组合包提供），profile patch 现在只剩 `tool-plugin-manager` 一行覆盖。
- 删除 `~/.dsh/skills/mirsa-unsafe-check`，改由组合包自带的 provider 提供；备份在 `backup/skills-userlevel/`。
- 验证：`skill` 工具的 base directory 已是 `dsh-bundle/skills/...`；`mirsa_variables` 调用正常。

### 10.2 UI v1（静态卡片）

- `dsh-bundle/client.js`：`window.__ModuleLoader__.load({id:'@local/dsh-mirsa', ...})`，
  向 `conversation.composer.dock` 注册 React 组件。
- `dsh-bundle/index.js`：Host 侧空壳（`export function apply() {}`）。
- `package.json` 加了 `./client` 导出与 `dsh.client`（platform web / immediately / inject conversation UI）。
- 卡片内容：结论徽章（确定违反/已证明/未证明）+ API + DSL + evidence chips + 代码行；数据暂为 `RESULTS` 常量。
- 最终形态：`shell.overlay`（右侧 380px 固定面板）+ 标题栏 `MIRSA` 按钮开合，形成左/中/右三栏。
  打开时调用 `ctx.layout.closeRightbar()` 收起原生右栏，并给 `body` 加 `padding-right` 让中间真正变窄。
- 面板内：文件选择 / 粘贴代码 → 源码预览 → 「开始分析」→ `conversation.send(prompt)`，由 agent 走 skill。
- 放弃右侧栏 tab 路线：`sidebarRightTabs` 在 `apply` 时尚未提供，`ctx.effect` 在客户端又会打断 `apply`；
  轮询等待也只是把失败推迟，最终改用不依赖该服务的 `shell.overlay`。
