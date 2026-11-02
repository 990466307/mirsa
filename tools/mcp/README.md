# MCP 适配层

完整文档见 [docs/mcp.md](../../docs/mcp.md)。

这里只有三样东西：

- `server.py`：MCP 服务（工具入口 `mirsa_query`）
- `requirements.txt`：Python 依赖（`mcp==2.3.0`，需要 Python ≥ 3.10）
- 安装脚本已统一到 `scripts/setup.py`（macOS / Linux 通用）

启动统一走 `bash scripts/start-mcp.sh`。
