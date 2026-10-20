# MIRSA

MIRSA 是基于 Rust MIR 的静态分析库，通过抽象解释、符号关系和按需约简获取程序信息，在性质查询时可使用 Z3 补充证明。用户或 RPL 等工具通过 Rust 接口查询抽象信息、验证指定性质。

当前 GitHub 源码版本为 `0.4.0`，接口与 crates.io 已发布的 `0.3.0` 不同，请使用本仓库源码。

## 安装与运行

使用 Linux 或 Windows WSL。需要 Rust `nightly-2025-02-14`，启用 Z3 时需要 Z3 和 Clang 开发库。Ubuntu / Debian 可按以下步骤准备：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libz3-dev libclang-dev
git clone https://github.com/990466307/mirsa.git
cd mirsa
rustup toolchain install nightly-2025-02-14 --component rustc-dev,rust-src,rustfmt
export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo run --locked --features z3 --example query_dsl
```

该例会验证切片下标边界，再打印切片的抽象信息和符号关系。未启用 `z3` 时仍可使用抽象解释，但无法使用 SMT 补充证明。

## 查询示例

在目标语句前标注位置：

```rust
// mirsa: access
let value = unsafe { *s.get_unchecked(i) };
```

在查询程序中绑定变量，再提交一条 DSL：

```rust
let bindings = SourceBindings::new()
    .variable("slice", "s")
    .variable("index", "i");
let check = Mirsa::from_file("input.rs")
    .query("$index < len($slice) at 'access", &bindings)?;
let info = Mirsa::from_file("input.rs")
    .query("facts($slice) at 'access", &bindings)?;
```

查询的是语句执行前的状态；标注名须唯一，MIRSA 自动确定函数。DSL 每次一条，不带赋值名称和末尾分号。完整可运行程序见 [使用流程](docs/使用流程.md)。

## 返回什么

性质查询返回 `Output::Check(CheckReport)`：`result=true` 表示该性质已证明；`false` 表示未证明。进一步查看 `definite_violation`、`unreachable` 和 `reason`，区分模型内确定违反、不可达和信息不足等情况。编译、绑定和查询错误返回 `Err`。

`facts($slice)` 返回 `Output::Facts(FactOutput)`，包含 `abstract_info`（范围、长度、空性、指向对象等）和 `symbolic_info`（结构化表达式与关系）。每次查询一个值，相关关系可以涉及其他变量。

## 接入自己的方法

| 入口 | 用法 |
|---|---|
| Rust 源码 | `Mirsa::from_file(path)` 或 `Mirsa::from_source(code)`，配合 `SourceBindings` |
| 已有 MIR 的工具 | `Mirsa::from_mir(tcx, body)`，通过 `query::Bindings` 绑定 `Place` / `Operand` 和 `Location` |

当前主要支持数值、位运算、长度、指针空性及部分内存关系；本地函数可使用上下文相关摘要。未知外部调用、循环和路径合并可能损失精度。单条性质成立不代表整个 unsafe 调用或文件安全；例如非空性不能替代边界、对齐、有效性等其他前提。

[使用流程与结果类型](docs/使用流程.md) · [DSL 完整文法](crates/dsl/grammar.ebnf)

开发验证：`cargo test --locked --workspace --features z3`；格式检查：`cargo fmt --all -- --check`。

可选：[Agent / MCP 适配层](tools/mcp/README.md)，供 DSH 等 Agent 调用；使用 MIRSA 本体不需要安装它或 Python。
