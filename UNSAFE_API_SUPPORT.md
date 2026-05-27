# Unsafe API 支持情况

本文档总结了 `mirsa` 当前两个分析域已经覆盖的 unsafe API。

## interval 域

入口文件：
- [`crates/domains/src/interval/warnings/mod.rs`](/home/wentao/mirsa/crates/domains/src/interval/warnings/mod.rs)

当前支持的 unsafe API：
- `NonZero::new_unchecked`
  - 规则文件：
    [`crates/domains/src/interval/warnings/nonzero.rs`](/home/wentao/mirsa/crates/domains/src/interval/warnings/nonzero.rs)
  - 安全性质：
    参数必须非零

- `slice::get_unchecked`
- `slice::get_unchecked_mut`
- `slice::split_at_unchecked`
- `slice::split_at_mut_unchecked`
  - 规则文件：
    [`crates/domains/src/interval/warnings/slice_bounds.rs`](/home/wentao/mirsa/crates/domains/src/interval/warnings/slice_bounds.rs)
  - 安全性质：
    - `get_unchecked` / `get_unchecked_mut`：`index < len`
    - `split_at_unchecked` / `split_at_mut_unchecked`：`mid <= len`

- `ptr::copy_nonoverlapping`
  - 规则文件：
    [`crates/domains/src/interval/warnings/copy_nonoverlapping.rs`](/home/wentao/mirsa/crates/domains/src/interval/warnings/copy_nonoverlapping.rs)
  - 安全性质：
    `count` 不能超过源对象和目标对象的可用长度

辅助函数：
- [`crates/domains/src/interval/warnings/shared.rs`](/home/wentao/mirsa/crates/domains/src/interval/warnings/shared.rs)

## Nullptr 域

入口文件：
- [`crates/domains/src/nullptr/warnings/mod.rs`](/home/wentao/mirsa/crates/domains/src/nullptr/warnings/mod.rs)

当前支持的 unsafe API：
- `NonNull::new_unchecked`
- `CStr::from_ptr`
- `Vec::from_raw_parts`
- `slice::from_raw_parts`
- `slice::from_raw_parts_mut`
- `ptr::read`
- `ptr::write`
  - 规则文件：
    [`crates/domains/src/nullptr/warnings/nonnull_arg.rs`](/home/wentao/mirsa/crates/domains/src/nullptr/warnings/nonnull_arg.rs)
  - 安全性质：
    指针参数必须非空

- `ptr::copy_nonoverlapping`
- `intrinsics::copy_nonoverlapping`
  - 规则文件：
    [`crates/domains/src/nullptr/warnings/copy_nonoverlapping.rs`](/home/wentao/mirsa/crates/domains/src/nullptr/warnings/copy_nonoverlapping.rs)
  - 安全性质：
    源指针和目标指针参数都必须非空

辅助函数：
- [`crates/domains/src/nullptr/warnings/shared.rs`](/home/wentao/mirsa/crates/domains/src/nullptr/warnings/shared.rs)

## 最终测试集

当前的最终数据集主要覆盖上述 API，并保留对应 Safe4U 原始片段：
- 数值类，22 个样例：
  [`examples/safe4u_final_numeric`](/home/wentao/mirsa/examples/safe4u_final_numeric)
- 指针类，21 个样例：
  [`examples/safe4u_final_pointer`](/home/wentao/mirsa/examples/safe4u_final_pointer)

更详细的测试集说明见：
- [`TEST_SUITE.md`](/home/wentao/mirsa/TEST_SUITE.md)
