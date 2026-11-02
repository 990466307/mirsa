# dsh-mirsa

把 MIRSA 接入 DSH 的插件：一条 MCP 连接、一个随包发布的 skill，以及会话里的结论卡片。

源码里写注解，agent 调用 MIRSA 验证并给出带证据的结论：

```rust
#[mirsa::check(index < len(table))]
unsafe { *table.get_unchecked(index) }
```

## 快速开始

```bash
# 1. 安装插件（DSH profile 目录里；或在插件页安装）
pnpm add dsh-mirsa

# 2. 一条命令装好 MIRSA（源码、Rust 工具链、z3、Python、worker）
npx dsh-mirsa setup

# 3. 重启 DSH
```

`setup` 默认安装到 `~/mirsa`，这正是插件默认查找的位置，所以**不需要额外配置**。

## setup 做了什么

| 步骤 | 内容 |
|---|---|
| 源码 | 解包内置的 MIRSA 源码到 `--root`（默认 `~/mirsa`） |
| 工具链 | 安装 rustup（如缺）与 `nightly-2025-02-14` + `rustc-dev`、`rust-src` |
| z3 | 使用 `--z3-prefix`、`~/mirsa/third-party/z3`、Homebrew 的 z3，缺失时 `brew install z3` |
| Python | 找 >= 3.10；缺失时 `brew install python@3.12`（mcp 2.3.0 的要求） |
| 构建 | 调用 `scripts/setup.py`：编译 worker、创建 venv、部署 `target/mcp` |

常用选项：

```bash
npx dsh-mirsa setup --root /opt/mirsa      # 换安装目录（记得 export MIRSA_ROOT）
npx dsh-mirsa setup --z3-prefix /opt/homebrew/opt/z3
npx dsh-mirsa setup --skip-toolchain       # 工具链已就绪时跳过
```

首次约需 1–2 GB 下载（Rust 工具链为主），之后重复执行是增量的。

网络受限时（例如 PyPI 的 IPv6 不通），可以换 pip 源：

```bash
PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple npx dsh-mirsa setup
```

## 手动安装（可选）

已有 MIRSA 仓库并构建过 `target/mcp` 时，只要让插件找到它：

```bash
export MIRSA_ROOT=/path/to/mirsa
```

## 使用

1. 在待检查的 unsafe 语句上方加注解（可多条）：
   ```rust
   #[mirsa::check(index < len(data))]
   #[mirsa::check(3 < len(table))]
   unsafe { ... }
   ```
2. 让 agent “检查这个文件的安全性质”，它会调用 `mirsa_query`。
3. 结论以卡片形式出现在会话末尾：性质、`status`（proved / violated /
   maybe_violated / unreachable），以及抽象解释 facts 或 SMT 的
   `¬P = unsat, P = sat` 证据。

`#[mirsa::facts(x)]` 只在需要单独查看某个变量的抽象状态时使用；`check` 的结论已带证据。

## 说明

- 只在 macOS（arm64）上验证过。
- 单条性质成立不代表整个 unsafe 调用安全；分析只看函数体，不含调用方实参。
- `setup` 需要网络访问 crates.io、PyPI、GitHub（rustup 与 Homebrew 的安装包）。

## 右栏逐行验证结果

Rust 文件使用文档预览中的 `Rust · 验证结果` 渲染器：注解行旁显示 ✅（proved）、❌（violated）或 ❔（未定论／不可达）。点击标记展开该行所有性质、路径证据和 SMT 结果；原始证据默认折叠。结果从当前会话的工具记录订阅，不依赖聊天卡片是否显示。

更新插件后刷新 DSH 页面。已有标签若保留了旧的“代码”选项，在右栏“打开方式”选择一次 `Rust · 验证结果`；新打开的 Rust 文件优先使用此预览。当前文件必须与分析结果的源码快照完全相同，文件变化后隐藏旧标记并提示重新验证。结果仅属于当前会话，不能跨会话借用。此预览读取完整文件，受 DSH 文件大小上限约束；缺失或截断的分析快照不能展示为有效结果。
