# 可复现演示：semver 原始实现的 NonZero 前提

当前主演示使用真实发布 crate **semver 1.0.25** 的原始函数，不使用自造函数或泛型布局查询。此前推荐的 bytemuck `internal.rs:353–374` 只证明引用非空，不能满足这个演示目标，已撤下该推荐。

## 工程和目标

- 工程：[tests/crates/semver](../tests/crates/semver/DEMO.md)。源码从本机 Cargo registry 的 1.0.25 发布包复制，`src/` 与缓存逐文件一致，保持未标注。
- 文件：`src/identifier.rs`。
- 目标：原始 **第 122 行**，`NonZeroUsize::new_unchecked(len)`。
- 所在函数：`Identifier::new_unchecked`，第 106 行开始。
- 实际类型：`len: usize`，无泛型大小或对齐查询。
- 该构造调用的安全前提：`len != 0`。
- 源码依据：调用位于 `match len as u64` 的 `9..=0xff_ffff_ffff_ffff` 分支中。

在原始第 122 行前插入：

```rust
#[mirsa::check(len != 0)]
```

这条属性的位置仍是第 122 行，原调用变为第 123 行；不修改已有分支和业务代码。

## 直接给 DSH 的请求

把 `tests/crates/semver/` 作为 DSH 工作目录，使用已重连的本仓库 MIRSA MCP 服务：

> 检查当前 semver crate 的 src/identifier.rs 原始第 122 行，只针对 NonZeroUsize::new_unchecked(len) 这个调用。读取所在 match 分支，识别这个调用的非零安全前提，并在它前面自动添加 MIRSA check 属性，不修改原有代码逻辑。用 file 指定 identifier.rs 的绝对路径、crate_root 指定 src/lib.rs 的绝对路径，lines 指定插入属性后的行号；rustc_args 使用 ["--edition=2018", "--cfg", "feature=\"std\""]。调用 mirsa_query，报告该调用前提的结论和证据，不扩展成整个函数安全。

对应的工具请求如下，将路径替换为当前演示工程的绝对路径：

```python
mirsa_query(
    file="/绝对路径/semver/src/identifier.rs",
    crate_root="/绝对路径/semver/src/lib.rs",
    lines="122",
    rustc_args=["--edition=2018", "--cfg", 'feature="std"'],
    verbose=True,
)
```

当前 MCP 已支持 `crate_root` 和 `lines`：从入口编译整个 crate，但只返回目标文件和行范围的查询；响应里的源码也是目标文件，供卡片显示。DSH 需要重新连接 MCP 服务以刷新工具参数，且该服务必须指向本仓库的 `scripts/start-mcp.sh`，不是另一份旧安装。

## 已完成的实测

2026-11-02，在独立副本上验证：

1. 原始无注解工程，通过实际 `scripts/start-mcp.sh` 启动的 **stdio MCP** 查询，返回 0 条报告。
2. 只在原调用前插入 `len != 0` 属性。
3. 传目标文件、crate 入口和 `lines="122"`，返回恰好 1 条报告：`proved`，查询点可达。
4. 证据包含分支下界 `_11 >= 9u64` 和上界；SMT 结果 `¬P = unsat`、`P = sat`。
5. 改成 `lines="123"` 或其他文件，返回 0 条报告，验证筛选没有串位置。
6. 已安装 worker 与当前源码构建的 worker 均得到相同的 `proved`。
7. 原始 crate 的 Cargo 离线构建检查通过；本机需使用能工作的 macOS SDK 26，直接 MIRSA 查询不依赖 Cargo 构建脚本。

这里已验证原始源码、有效 DSL 注解、crate 编译、指定文件/行范围和 stdio MCP 返回全过程。**未实际驱动 DSH 模型生成注解或检查其界面卡片**，所以不能将这些结果说成 agent 自主执行成功率保证。

## 范围与重置

`NonZeroUsize::new_unchecked` 这一调用的非零前提是完整且可表达的；函数内其他 copy、transmute、allocation 调用不在本次结论中。

原始样本没有注解。重复演示最好操作工程副本；删除仅新增的 `#[mirsa::check(len != 0)]` 即可恢复行号。不要运行固定规则自动标注脚本代替 DSH，再声称验证了 agent 的自动标注。
