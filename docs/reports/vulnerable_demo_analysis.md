# vulnerable_demo.rs 的 unsafe 安全性分析

> 历史记录：本文保留当时的路径和结果；旧演示输入已移除，当前测试结构与运行方式见 [tests/README.md](../../tests/README.md)。

- 源文件（WSL 原始路径）：`/home/wentao/mirsa/examples/inputs/vulnerable_demo.rs`
- 本地副本：[vulnerable_demo.rs](./vulnerable_demo.rs)（内容逐字一致，35 行）
- 分析工具：MIRSA MCP（`mirsa_read_lines` / `mirsa_variables` / `mirsa_query_at`）
- 判定口径：对每处 `get_unchecked`，在**该语句执行前**判定其内存安全前置条件 `index < len(slice)` 是否被证明。

## 结论总览

文件里没有 `unsafe fn`，是 3 个包含 `unsafe` 块的函数。

| 函数 | unsafe 位置 | 查询性质 | verdict | 判定 |
|---|---|---|---|---|
| `definitely_out_of_bounds` | 第 12 行 | `$index < len($table)` | **F**，`definite_violation = true` | **确定违反（真漏洞）** |
| `looks_like_off_by_one` | 第 21 行 | `$index < len($data)` | **T** | 已证明 |
| `correctly_guarded` | 第 31 行 | `$index < len($data)` | **T** | 已证明 |

即：**第 1 个函数不安全，后 2 个安全**（在其自身守卫与调用方契约成立的前提下）。

## 1. `definitely_out_of_bounds`（第 12 行）—— 确定违反

查询：

```
$index < len($table) at 'mirsa_line_entry
```

返回 `verdict = F`、`definite_violation = true`、`paths = 1`、来源 `Abstract`（抽象解释直接算出常量）。

补充查询 `$index == 5 && len($table) == 3` → **T**，说明进入该语句时：

- `table = [10u8, 20, 30]`，长度恒为 3；
- `index = 5usize`，是常量。

`5 < 3` 不成立且**没有任何输入能改变它**，因此 `get_unchecked(5)` 是无条件越界读，属未定义行为（UB）。这里的 F 不是"求解器能力不足"，而是"已证明不安全"（`definite_violation = true`）。

## 2. `looks_like_off_by_one`（第 21 行）—— 已证明安全

查询 `$index < len($data)` → **T**，来源 `Solver`，单路径。

守卫 `if index <= data.len() - 1` 对无符号数而言：

- 当 `len >= 1`：`index <= len - 1` 恰好等价于 `index < len`；
- 当 `len == 0`：`data.len() - 1` 先发生 usize 下溢（debug 下 panic、release 下按溢出语义终止该路径），**到不了 unsafe 块**。

补充查询守卫本身 `$index <= len($data) - 1` → **T**，与上面的分析一致。这正是文件注释强调的"纠正直觉"：看起来像差一位越界，形式化结论是安全的。

## 3. `correctly_guarded`（第 31 行）—— 已证明安全

守卫 `if index < data.len()` 与 unsafe 块所需前提同形，查询 `$index < len($data)` → **T**，来源 `Solver`，单路径。标准写法。

## 结论边界（务必区分三种结论）

- **已证明（T）**：指"在该程序点，该性质成立"，不覆盖函数的其他缺陷（逻辑错误、契约被滥用等）。
- **确定违反（F 且 `definite_violation = true`）**：真问题，附带的证据可解释原因。
- **未证明（F 且 `definite_violation = false`）**：只说明约束不足或超出求解能力，**既不等于安全，也不等于漏洞**。本次三处查询都没有落入这一类。
- 第 21 行的证明依赖"`len == 0` 时减法先 panic 截断路径"这一事实；若守卫改写成 `wrapping_sub` 等形式，该证明不再成立。

## 本次查询的原始参数

| 行号 | DSL | bindings | verdict |
|---|---|---|---|
| 12 | `$index < len($table) at 'mirsa_line_entry` | `{"index":"index","table":"table"}` | F / definite_violation=true |
| 12 | `$index == 5 && len($table) == 3 at 'mirsa_line_entry` | 同上 | T |
| 21 | `$index < len($data) at 'mirsa_line_entry` | `{"index":"index","data":"data"}` | T |
| 21 | `$index <= len($data) - 1 at 'mirsa_line_entry` | 同上 | T |
| 31 | `$index < len($data) at 'mirsa_line_entry` | `{"index":"index","data":"data"}` | T |
