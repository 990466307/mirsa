# 测试集说明

`examples` 下保留两组 Safe4U 衍生示例，所有样例都有对应的 `original/*.orig.rs`，其中记录来源。它们用于驱动程序的内置检查回归；通用查询接口另有工作区测试。

## safe4u_final_numeric

路径：[`examples/safe4u_final_numeric`](examples/safe4u_final_numeric)

共 22 个样例，覆盖数值类安全性质：
- 非零约束：`NonZero::new_unchecked`
- 切片边界：`get_unchecked`、`get_unchecked_mut`、`split_at_unchecked`、`split_at_mut_unchecked`
- 内存复制长度：`ptr::copy_nonoverlapping`

新增的 17-22 号样例来自 Safe4U 中的 tormol、rust-lexical、encoding_rs、bitvec，和已有样例不重复。重构版本保持单文件、可独立编译分析，并将核心 unsafe 调用内联在 `main` 中。

## safe4u_final_pointer

路径：[`examples/safe4u_final_pointer`](examples/safe4u_final_pointer)

共 21 个样例，覆盖指针非空安全性质：
- `NonNull::new_unchecked`
- `CStr::from_ptr`
- `Vec::from_raw_parts`
- `slice::from_raw_parts`
- `ptr::read` / `ptr::write`
- `ptr::copy_nonoverlapping`

新增的 19-21 号样例来自 Safe4U 中的 funty 和 bumpalo，和已有样例不重复。重构版本保留原始 API 的核心调用逻辑，同时去掉与性质无关的包装类型。

## 查询接口回归

准备环境后，在仓库根目录运行：

```bash
cargo test --locked --workspace --features cli,z3
```

包括 DSL、求解器、路径、内存关系、函数摘要及源码/MIR 入口等测试。Agent 接口另运行 `target/mcp/venv/bin/python tools/mcp/test_mcp.py`，见 [MCP 说明](tools/mcp/README.md)。

## 驱动程序批量回归

默认批量测试只运行两个主测试集：

```bash
scripts/run_examples.sh
```

期望告警写在 [`examples/expected_warnings.tsv`](examples/expected_warnings.tsv)。如需只跑单个测试集或单个文件，可把路径作为参数传给脚本。

这些源码包含刻意违反安全前提的操作，批量脚本仅编译分析，不执行其 `main`。上述历史期望告警不能用来表示通用查询接口的完整覆盖率。
