# MCP / Agent 接入

MIRSA 通过 stdio MCP 暴露一个工具 `mirsa_query`：分析源码里的
`#[mirsa::check(...)]` / `#[mirsa::facts(...)]` 属性，返回每条属性的结论与证据。
服务本身不需要模型 API key；不用 Agent 时也不需要这一层。

## 安装

```bash
# 仓库根目录
python3 scripts/setup.py               # macOS 与 Linux 通用
bash scripts/start-mcp.sh
```

脚本会编译 `mirsa-query`、在 `target/mcp` 建 Python venv 并部署运行时；
依赖 Rust nightly（`rustc-dev`、`rust-src`）、Z3 和 libclang。

## 注册到宿主

```json
{
  "mcpServers": {
    "mirsa": {
      "command": "/bin/bash",
      "args": ["/绝对路径/mirsa/scripts/start-mcp.sh"]
    }
  }
}
```

macOS 上 `start-mcp.sh` 会自行设置 `DYLD_LIBRARY_PATH`，不需要额外环境变量。

## 工具

```python
mirsa_query(file="src/lib.rs")                 # 分析文件里全部注解
mirsa_query(file="src/buffer.rs", lines="10-20")  # 只看某个行范围
mirsa_query(file="src/identifier.rs", crate_root="src/lib.rs", lines="122",
            rustc_args=["--edition=2018", "--cfg", 'feature="std"'])  # crate 子模块
mirsa_query(code="<完整 Rust 源码>")             # 直接传源码
mirsa_query(file="src/lib.rs", verbose=True)   # 附完整原始报告
```

- `file` 与 `code` 二选一；传 crate 入口（如 `src/lib.rs`）会编译整个 crate，
  一次返回所有模块的注解结果。
- 指定 `crate_root` 时，`file` 是目标源码文件，`crate_root` 是编译入口；只返回目标文件的查询，卡片源码也来自该文件。`crate_root` 不能与 `code` 一起使用。
- `lines` 按目标文件插入属性后的行号过滤，不会混入其他模块相同行号的结果。
- 返回 `reports[]`：`location`、`query`、`conclusion.status`
  （`proved` / `violated` / `maybe_violated` / `unreachable`）、逐路径 `evidence`。
- 出错时 `kind` 为 `compilation` / `binding` / `query` / `input`，
  并带上分析器的诊断（位置、caret、建议）。

## DSH 插件

`dsh-plugin/` 是 DSH 的插件包（npm 名 `dsh-mirsa`），负责把
`mirsa_query` 的结果画到右侧栏源码行上。安装：

```bash
pnpm add dsh-mirsa
npx dsh-mirsa setup      # 自动准备 MIRSA（源码、工具链、z3、Python）
```

插件按 `MIRSA_ROOT` 找 MIRSA 仓库，默认 `~/mirsa`。

## 历史

Windows/WSL 的旧接入方式已移除；macOS 移植记录见
[mac-port-handoff.md](mac-port-handoff.md)。
