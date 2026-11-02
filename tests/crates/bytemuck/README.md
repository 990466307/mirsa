# bytemuck 1.21.0：分析能力研究样本

> 已撤下此前的主演示推荐。这里的 `proved` 主要重复证明引用参数非空，不能验证泛型转换的关键布局前提。可运行的完整非零前提演示请使用 [semver](../semver/DEMO.md)。

从此前保存的 bytemuck 1.21.0 源码恢复，现已移除本地添加的 `mirsa_example.rs` 及其模块声明。演示直接使用 `internal::try_cast_slice` 和 `fill_zeroes` 的原有函数体，不添加包装函数、构造输入或修改分支条件。

源码保持未标注，供 DSH agent 按文件和行范围现场添加查询。

| 目标 | 原始行范围 | 可复现结果 |
|---|---|---|
| `src/internal.rs`，`try_cast_slice` | 353–374 | 两个 `from_raw_parts` 调用点的 `!is_null(a)` 均为 `proved` |
| `src/lib.rs`，`fill_zeroes` | 548–558 | 写入前的 `!is_null(slice)`、`len <= len(slice)` 均为 `proved` |

当前主演示已迁移至 [semver 演示说明](../../../docs/demo-auto-annotate.md)。从 `src/lib.rs` 分析整个 crate，并传 `rustc_args=["--edition=2018"]`。

这里只验证非空与写入元素数量边界，不代表已证明所有泛型布局、对齐、初始化及别名条件。没有发现可据实展示为 `violated` 的内存安全漏洞；不通过否定正确性质或修改源码制造该结果。

`Cargo.toml` 添加独立 `[workspace]`，移除了未随样本附带的上游测试目标，原有可选依赖和 features 保留。MIRSA 直接编译默认 crate 入口已通过；本机 Cargo 离线检查缺少 `bytemuck_derive` 索引缓存，演示不依赖该可选功能。

来源版本：bytemuck 1.21.0。许可证：MIT / Apache-2.0 / Zlib，文件均保留。此次验证采用恢复的本地源码；未从网络重新下载上游发布包。
