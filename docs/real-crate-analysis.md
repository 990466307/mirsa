# 真实 crate 上的分析记录

> 历史记录：本文保留当时的路径和结果；bytemuck 演示工程现已恢复到 `tests/crates/bytemuck/`，并移除了下文历史记录中的自造示例模块，当前可复现步骤见 [自动标注演示](demo-auto-annotate.md)；semver 现从原始 registry 源码恢复，当前主演示改用其中第 122 行的 NonZero 调用，不采用下文三处未证明案例。其他旧演示输入已移除，测试结构与运行方式见 [tests/README.md](../tests/README.md)。

目标：在**一个完整 crate**里给出可准确判定的安全性质，而不是只跑单文件样例。

## 一、本地语料盘点（2026-11）

| 位置 | 内容 | 能否用于分析 |
|---|---|---|
| `RPL` / `RPL_next/tests` | 单文件模式与 UI 用例 | 否（非工程） |
| `RSAT-PoC-main/data/input` | 125 个 CVE 目录：README + patch + repro；依赖目录（`bitvec/` 等）为 0 B | 否（无源码） |
| `rapx-rpl-comparison/*/sr-targets` | 1.4 GB / 228 个目录，全是 cargo 构建缓存 | 否（无源码） |
| `Safe4U-replication/data` | JSON 片段，带 repo/文件/行号，但非完整工程 | 仅能提取函数 |
| `~/.cargo/registry/src` | 323 个真实 crate 源码 | **是** |

结论：真实工程源码只能从 cargo registry 取；CVE 语料缺源码，patch 也多为逻辑/cfg 修复，不是可注解的内存安全前提。

## 二、semver 1.0.25（真实 crate，3 条性质，均未证明）

在 crate 内部 3 处 unsafe 加注解：

| 位置 | 性质 | 结果 | 缺口 |
|---|---|---|---|
| `identifier.rs:343` | `nonzero_bytes != 0` | maybe_violated | 位级推理（`leading_zeros`） |
| `identifier.rs:355` | `len <= 8` | maybe_violated | 跨函数摘要（`inline_len`） |
| `identifier.rs:378` | `(first & 0x7f) != 0` | maybe_violated | varint 编码约束 |

证据均为 `¬P = sat`（存在反例模型），说明当前前提不足以证明，而非用例写错。

## 三、bytemuck 1.21.0（完整 crate + 一个示例模块）

在真实 crate 内新增 `src/mirsa_example.rs`，调用它自己的
`internal::try_from_bytes::<u32>`（要求 `s.len() == size_of::<u32>()`），
用具体类型与局部数组给出三种调用：

| 函数 | 性质 | 结果 | 证据 |
|---|---|---|---|
| `read_u32_exact()` | `len(bytes) == 4` | **proved** | `len(bytes) = 4` |
| `read_u32_short()` | `len(bytes) == 4` | **violated** | `len(bytes) = 3` |
| `read_u32_guarded(&[u8])` | `len(bytes) == 4` | **proved** | `len(bytes) = 4` |

整 crate 通过 `src/lib.rs` 一次编译（12 个模块），这就是「完整 crate 中准确分析」
的最小可复现例子。

## 四、经验

1. **能证明的**：具体类型 + 局部常量/守卫决定的下标、长度、非零、非空。
2. **证明不了的**：位级运算（`leading_zeros`/`0x7f`）、跨函数返回值约束、泛型大小
   （`size_of::<T>()`、常量泛型 `N`）。
3. **找例子的顺序**：先在 registry 里挑无依赖的小 crate → `mirsa_query(file=src/lib.rs)`
   确认能编译 → 在其内部挑具体类型的 unsafe 入口 → 加注解验证。
4. 若 crate 内部没有合适的入口，**在 crate 里加一个模块**调用它自己的 unsafe API
   是可行的（如上例），既保持「完整 crate」形态，又能给出准确结论。


## 五、自动标注 + 复杂项目

演示脚本见 [demo-auto-annotate.md](demo-auto-annotate.md)：用未加注解的 bytemuck
原始 crate，让 agent 现场补性质并验证（5 条 proved、1 条 violated）。

如果 crate 里已有边界断言，也可以用 `scripts/auto_annotate.py` 把
`debug_assert!(i < x.len())` 直接转成注解并分析，不依赖模型。
