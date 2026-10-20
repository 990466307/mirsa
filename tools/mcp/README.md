# Agent / MCP 接入

这是 MIRSA 的可选 Agent 适配层，通过 stdio MCP 暴露查询能力。Agent 提出要检查的性质，MIRSA 返回分析结果。用户可以给文件或完整的 Rust 代码片段；MIRSA 服务本身不需要模型 API key。使用 Rust / MIR 接口不需要安装此适配层。

## 安装与注册

先按根目录 [README](../../README.md) 安装 Rust、Z3 和 Clang。此适配层另需 Python 及虚拟环境支持；Ubuntu / Debian 可运行 `sudo apt-get install -y python3-venv`。然后在仓库根目录运行：

```bash
python3 tools/mcp/setup.py
bash scripts/start-mcp.sh
```

安装脚本构建带 Z3 的查询程序，并在 `target/mcp` 建立 Python 环境。服务启动后等待 MCP 请求，不显示交互提示符。Agent 的 MCP 配置使用：

```json
{
  "mcpServers": {
    "mirsa": {
      "command": "/bin/bash",
      "args": ["/你的绝对路径/mirsa/scripts/start-mcp.sh"]
    }
  }
}
```

这是常见的 stdio 配置样式；各宿主的配置外层可能不同。[Windows DSH 配置](DSH接入.md) 通过 `wsl.exe` 转接同一服务。

## 三个工具

| 工具 | 输入 | 返回 |
|---|---|---|
| `mirsa_read_lines` | `file`、`start`，可选 `count` | 带行号的源码 |
| `mirsa_variables` | `file` 或 `code`、`line`，可选 `candidates` | 候选中能解析的变量 `visible` 和未解析的变量 `not_visible` |
| `mirsa_query` | `file` 或 `code`、`line`、`dsl` | 性质结论 `conclusion`，或 `facts` 信息 |

`file` 和 `code` 必须二选一。`line` 从 1 开始，查目标语句执行前的状态；先用独占一行的语句，避免把一行中的多次操作混为一个位置。工具只在内存中添加定位标注，不修改原文件。

例如直接调用 `mirsa_query`：

```json
{
  "code": "pub fn example(s: &[u8], i: usize) -> u8 {\n    if i < s.len() {\n        unsafe { *s.get_unchecked(i) }\n    } else { 0 }\n}\n",
  "line": 3,
  "dsl": "$i < len($s) at 'mirsa_line_entry"
}
```

应得到 `conclusion.proved=true`、`conclusion.result=true`。改查 `facts($s) at 'mirsa_line_entry`，则得到 `facts.abstract_info` 和 `facts.symbolic_info`。两类结果均附查询位置及实际变量绑定。

DSL 位置固定写 `at 'mirsa_line_entry`，只有开头一个单引号，末尾不加分号。默认 `$i` 对应源码 `i`；不同名时传 `bindings: {"index": "i"}`。文件输入可用 Linux / WSL 绝对路径，WSL 服务也支持转换 Windows 盘符路径。外部依赖通过 `mirsa_query` 的可选 `rustc_args` 提供。

## 结果怎么读

| 字段 | 含义 |
|---|---|
| `conclusion.proved=true` | 指定位置的这条性质已证明 |
| `conclusion.definite_violation=true` | 当前分析模型内存在必然违反性质的路径 |
| `conclusion.unreachable=true` | 查询点不可达 |
| 以上均为 false | 未能判定，查看 `reason` |
| MCP 工具错误 | 编译、绑定、查询或运行失败，不是性质结论 |

`result=false` 不能单独判漏洞。`evidence` 是相关变量的抽象事实；`unproven_witness` 是求解器模型，均不能自动当作真实执行反例。检查一条边界或非空性质，也不能宣称整个 unsafe 调用安全。

当前入口要求能编译的单文件源码；类型、依赖或上下文不完整的片段会编译失败。函数参数没有隐含的调用方前提，跨函数、循环及不支持的库语义仍可能导致未证明。

## 验证

```bash
target/mcp/venv/bin/python tools/mcp/test_mcp.py
```

测试包括文件、代码片段、错误处理及取消；Windows/WSL 转接项需要相应环境。MIRSA 代码修改后重新运行 `setup.py`。非系统安装的原生依赖可用 `--z3-prefix /依赖前缀` 指定，前缀须包含 `include/z3.h` 和库目录。
