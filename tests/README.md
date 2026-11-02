# 测试

本目录只保留 crate 级别与文件级别的分析测试。各 crate 源码内的单元测试仍随所属模块保留。

| 目录 | 内容 |
|---|---|
| `crates/` | 完整 Rust crate 的分析：从 crate 根文件加载多个模块，检查跨文件的查询结果和位置 |
| `files/` | 单文件分析：源码属性、输入 API、数值、指针、路径、内存关系和查询证据 |

自动测试的源码输入写在相应测试中，需要磁盘文件或多文件工程时创建临时文件并清理。

`crates/semver/` 是当前 DSH 演示工程：原始 `NonZeroUsize::new_unchecked(len)` 调用的完整非零前提可写成 DSL 并得到 `proved`。它保持未标注，独立于 MIRSA 的 Cargo workspace，供 agent 操作副本。流程见 [自动标注演示](../docs/demo-auto-annotate.md)。

`crates/bytemuck/` 保留用于分析能力研究；此前的引用非空结果不足以验证泛型转换的关键前提，不再作为主演示。

## 运行

```bash
cargo test --locked --workspace --features z3
```

嵌套目录的 Rust 测试在根 `Cargo.toml` 中通过 `[[test]]` 显式注册。新增测试文件时需同时注册，避免遗漏。

可按目标运行：

```bash
cargo test --locked --features z3 --test crate_modules
cargo test --locked --features z3 --test query_slice
```

去掉 `--features z3` 可验证不启用求解器的分析行为。

## API 示例

可运行的调用示例保留在 [examples/query_annotations.rs](../examples/query_annotations.rs)，使用内置源码，不依赖测试目录：

```bash
cargo run --locked --features z3 --example query_annotations
```
