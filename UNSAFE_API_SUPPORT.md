# 内置检查与查询能力

MIRSA 的通用入口是 DSL 查询，支持的表达式见 [完整文法](crates/dsl/grammar.ebnf)，使用方法见 [README](README.md)。下表仅介绍驱动程序保留的内置检查，不代表每个 API 的全部安全要求都已覆盖。

| 内置检查 | 涉及的 API | 实现 |
|---|---|---|
| 非零 | `NonZero::new_unchecked` | [nonzero.rs](crates/contracts/src/interval/nonzero.rs) |
| 下标、切分位置边界 | 切片 `get_unchecked` / `get_unchecked_mut` / `split_at_unchecked` / `split_at_mut_unchecked` | [slice_bounds.rs](crates/contracts/src/interval/slice_bounds.rs) |
| 复制长度 | `copy_nonoverlapping` 的源和目标空间 | [copy_nonoverlapping.rs](crates/contracts/src/interval/copy_nonoverlapping.rs) |
| 指针非空 | `NonNull::new_unchecked`、`CStr::from_ptr`、`Vec::from_raw_parts`、切片 `from_raw_parts` / `from_raw_parts_mut`、`ptr::read` / `ptr::write` | [nonnull_arg.rs](crates/contracts/src/nullptr/nonnull_arg.rs) |
| 复制指针非空 | `copy_nonoverlapping` 的源和目标指针 | [copy_nonoverlapping.rs](crates/contracts/src/nullptr/copy_nonoverlapping.rs) |

例如 `CStr::from_ptr` 的非空性通过，不代表 NUL 终止、内存有效性等要求已经满足。通用 DSL 的性质验证和以上内置检查是不同入口；接入自己的方法或 Agent 时，按需要选择具体性质。

已有 Safe4U 衍生示例及验证方式见 [测试说明](TEST_SUITE.md)。
