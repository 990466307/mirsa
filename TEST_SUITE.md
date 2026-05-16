# 测试集说明

`examples` 下当前保留两个主测试集，所有样例都有对应的 `original/*.orig.rs`，其中记录 Safe4U 的 `sample_label`、仓库名、相对路径和函数名。

## safe4u_final_numeric

路径：[`examples/safe4u_final_numeric`](/home/wentao/mirsa/examples/safe4u_final_numeric)

共 22 个样例，覆盖数值类安全性质：
- 非零约束：`NonZero::new_unchecked`
- 切片边界：`get_unchecked`、`get_unchecked_mut`、`split_at_unchecked`、`split_at_mut_unchecked`
- 内存复制长度：`ptr::copy_nonoverlapping`

新增的 17-22 号样例来自 Safe4U 中的 tormol、rust-lexical、encoding_rs、bitvec，和已有样例不重复。重构版本保持单文件、可独立编译运行，并将核心 unsafe 调用内联在 `main` 中。

## safe4u_final_pointer

路径：[`examples/safe4u_final_pointer`](/home/wentao/mirsa/examples/safe4u_final_pointer)

共 21 个样例，覆盖指针非空安全性质：
- `NonNull::new_unchecked`
- `CStr::from_ptr`
- `Vec::from_raw_parts`
- `slice::from_raw_parts`
- `ptr::read` / `ptr::write`
- `ptr::copy_nonoverlapping`

新增的 19-21 号样例来自 Safe4U 中的 funty 和 bumpalo，和已有样例不重复。重构版本保留原始 API 的核心调用逻辑，同时去掉与性质无关的包装类型。

## 批量运行

默认批量测试只运行两个主测试集：

```bash
scripts/run_examples.sh
```

期望告警写在 [`examples/expected_warnings.tsv`](/home/wentao/mirsa/examples/expected_warnings.tsv)。如需只跑单个测试集或单个文件，可把路径作为参数传给脚本。
