"""Analyze MIRSA attributes already present in Rust source over MCP stdio."""
import json
import os
import re
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Annotated, Any

import anyio
from mcp.server import MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from pydantic import Field

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "target/mcp"
WORKER = RUNTIME / "mirsa-query"
QUERY_SECONDS = 90
TRACE_PATH = os.environ.get("MIRSA_TRACE")
TRACE_LOCK = threading.Lock()

class AttributeServer(MCPServer):
    async def call_tool(self, name, arguments, context=None):
        # The MCP runtime otherwise silently discards unknown function arguments.
        if name == "mirsa_query":
            unknown = set(arguments) - {"file", "code", "crate_root", "rustc_args", "lines", "verbose"}
            if unknown:
                raise ToolError("input: 不支持的参数：" + ", ".join(sorted(unknown)) + "；请在源码中使用 MIRSA 查询属性")
        return await super().call_tool(name, arguments, context)


mcp = AttributeServer(
    "MIRSA",
    instructions=(
        "MIRSA 分析 Rust 源码中已有的 #[mirsa::check(...)] 和 #[mirsa::facts(...)] 属性。"
        "先在待分析语句或表达式上写属性，再传入 file 或 code，二者必须且只能提供一个。"
        "不接受外部 DSL、行号、变量绑定或注释位置标注；源码变量自动按标注位置的作用域解析。"
        "例如 #[mirsa::check(index < len(table))]、#[mirsa::facts(table)]；属性中不需要 dollar 符号。"
        "工具不修改源文件，一次返回全部属性查询，按源码位置和分析路径分别提供结论和证据。"
        "默认返回摘要；仅在需要完整原始报告时传 verbose=true。"
        "reports 为空只表示没有启用的查询属性，不表示整个文件安全。"
        "result=true 表示所查询的性质已证明，definite_violation=true 表示模型内确定违反；"
        "其他正常状态为 maybe_violated 或 unreachable，查看逐路径 evidence 和 notes。"
        "单条性质成立不代表整个 unsafe 调用安全，候选 SMT 模型不保证对应真实执行。"
        "分析 crate 内某个文件时，用 file 指定目标文件、crate_root 指定 src/lib.rs 等编译入口。"
        "lines 限定目标文件插入属性后的行范围；返回的源码与位置都对应目标文件。"
        "当前接受单文件或可编译的完整源码；外部依赖用 rustc_args 传 --extern/-L。"
        "代码缺少依赖、编译错误或查询错误不构成安全性质结论。"
    ),
    log_level="WARNING",
)

def trace_query(source: str, label: str, line: int, dsl: str, result: dict[str, Any]) -> None:
    """Append one query record; never let tracing break a query."""
    if not TRACE_PATH:
        return
    entry = {
        "time": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "source_label": label,
        "line": line,
        "dsl": dsl,
        "source": source,
        "location": result.get("location"),
        "resolved_bindings": result.get("resolved_bindings"),
        "conclusion": result.get("conclusion"),
    }
    try:
        path = Path(TRACE_PATH)
        path.parent.mkdir(parents=True, exist_ok=True)
        with TRACE_LOCK, open(path, "a", encoding="utf-8") as handle:
            handle.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except OSError:
        pass


def worker_env() -> dict[str, str]:
    config = json.loads((RUNTIME / "runtime.json").read_text())
    libraries = [str(RUNTIME / "lib"), config["rustc_lib"]]
    # Linux 用 LD_LIBRARY_PATH，macOS 用 DYLD_LIBRARY_PATH：设置当前平台认的那个，
    # worker 才能找到 libz3 与 rustc driver 的动态库。
    var = "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
    if os.environ.get(var):
        libraries.append(os.environ[var])
    return dict(os.environ, **{var: os.pathsep.join(libraries)})


async def run_worker(request: dict[str, Any]) -> dict[str, Any]:
    """Run one request through the MIRSA worker and return its raw reply."""
    payload = json.dumps(request, ensure_ascii=False).encode()
    try:
        with tempfile.TemporaryDirectory(prefix="mirsa-query-") as directory:
            with anyio.fail_after(QUERY_SECONDS):
                result = await anyio.run_process(
                    [str(WORKER)], input=payload, cwd=directory, env=worker_env(), check=False,
                )
    except TimeoutError as error:
        raise ToolError(f"timeout: 查询超过 {QUERY_SECONDS} 秒，未得到性质结论") from error
    except OSError as error:
        raise ToolError(f"runtime: {error}") from error
    diagnostics = result.stderr.decode(errors="replace")[-12000:]
    try:
        reply = json.loads(result.stdout)
    except (ValueError, UnicodeError) as error:
        raise ToolError(
            f"runtime: 查询进程未返回有效结果（退出码 {result.returncode}）\n{diagnostics}"
        ) from error
    if reply.get("status") == "error" and reply.get("kind") == "compilation" and diagnostics.strip():
        message = reply.get("message", "源码编译失败")
        if diagnostics.strip() not in message:
            reply["message"] = f"{message}\n{diagnostics.strip()}"
    return reply


def fact_highlights(facts: dict[str, Any], limit: int = 6) -> list[str]:
    """Compact, readable values out of a `facts()` result.

    These are the concrete numbers behind a verdict: `$index = [5, 5]` against
    `Length($table) = [3, 3]` is the reason a call is out of bounds, which is a
    different question from why the analysis could not decide something.
    """
    if "paths" in facts:
        out: list[str] = []
        for path in facts.get("paths") or []:
            prefix = f"path {path.get('path_id')} (merged={path.get('merged')}): "
            out.extend(prefix + text for text in fact_highlights(path, limit))
        return out[:limit]
    out: list[str] = []
    for item in facts.get("abstract_info", []) or []:
        subject = item.get("subject", {}) or {}
        value = item.get("value", {}) or {}
        kind = subject.get("kind")
        if kind == "variable":
            label = f"${str(subject.get('name', '?')).lstrip('$')}"
        elif kind == "property":
            argument = subject.get("argument", {}) or {}
            name = f"${str(argument.get('name', '?')).lstrip('$')}"
            label = f"{subject.get('property')}({name})"
        elif kind == "allocation":
            label = str(subject.get("identity"))
        else:
            continue
        if value.get("kind") == "integer":
            out.append(f"{label} = [{value.get('low')}, {value.get('high')}]")
        elif value.get("kind") == "allocation":
            if kind == "allocation" and label.startswith(("stack(", "external(")):
                continue
            out.append(f"{label} extent = [{value.get('extent_low')}, {value.get('extent_high')}]")
        elif value.get("kind") == "bool":
            out.append(f"{label} = {value.get('value')}")
    out.sort(key=lambda text: 0 if _is_singleton(text) else 1)
    deduped: list[str] = []
    for text in out:
        if text not in deduped:
            deduped.append(text)
    return deduped[:limit]


def _is_singleton(text: str) -> bool:
    """True for `x = [5, 5]` style lines, where low == high."""
    if "= [" not in text:
        return False
    inner = text.split("= [", 1)[1].rstrip("]")
    if ", " not in inner:
        return False
    low, high = inner.split(", ", 1)
    return low == high


def _render_expr(node: Any) -> str:
    if not isinstance(node, dict):
        return str(node)
    kind = node.get("kind")
    if kind == "variable":
        return str(node.get("name", "?"))
    if kind == "constant":
        if node.get("ty") == "bool":
            return "true" if str(node.get("bits")) == "1" else "false"
        return str(node.get("bits", "?"))
    if kind == "typed":
        return _render_expr(node.get("expression"))
    if kind == "property":
        prop = str(node.get("property"))
        argument = _render_expr(node.get("argument"))
        if prop == "Length":
            return "len(" + argument + ")"
        return prop + "(" + argument + ")"
    if kind == "binary":
        op = _BINARY.get(str(node.get("operator")), str(node.get("operator")))
        return f"{_render_expr(node.get('left'))} {op} {_render_expr(node.get('right'))}"
    if kind == "unary":
        return f"{node.get('operator', '')}{_render_expr(node.get('expression'))}"
    return str(kind or "?")


def _render_fact(raw: Any) -> str:
    try:
        fact = json.loads(raw) if isinstance(raw, str) else raw
    except ValueError:
        return str(raw)[:160]
    if not isinstance(fact, dict):
        return str(raw)[:160]
    kind = fact.get("kind")
    if kind == "condition":
        return _render_expr(fact.get("expression"))
    if kind == "definition":
        return f"{_render_expr(fact.get('target'))} := {_render_expr(fact.get('expression'))}"
    if kind == "same_value":
        members = fact.get("members") or []
        return " ≡ ".join(_render_expr(m) for m in members)
    expression = fact.get("expression")
    if expression is not None:
        return _render_expr(expression)
    return ""


def _render_value(node: Any) -> str:
    if not isinstance(node, dict):
        return str(node)
    kind = node.get("kind")
    if kind == "bool":
        if node.get("value") is None:
            return "{true, false}"
        return "true" if node.get("value") else "false"
    if kind == "integer":
        return f"[{node.get('low')}, {node.get('high')}]"
    return str(kind or "?")


def _render_eval(
    node: Any,
    depth: int = 0,
    out: list[str] | None = None,
    parent_expression: str | None = None,
) -> list[str]:
    """Flatten a reasoning tree, collapsing a node that repeats its parent."""
    out = [] if out is None else out
    if not isinstance(node, dict) or depth > 4 or len(out) >= 6:
        return out
    expression = _render_expr(node.get("expression"))
    value = _render_value(node.get("value"))
    same = expression == parent_expression
    if not same:
        out.append("  " * depth + f"{expression} = {value}")
    for premise in node.get("premises") or []:
        _render_eval(premise, depth if same else depth + 1, out, expression)
        if len(out) >= 6:
            break
    return out


def _clean_smt_assumption(text: str) -> bool:
    """Drop tautologies such as `(index == index)` or `(_3 >= true)`."""
    body = text.strip().strip("()")
    if "==" in body:
        left, right = (part.strip() for part in body.split("==", 1))
        if left == right:
            return False
    if body.endswith(">= true") or body.endswith("<= false"):
        return False
    # 整数类型的“不超过最大值”是恒真上界，不是证据
    if re.search(r"<=\s*(1844674407370955161[45]|9223372036854775807|"
                 r"429496729[45]|2147483647|6553[45]|32767|255|127)", body):
        return False
    return True


def summarize_evidence(evidence: Any) -> dict[str, Any]:
    """Render one path's evidence with the analyzer's source-level expression view."""
    if not isinstance(evidence, dict):
        return {}
    out: dict[str, Any] = {}
    notes: list[str] = []

    view = evidence.get("expressions")
    if isinstance(view, dict):
        kind = view.get("kind")
        goal = str(view.get("goal", ""))
        if kind:
            out["method"] = str(kind)
        if kind == "abstract":
            notes.extend(str(fact) for fact in view.get("facts") or [])
        elif kind == "smt":
            negative = view.get("negative")
            positive = view.get("positive")
            notes.append(f"¬P = {negative}, P = {positive}")
            seen: set[str] = set()
            for assumption in view.get("assumptions") or []:
                text = str(assumption)
                if not _clean_smt_assumption(text) or text in seen:
                    continue
                seen.add(text)
                notes.append(text)
                if len(notes) >= 6:
                    break
            out["smt"] = {
                "negative": negative,
                "positive": positive,
                "goal": goal,
            }

    if not notes:
        # 旧格式兜底：推理树或 facts 列表
        smt = evidence.get("smt")
        if isinstance(smt, dict):
            checks = smt.get("checks") if isinstance(smt.get("checks"), dict) else smt
            negative = checks.get("negative")
            positive = checks.get("positive")
            if negative is not None:
                notes.append(
                    f"SMT: ¬P = {negative}"
                    + (f", P = {positive}" if positive is not None else "")
                )
        context = evidence.get("context")
        tree = evidence.get("abstract_evaluation")
        if tree and not notes:
            notes = _render_eval(tree)
        elif isinstance(context, list):
            notes.extend(_render_fact(item) for item in context[:4])

    deduped: list[str] = []
    for note in notes:
        if note and note not in deduped:
            deduped.append(note)
    out["notes"] = deduped[:6]
    return out


def summarize(output: dict[str, Any] | None) -> dict[str, Any]:
    """Lift the verdict details agents need, instead of leaving them in paths[0].

    `mirsa-query` returns a rich check (verdict, definite_violation, unreachable,
    status, paths). Older builds collapsed it to a bare boolean, in which case a
    `false` says nothing about whether the property was refuted or may be violated.

    Only data goes here. How to read it is stated once in the server instructions,
    because anything repeated per result accumulates in the conversation history.
    """
    if not isinstance(output, dict) or output.get("kind") != "check":
        return {"kind": (output or {}).get("kind"), "result": None}
    result = output.get("result")
    if isinstance(result, bool):
        return {
            "kind": "check", "result": result, "proved": result,
            "definite_violation": None, "unreachable": None,
            "status": "proved" if result else "maybe_violated",
        }
    if not isinstance(result, dict):
        return {"kind": "check", "result": result, "proved": bool(result)}
    verdict = result.get("verdict")
    paths = result.get("paths") or []
    counterexamples = [p["candidate"] for p in paths if p.get("candidate")]
    summary = {
        "kind": "check",
        "result": result.get("result"),
        "verdict": verdict,
        "proved": verdict == "T",
        "definite_violation": bool(result.get("definite_violation", False)),
        "unreachable": bool(result.get("unreachable", False)),
        "paths": len(paths),
        "path_verdicts": [
            {
                "path_id": p.get("path_id"),
                "verdict": p.get("verdict"),
                "definite_violation": p.get("definite_violation"),
                "status": p.get("status") or (
                    "violated" if p.get("definite_violation") else
                    "unreachable" if p.get("unreachable") else
                    "proved" if p.get("verdict") == "T" else "maybe_violated"
                ),
                "source": p.get("source"),
                **({"notes": p["notes"]} if p.get("notes") else {}),
                **(
                    {"evidence": summarize_evidence(p.get("evidence"))}
                    if p.get("evidence")
                    else {}
                ),
            }
            for p in paths
        ],
    }
    if counterexamples:
        summary["unproven_witness"] = counterexamples

    # 正常结论由逐路径 evidence 解释，补充求解诊断只作为 notes 输出。
    if summary["definite_violation"]:
        summary["status"] = "violated"
    elif summary["unreachable"]:
        summary["status"] = "unreachable"
    elif summary["proved"]:
        summary["status"] = "proved"
    else:
        summary["status"] = "maybe_violated"
    notes = list(dict.fromkeys(note for p in paths for note in p.get("notes", [])))
    if notes:
        summary["notes"] = notes
    return summary


def format_error(reply: dict[str, Any]) -> str:
    """Message plus any diagnostic detail the message itself does not carry."""
    message = str(reply.get("message", "") or "").strip()
    lines = [message] if message else []
    for diagnostic in reply.get("diagnostics") or []:
        if not isinstance(diagnostic, dict):
            continue
        text = str(diagnostic.get("message", "") or "").strip()
        hint = str(diagnostic.get("hint", "") or "").strip()
        if text and text not in message:
            lines.append(text)
        if hint and hint not in message:
            lines.append("hint: " + hint)
    return "\n".join(lines) or "查询失败"


def to_local_path(raw: str) -> Path:
    """把工具参数里的路径规整成本机绝对路径。

    macOS 上只认本机路径：去掉多余的引号/空白、展开 `~`；相对路径按
    server 进程的 cwd 解析。不再做 Windows 盘符或 WSL 路径的翻译。
    """
    text = raw.strip().strip('"').strip("'")
    path = Path(text).expanduser()
    return path if path.is_absolute() else path.resolve()


def parse_line_range(text: str | None) -> tuple[int, int] | None:
    """Parse "10-20" / "12" into an inclusive range; None keeps every line."""
    if text is None or not str(text).strip():
        return None
    matched = re.fullmatch(r"\s*(\d+)\s*(?:-\s*(\d+)\s*)?", str(text))
    if matched is None:
        raise ToolError(f'input: lines 格式应为 "10-20" 或 "12"，收到 {text!r}')
    start = int(matched.group(1))
    end = int(matched.group(2) or start)
    if start < 1 or end < start:
        raise ToolError(f"input: lines 范围无效：{text!r}")
    return start, end


def resolve_source(
    file: str | None, code: str | None
) -> tuple[str, str | None, str]:
    """Pick exactly one of a file path or inline code; return source, file arg, label."""
    if (file is None) == (code is None):
        raise ToolError("input: file 和 code 必须且只能给一个")
    if code is not None:
        if not code.strip():
            raise ToolError("input: code 不能为空")
        return code, None, "<pasted code>"
    path = to_local_path(file)
    if not path.is_file():
        raise ToolError(f"input: 找不到这个文件：{file}（解析为 {path}）；请给本机绝对路径")
    if path.suffix != ".rs":
        raise ToolError("input: 请传入 .rs 源文件；当前不直接驱动 Cargo 项目")
    return path.read_text(encoding="utf-8", errors="replace"), str(path.resolve()), str(path)


@mcp.tool()
async def mirsa_query(
    file: Annotated[str | None, Field(description="已包含 MIRSA 属性的 .rs 文件路径；与 code 二选一")] = None,
    code: Annotated[str | None, Field(description="已包含 #[mirsa::check(...)] 或 #[mirsa::facts(...)] 的完整 Rust 源码；与 file 二选一")] = None,
    crate_root: Annotated[
        str | None,
        Field(description="可选 crate 入口 .rs 路径（如 src/lib.rs）；与 file 配合，从入口编译整个 crate，只返回 file 中的查询"),
    ] = None,
    rustc_args: Annotated[list[str] | None, Field(description="可选 rustc 参数，依赖路径使用绝对路径")] = None,
    lines: Annotated[
        str | None,
        Field(description='只返回 file 中该行范围内的结论，行号以插入属性后的源码为准，例如 "10-20" 或 "12"；默认返回全部'),
    ] = None,
    verbose: Annotated[bool, Field(description="是否附加完整原始报告，默认仅返回摘要")] = False,
) -> dict[str, Any]:
    """分析源码中全部 MIRSA 查询属性，返回每条属性的位置、结论和逐路径证据。

    crate_root 指定编译入口，file 指定目标文件；lines 使用标注后的行号。
    """
    source, file_arg, label = resolve_source(file, code)
    window = parse_line_range(lines)
    compilation_file = file_arg
    if crate_root is not None:
        if file_arg is None:
            raise ToolError("input: crate_root 必须与 file 配合使用，不能与 code 配合")
        _, compilation_file, _ = resolve_source(crate_root, None)
    selected_file = (
        Path(file_arg)
        if file_arg is not None and (crate_root is not None or window is not None)
        else None
    )
    request: dict[str, Any] = {"rustc_args": rustc_args or []}
    if file_arg is None:
        request["source"] = source
    else:
        request["file"] = compilation_file
    reply = await run_worker(request)
    if reply.get("status") != "ok":
        raise ToolError(f"{reply.get('kind', 'runtime')}: {format_error(reply)}")
    reports: list[dict[str, Any]] = []
    for item in reply.get("reports", []):
        if selected_file is not None:
            report_file = item.get("file")
            if not isinstance(report_file, str) or Path(report_file).resolve() != selected_file:
                continue
        if window is not None:
            line = item.get("line")
            if not isinstance(line, int) or not (window[0] <= line <= window[1]):
                continue
        output = (item.get("report") or {}).get("output") or {}
        entry: dict[str, Any] = {
            "location": {"source": item.get("file", label), "line": item["line"], "column": item["column"]},
            "query": item["query"],
        }
        if verbose:
            entry["report"] = item["report"]
        if output.get("kind") == "check":
            entry["conclusion"] = summarize(output)
        elif output.get("kind") == "facts":
            entry["facts"] = output.get("result")
        reports.append(entry)
        trace_query(source=source, label=label, line=item["line"], dsl=item["query"], result=entry)
    result: dict[str, Any] = {
        "source_label": label,
        "source": source[:40000],
        "reports": reports,
    }
    if lines is not None:
        result["lines"] = lines
    return result


if __name__ == "__main__":
    mcp.run(transport="stdio")
