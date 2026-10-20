"""Expose the existing MIRSA query API over MCP stdio, shaped for agent use."""

import json
import os
from pathlib import Path
import re
import tempfile
from typing import Annotated, Any

import anyio
from mcp.server import MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from pydantic import Field

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "target/mcp"
WORKER = RUNTIME / "mirsa-query"
QUERY_SECONDS = 90
AUTO_MARKER = "mirsa_line_entry"

PROBE_FORMS = ("${n} == ${n}", "${n} != 0", "len(${n}) >= 0", "is_null(${n})")

BINDING_PATTERN = re.compile(r"\b(?:let|mut)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)")
RUST_KEYWORDS = frozenset({
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "async", "await", "box", "macro", "union",
})

PARAM_PATTERN = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)\s*:")
FN_PATTERN = re.compile(r"\bfn\s+[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^>]*>)?\s*\(")

mcp = MCPServer(
    "MIRSA",
    instructions=(
        "MIRSA 分析 Rust 代码中的指定性质，供 Agent 核实自己的判断。"
        "用户给文件时先 mirsa_read_lines 找行号；给代码时直接使用同一份 code。"
        "可用 mirsa_variables 核实该位置的变量名，再用 mirsa_query 验证性质。"
        "file 与 code 二选一；line 从 1 开始，查询目标语句执行前的状态。"
        "代码须能独立编译；缺少依赖、类型或调用前提时，应说明缺失的信息，不能自行假定。"
        "工具在内存中加定位标注，不修改原文件。"
        "DSL 例如 $i < len($s) at 'mirsa_line_entry 或 facts($s) at 'mirsa_line_entry。"
        "位置只有开头一个单引号，末尾不加分号。bindings 可省略，默认源码与 DSL 同名；"
        "不同名时传映射，例如 {\"index\": \"i\"}。"
        "支持数值、逻辑、位运算、类型转换以及 len/is_null/is_live/offset/extent 等查询；"
        "完整文法见仓库 crates/dsl/grammar.ebnf。facts 只接受一个参数，返回抽象信息和符号关系。"
        "Agent 负责选择要验证的性质：例如切片下标检查 $i < len($s)，"
        "NonZero 构造检查 $x != 0。每条查询只验证该性质；"
        "指针非空不能替代对象有效性、对齐、边界、生命周期等条件，复制还涉及源、目标及重叠要求。"
        "conclusion.proved=true 表示该性质在当前模型和输入前提下已证明；"
        "definite_violation=true 表示模型内存在必然违反性质的路径；"
        "unreachable=true 表示查询点不可达。其他情况是未证明，查看 reason。"
        "result=false 不能单独判漏洞；reason 解释分析为何未证明，并不描述代码缺陷。"
        "evidence 是抽象事实，unproven_witness 是求解器候选模型，均不能自动当成真实执行反例。"
        "facts 是合并信息，不能将不同变量的范围任意组合成一次真实执行。"
        "当前按目标函数分析，不会自动把某个调用方的具体实参当作函数入口前提。"
        "未知库语义、路径合并和循环可能损失精度；不要把一条性质证明扩展为整个调用或文件安全。"
        "汇报实际验证的位置、性质、结论和未覆盖的前提。"
        "编译、绑定、查询、进程错误及超时不构成性质结论。"
        "当前接受 .rs 或完整源码，不直接接受 Cargo.toml；"
        "外部依赖用 rustc_args 传 --extern/-L，路径使用 WSL 绝对路径。"
    ),
    log_level="WARNING",
)


def worker_env() -> dict[str, str]:
    config = json.loads((RUNTIME / "runtime.json").read_text())
    libraries = [str(RUNTIME / "lib"), config["rustc_lib"]]
    if os.environ.get("LD_LIBRARY_PATH"):
        libraries.append(os.environ["LD_LIBRARY_PATH"])
    return dict(os.environ, LD_LIBRARY_PATH=os.pathsep.join(libraries))


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
        return json.loads(result.stdout)
    except (ValueError, UnicodeError) as error:
        raise ToolError(
            f"runtime: 查询进程未返回有效结果（退出码 {result.returncode}）\n{diagnostics}"
        ) from error

def line_offsets(source: str) -> list[int]:
    """Byte offsets at which each 1-based line starts, plus a final sentinel."""
    offsets = [0]
    for index, char in enumerate(source):
        if char == "\n":
            offsets.append(index + 1)
    return offsets


def find_anchor(lines: list[str], line: int) -> int:
    """Pick the line after which the marker must sit to annotate `line`.

    MIRSA requires a marker to be immediately followed by the statement it
    annotates. A statement often spans several lines (or starts after a blank
    line), so the marker cannot simply go directly above `line`: it must go after
    the previous non-blank, non-comment line. Otherwise MIRSA sees a blank line
    between marker and statement and reports the point as unreachable.
    """
    index = line - 2
    while index >= 0:
        text = lines[index].strip()
        if text and not text.startswith("//"):
            return index + 2
        index -= 1
    return line


def mark_statement(source: str, line: int) -> tuple[str, int]:
    """Insert `// mirsa: <AUTO_MARKER>` so it annotates 1-based `line`.

    Returns the rewritten source and the 1-based line the marker precedes.
    MIRSA rejects two markers with the same name, so an existing marker with our
    name is reused (in that case the caller must have run against the annotated
    source already).
    """
    for text in source.splitlines():
        if text.strip().startswith(f"// mirsa: {AUTO_MARKER}"):
            return source, line
    lines = source.splitlines()
    if not 1 <= line <= len(lines):
        raise ToolError(f"input: line={line} 超出文件范围（该文件共 {len(lines)} 行）")
    anchor = find_anchor(lines, line)
    offsets = line_offsets(source)
    start = offsets[anchor - 1]
    end = source.find("\n", start)
    text = source[start:] if end < 0 else source[start:end]
    indent = text[: len(text) - len(text.lstrip())]
    return f"{source[:start]}{indent}// mirsa: {AUTO_MARKER}\n{source[start:]}", anchor


def source_candidates(source: str, line: int) -> list[str]:
    """Names that might be in scope at `line`, innermost first (a hint only).

    Two passes, because they need different scoping rules:

    * local bindings are collected by walking backwards while tracking brace
      nesting - a `}` means that block has already closed before the query point,
      so declarations inside it are out of scope;
    * function parameters are scoped to the whole function, so every enclosing
      `fn` signature is collected regardless of depth (an open signature on the
      preceding line has no `}` yet, which is exactly the enclosing function).

    This is a source-level guess, not MIRSA's binding decision - `mirsa_variables`
    probes the real answer, and a wrong guess is not evidence about any property.
    """
    lines = source.splitlines()
    if not 1 <= line <= len(lines):
        return []
    seen: list[str] = []

    def add(name: str) -> None:
        if name not in seen and name not in RUST_KEYWORDS:
            seen.append(name)

    for offset in range(line - 1, -1, -1):
        if FN_PATTERN.search(lines[offset]):
            for name in PARAM_PATTERN.findall(lines[offset]):
                add(name)

    depth = 0
    for offset in range(line - 1, -1, -1):
        text = lines[offset]
        depth += text.count("}") - text.count("{")
        if depth > 0:
            continue
        for name in BINDING_PATTERN.findall(text):
            add(name)
    return seen


def dsl_names(dsl: str) -> list[str]:
    """Variable names referenced by a DSL expression, in order of appearance."""
    names: list[str] = []
    for name in re.findall(r"\$([A-Za-z_][A-Za-z0-9_]*)", dsl):
        if name not in names:
            names.append(name)
    return names


def identity_bindings(
    dsl: str, bindings: dict[str, str] | None
) -> tuple[dict[str, str], list[str]]:
    """Default each DSL variable to a same-named source variable.

    Returns the effective mapping plus the DSL names that were defaulted.
    """
    effective = dict(bindings or {})
    defaulted = [name for name in dsl_names(dsl) if name not in effective]
    for name in defaulted:
        effective[name] = name
    return effective, defaulted


async def probe_variables(
    file: str | None, source: str, names: list[str]
) -> dict[str, list[str]]:
    """Split `names` into those MIRSA can resolve at the marker and those it cannot."""
    visible: list[str] = []
    unresolved: list[str] = []
    for name in names:
        found = False
        for form in PROBE_FORMS:
            dsl = form.replace("{n}", name) + f" at '{AUTO_MARKER}"
            reply = await run_worker(
                {
                    "file": file, "source": source, "dsl": dsl,
                    "bindings": {name: name}, "rustc_args": [],
                }
            )
            if reply.get("status") == "ok":
                found = True
                break
            message = reply.get("message", "")
            if reply.get("kind") == "compilation":
                continue
            if reply.get("kind") == "binding" and name in message:
                break
            found = True
            break
        (visible if found else unresolved).append(name)
    return {"visible": visible, "not_visible": unresolved}


def fact_highlights(facts: dict[str, Any], limit: int = 6) -> list[str]:
    """Compact, readable values out of a `facts()` result.

    These are the concrete numbers behind a verdict: `$index = [5, 5]` against
    `Length($table) = [3, 3]` is the reason a call is out of bounds, which is a
    different question from why the analysis could not decide something.
    """
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


async def gather_evidence(
    file: str, source: str, dsl: str, bindings: dict[str, str], rustc_args: list[str]
) -> dict[str, list[str]]:
    """Ask `facts()` for every variable the property mentions.

    The variable's own numeric range and the slice's length are what make a
    definite violation explicable; the solver's `reason` cannot supply that.
    """
    evidence: dict[str, list[str]] = {}
    for name in dsl_names(dsl):
        reply = await run_worker(
            {
                "file": file, "source": source, "dsl": f"facts(${name}) at '{AUTO_MARKER}",
                "bindings": bindings, "rustc_args": rustc_args,
            }
        )
        if reply.get("status") != "ok":
            continue
        highlights = fact_highlights((reply["report"].get("output") or {}).get("result") or {})
        if highlights:
            evidence[name] = highlights
    return evidence


def summarize(output: dict[str, Any] | None) -> dict[str, Any]:
    """Lift the verdict details agents need, instead of leaving them in paths[0].

    `mirsa-query` returns a rich check (verdict, definite_violation, unreachable,
    reason, paths). Older builds collapsed it to a bare boolean, in which case a
    `false` says nothing about whether the property was refuted or merely unknown.

    Only data goes here. How to read it is stated once in the server instructions,
    because anything repeated per result accumulates in the conversation history.
    """
    if not isinstance(output, dict) or output.get("kind") != "check":
        return {"kind": (output or {}).get("kind"), "result": None}
    result = output.get("result")
    if isinstance(result, bool):
        return {
            "kind": "check", "result": result, "proved": result,
            "definite_violation": None, "unreachable": None, "reason": None,
        }
    if not isinstance(result, dict):
        return {"kind": "check", "result": result, "proved": bool(result)}
    verdict = result.get("verdict")
    paths = result.get("paths") or []
    reason = result.get("reason")
    counterexamples = [p["candidate"] for p in paths if p.get("candidate")]
    summary = {
        "kind": "check",
        "result": result.get("result"),
        "verdict": verdict,
        "proved": verdict == "T",
        "definite_violation": bool(result.get("definite_violation", False)),
        "unreachable": bool(result.get("unreachable", False)),
        "reason": reason,
        "paths": len(paths),
        "path_verdicts": [
            {"path_id": p.get("path_id"), "verdict": p.get("verdict"),
             "definite_violation": p.get("definite_violation")}
            for p in paths
        ],
    }
    if counterexamples:
        summary["unproven_witness"] = counterexamples
    return summary

@mcp.tool()
async def mirsa_variables(
    line: Annotated[int, Field(ge=1, description="要查看的语句所在行号（1 起）")],
    file: Annotated[
        str | None,
        Field(description="Rust 源文件路径：WSL（/home/... 、/mnt/d/...）或 Windows（D:\\\\dir\\\\f.rs）都行；与 code 二选一"),
    ] = None,
    code: Annotated[
        str | None, Field(description="直接粘贴的 Rust 代码全文；与 file 二选一")
    ] = None,
    candidates: Annotated[
        list[str] | None,
        Field(description="可选：想核实的源码变量名；不传则核实按作用域扫描到的名字"),
    ] = None,
) -> dict[str, Any]:
    """列出某一行可见的源码变量名 —— **不确定变量名时先调这个，不要靠猜**。

    文件和粘贴的代码都支持（file 与 code 二选一）：
    绑定时传 file 就能核实该文件的变量，贴代码时把同一份 code 传进来即可。
    绑定失败（报“找不到可见的变量 X”）只说明该名字在此处不可见，不说明任何性质结论。
    返回 visible（MIRSA 确认在该点可解析）和 not_visible。
    not_visible 只涵盖你给出的候选；没列进来的名字也可能存在，把它们放进 candidates 再核实一次。
    这个工具只回答“哪些名字能用”，不判断任何性质。
    """
    source, file_arg, source_label = resolve_source(file, code)
    annotated, _ = mark_statement(source, line)
    names = list(dict.fromkeys(candidates or source_candidates(source, line)))
    lines = source.splitlines()
    source_line = lines[line - 1] if line - 1 < len(lines) else ""
    if not names:
        return {
            "line": line, "source": source_label, "source_line": source_line,
            "visible": [], "not_visible": [],
            "hint": "该行之前没有扫描到名字；请用 candidates 直接给出候选变量名。",
        }
    probed = await probe_variables(file_arg, annotated, names)
    return {
        "line": line,
        "source": source_label,
        "source_line": source_line,
        **probed,
    }


async def run_check(
    source: str,
    source_label: str,
    file: str | None,
    line: int,
    dsl: str,
    bindings: dict[str, str] | None,
    rustc_args: list[str] | None,
    tool_hint: str,
) -> dict[str, Any]:
    """Shared body of the line-based checks.

    `file` names the on-disk file when there is one; it is `None` for a pasted
    snippet, in which case the worker compiles the in-memory source alone and the
    label is only used in messages.
    """
    if AUTO_MARKER not in dsl:
        raise ToolError(
            f"query: dsl 的标注名必须用 '{AUTO_MARKER}'，例如 "
            f"`$index < len($slice) at '{AUTO_MARKER}`（末尾不加引号），收到的是 {dsl!r}"
        )
    annotated, _ = mark_statement(source, line)
    effective, defaulted = identity_bindings(dsl, bindings)
    request: dict[str, Any] = {
        "source": annotated, "dsl": dsl,
        "bindings": effective, "rustc_args": rustc_args or [],
    }
    if file is not None:
        request["file"] = file
    reply = await run_worker(request)
    if reply.get("status") != "ok":
        defaulted_note = (
            f"（{', '.join(defaulted)} 是未显式指定、按同名默认补上的）" if defaulted else ""
        )
        message = reply.get("message", "")
        if reply.get("kind") == "binding" and "找不到源码标注" in message:
            raise ToolError(
                f"tool-error: 工具未能在第 {line} 行附近放置标注，因此这条语句没有被分析"
                f"（不是你的调用错误，也没有任何性质结论）。\n"
                f"常见原因：该行是注释、空行或文件级属性，或者它和上一条语句之间没有可插入"
                f"标注的位置。请改查同一条语句的首行，或改用 mirsa_query 在源码里显式标注。\n"
                f"原始错误：{message}"
            )
        raise ToolError(
            f"{reply.get('kind')}: {message}\n"
            f"本次使用的 bindings：{json.dumps(effective, ensure_ascii=False)}{defaulted_note}\n"
            f"提示：{tool_hint}"
        )
    lines = source.splitlines()
    output = reply["report"].get("output") or {}
    conclusion = summarize(output)
    statement = lines[line - 1] if line - 1 < len(lines) else ""
    location = {
        "source": source_label, "line": line,
        "source_line": statement, "annotated_statement": statement,
    }
    if conclusion.get("kind") == "facts":
        return {
            "facts": output.get("result"),
            "location": location,
            "resolved_bindings": effective,
        }
    if conclusion.get("definite_violation"):
        evidence = await gather_evidence(
            file, annotated, dsl, effective, rustc_args or []
        )
        if evidence:
            conclusion["evidence"] = evidence
    return {
        "conclusion": conclusion,
        "location": location,
        "resolved_bindings": effective,
    }


def to_wsl_path(raw: str) -> Path:
    """Accept a Windows path as well as a WSL one.

    The worker runs inside WSL, so it needs `/mnt/<drive>/...` style paths, but
    users and agents naturally write `D:\\dir\\file.rs` or `\\\\wsl$\\Distro\\...`.
    Translate both instead of rejecting them.
    """
    text = raw.strip().strip('"').strip("'")
    lowered = text.lower()
    for prefix in ("\\\\wsl$\\", "\\\\wsl.localhost\\", "//wsl$/", "//wsl.localhost/"):
        if lowered.startswith(prefix.lower()):
            rest = text[len(prefix):]
            head, _, tail = rest.partition("\\" if "\\" in rest else "/")
            return Path("/" + tail.replace("\\", "/"))
    if len(text) >= 3 and text[1] == ":" and text[2] in ("\\", "/"):
        drive = text[0].lower()
        return Path(f"/mnt/{drive}/" + text[3:].replace("\\", "/"))
    return Path(text)


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
    path = to_wsl_path(file)
    if not path.is_absolute() or not path.is_file():
        raise ToolError(
            f"input: 找不到这个文件：{file}"
            f"（按 WSL 路径解析为 {path}）。WSL 路径如 /home/... ，"
            f"Windows 路径如 D:\\\\dir\\\\f.rs 也可以，会自动转成 /mnt/d/dir/f.rs"
        )
    if path.suffix != ".rs":
        raise ToolError("input: 请传入 .rs 源文件；当前不直接驱动 Cargo 项目")
    return path.read_text(encoding="utf-8", errors="replace"), str(path.resolve()), str(path)


@mcp.tool()
async def mirsa_query(
    line: Annotated[
        int,
        Field(
            ge=1,
            description=(
                "要查询的语句的行号（1 起）；查的是它执行前的状态。"
                "传 file 时按该文件行号算，传 code 时从粘贴内容第一行算。"
            ),
        ),
    ],
    dsl: Annotated[
        str,
        Field(max_length=65536, description="性质或 facts 查询，末尾不加分号；标注写 at 'mirsa_line_entry（只有一个引号）"),
    ],
    file: Annotated[
        str | None,
        Field(description="Rust 源文件路径：WSL（/home/... 、/mnt/d/...）或 Windows（D:\\\\dir\\\\f.rs）都行；与 code 二选一"),
    ] = None,
    code: Annotated[
        str | None, Field(description="直接粘贴的 Rust 源码全文；与 file 二选一")
    ] = None,
    bindings: Annotated[
        dict[str, str] | None,
        Field(description='DSL 名到源码变量名；省略则默认同名（$index -> index）。拿不准时先调 mirsa_variables'),
    ] = None,
    rustc_args: Annotated[list[str] | None, Field(description="可选 rustc 编译参数；依赖路径用绝对路径")] = None,
) -> dict[str, Any]:
    """查某个 unsafe 调用点（或任意语句）在**执行前**的性质是否成立。

    文件和粘贴的代码都支持（file 与 code 二选一），两者行为完全一致：
    代码在内存里编译，不落盘、不改原文件，也不需要预先写标注。

    不确定该查什么性质时，用下面这些：
    `get_unchecked`/`get_unchecked_mut` 查 `$i < len($s)`；
    `split_at_unchecked` 查 `$mid <= len($s)`；`NonZero::new_unchecked` 查 `$v != 0`；
    `from_raw_parts`/`from_ptr` 可先查 `!is_null($p)`，这只覆盖非空前提；
    `copy_nonoverlapping` 可先查 `$count <= len($src)`，还须分别检查目标空间、对齐和不重叠等前提。

    line 就是你想查的那条语句的行号——工具自己把标注放到正确位置，不需要你换算；
    文件里已有同名标注时会直接复用。换行号可以对同一处调用换一条性质再问，
    也可以传 `facts($x)` 看抽象信息。
    返回 conclusion（含 proved / definite_violation / reason 等字段）和 evidence；
    模型内确定违反时会附 evidence 给出相关抽象事实。如何解读结论见服务器说明，返回里不再重复。
    """
    source, file_arg, label = resolve_source(file, code)
    return await run_check(
        source=source,
        source_label=label,
        file=file_arg,
        line=line,
        dsl=dsl,
        bindings=bindings,
        rustc_args=rustc_args,
        tool_hint=(
            "先调用 mirsa_variables 确认该行可见的变量名，再重试"
            "（传 file 就传同一个 file，传 code 就传同一份 code）。"
        ),
    )


@mcp.tool()
async def mirsa_read_lines(
    file: Annotated[
        str,
        Field(description="Rust 源文件路径：WSL 形式（/home/... 或 /mnt/d/...）或 Windows 形式（D:\\\\dir\\\\f.rs）都可以"),
    ],
    start: Annotated[int, Field(ge=1, description="起始行号（1 起）")],
    count: Annotated[int, Field(ge=1, le=400, description="要读取的行数，最多 400")] = 80,
) -> dict[str, Any]:
    """读源文件的一段带行号文本 —— **分析文件时的第一步，用来找到 unsafe 在第几行**。

    存在的理由：agent 自己的读文件工具走 Windows 路径，读 `/home/...` 或
    `\\\\wsl$\\...` 都会失败（实测），只有这个工具能读到 WSL 里的源码。
    路径给 Windows 形式（`D:\\dir\\f.rs`）或 WSL 形式（`/mnt/d/dir/f.rs`、`/home/...`）都行。
    只读文件、不做分析，因此不产生任何性质结论；拿到行号后再交给 mirsa_query。
    用户把代码直接贴进对话时不需要它——代码本身就在手上。
    """
    path = to_wsl_path(file)
    if not path.is_absolute() or not path.is_file():
        raise ToolError(f"input: 找不到这个文件：{file}（按 WSL 路径解析为 {path}）")
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    window = lines[start - 1 : start - 1 + count]
    return {
        "file": str(path),
        "total_lines": len(lines),
        "start": start,
        "lines": [f"{start + i:>5}: {text}" for i, text in enumerate(window)],
    }


if __name__ == "__main__":
    mcp.run(transport="stdio")
