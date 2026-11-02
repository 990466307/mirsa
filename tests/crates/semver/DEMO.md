# DSH 演示：semver 1.0.25 原始 NonZero 调用

目标是 `src/identifier.rs` 原始第 122 行的 `NonZeroUsize::new_unchecked(len)`。

该调用位于 `9..=0xff_ffff_ffff_ffff` 分支，安全前提为 `len != 0`。只插入相应 MIRSA check 属性即可验证为 `proved`，不需要自造函数、泛型大小或布局表达式。

完整请求和实测步骤：[演示流程](../../../docs/demo-auto-annotate.md)。原始源码保持未标注；运行时使用工程副本，避免重复插入属性。

来源：本机 Cargo registry 中 semver 1.0.25 的发布源码；上游提交见 `.cargo_vcs_info.json`（b1e2848d0360fd80c5d86b1a5f70d94c6e602819）。保留上游源码、构建脚本、测试与许可证。只为隔离 MIRSA workspace 在 Cargo.toml 增加 `[workspace]`；未修改任何上游 Rust 源文件。

编译入口是 `src/lib.rs`，目标文件是 `src/identifier.rs`；MCP 请求传 `crate_root`、`file`、`lines="122"` 和 `rustc_args=["--edition=2018", "--cfg", 'feature="std"']`。
