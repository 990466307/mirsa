# MIRSA

MIRSA 是基于 Rust MIR 的静态分析库，通过抽象解释、符号关系和按需约简获取程序信息，在性质查询时可使用 Z3 补充证明。用户或 RPL 等工具通过 Rust 接口查询抽象信息、验证指定性质。

当前 GitHub 源码版本为 `0.4.0`，接口与 crates.io 已发布的 `0.3.0` 不同，请使用本仓库源码。

## 安装与运行

支持 Linux、macOS 与 Windows WSL。需要 Rust `nightly-2025-02-14`；启用 Z3 时需要 Z3 和 Clang 开发库。
Ubuntu / Debian 可按以下步骤准备（macOS 见文末「MCP 与 DSH 插件」）：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libz3-dev libclang-dev
git clone https://github.com/990466307/mirsa.git
cd mirsa
rustup toolchain install nightly-2025-02-14 --component rustc-dev,rust-src,rustfmt
export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo run --locked --features z3 --example query_annotations
```

该例会验证切片下标边界，再打印切片的抽象信息和符号关系。未启用 `z3` 时仍可使用抽象解释，但无法使用 SMT 补充证明。

## 源码属性查询

在自己的 Rust 文件中，语句前写 `#[mirsa::check(index < len(table))]` 或 `#[mirsa::facts(table)]`，即可自动解析源码变量并返回逐路径证据。将下列路径替换为待分析文件：

```bash
cargo run --locked --features cli,z3 --bin mirsa-query -- /path/to/your.rs
```

Rust API 使用 `Mirsa::from_file(path).analyze_annotations()`。属性中调用形式的 `len(...)` 是 builtin，单独的 `len` 是源码变量，不需要 dollar 符号。[完整用法](docs/使用流程.md#直接使用源码属性)。

## 仓库结构

```
crates/        分析引擎（9 个 Rust crate）
src/           对外 API 与 mirsa-query worker
tools/mcp/     Python MCP 服务（mirsa_query 工具）
dsh-plugin/    DSH 插件，npm 包 dsh-mirsa（会话里的源码 + 行内卡片）
examples/      cargo 示例（cargo run --example …）
tests/         自动测试，按 crates / files 分类
scripts/       安装、启动 MCP、自动标注等辅助脚本
docs/          文档
third-party/   下载的 z3 与 Python（构建用二进制，可删，setup 会重建）
```

## MCP 与 DSH 插件

macOS 上一条命令准备运行环境：

```bash
python3 scripts/setup.py
```

接入方式见 [docs/mcp.md](docs/mcp.md)。DSH 插件在 `dsh-plugin/`，发布为 npm 包 `dsh-mirsa`。

## 查询示例

```rust
pub fn example(table: &[u8], index: usize) -> u8 {
    if index < table.len() {
        #[mirsa::check(index < len(table))]
        #[mirsa::facts(table)]
        unsafe { *table.get_unchecked(index) }
    } else { 0 }
}
```

查询被标注语句执行前的状态。`Mirsa::from_file(path).analyze_annotations()` 或 `Mirsa::from_source(code).analyze_annotations()` 一次返回源码中的全部属性查询，不需要另外提供位置名、DSL 字符串或变量绑定。错误显示属性表达式中的位置和简短原因。

## 返回什么

性质查询返回 `Output::Check(CheckReport)`，`status` 为 `proved`（已证明）、`violated`（模型内存在确定违反的路径）、`maybe_violated`（模型中可能违反）或 `unreachable`（不可达）。各类结论均由逐路径 `evidence` 说明；`result=true` 仍表示已证明。Z3 超时或未完成判定时，采用抽象分析结论和证据，在路径的 `notes` 中说明，不作为查询错误。编译、绑定、不支持的查询和抽象分析未完成等情况返回 `Err`。

`facts($slice)` 返回 `Output::Facts(FactOutput)`，包含逐路径的 `paths`；每个路径保留 `path_id`、`merged`、`abstract_info`（范围、长度、空性、指向对象等）和 `symbolic_info`（结构化表达式与关系）。不可达时 `unreachable=true` 且 `paths` 为空。性质查询的每条路径还包含 `evidence`，记录抽象求值树或 SMT 前提、目标和实际求解结果。每次查询一个值，相关关系可以涉及其他变量。

## 接入自己的方法

| 入口 | 用法 |
|---|---|
| Rust 源码 | `Mirsa::from_file(path)` 或 `Mirsa::from_source(code)`，调用 `.analyze_annotations()` |
| 已有 MIR 的工具 | `Mirsa::from_mir(tcx, body)`，通过 `query::Bindings` 绑定 `Place` / `Operand` 和 `Location` |

当前主要支持数值、位运算、长度、指针空性及部分内存关系；本地函数可使用上下文相关摘要。未知外部调用、循环和路径合并可能损失精度。单条性质成立不代表整个 unsafe 调用或文件安全；例如非空性不能替代边界、对齐、有效性等其他前提。

[使用流程与结果类型](docs/使用流程.md) · [属性表达式文法](crates/dsl/grammar.ebnf)

开发验证：`cargo test --locked --workspace --features z3`；格式检查：`cargo fmt --all -- --check`。

可选：[Agent / MCP 适配层](tools/mcp/README.md)，供 DSH 等 Agent 调用；使用 MIRSA 本体不需要安装它或 Python。
