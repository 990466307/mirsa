#!/usr/bin/env bash
set -euo pipefail
mirsa_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ ! -x "$mirsa_root/target/mcp/venv/bin/python" || ! -x "$mirsa_root/target/mcp/mirsa-query" || ! -f "$mirsa_root/target/mcp/runtime.json" ]]; then
    echo "MIRSA MCP 尚未安装，请先运行 tools/mcp/setup.py。" >&2
    exit 1
fi
cd -- "$mirsa_root"
exec "$mirsa_root/target/mcp/venv/bin/python" "$mirsa_root/tools/mcp/server.py"
