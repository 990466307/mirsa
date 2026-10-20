import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import anyio
from mcp import Client
from mcp.client.stdio import StdioServerParameters
from mcp.server.mcpserver.exceptions import ToolError

import server

ROOT = server.ROOT
MARKER = server.AUTO_MARKER
TARGET = ROOT / "examples/inputs/query_target.rs"
ACCESS_LINE = 6

EXPECTED_TOOLS = ["mirsa_query", "mirsa_read_lines", "mirsa_variables"]

PASTED = (
    "pub fn definite() -> u8 {\n"
    "    let table = [1u8, 2, 3];\n"
    "    let slot = 7usize;\n"
    "    unsafe { *table.get_unchecked(slot) }\n"
    "}\n"
    "\n"
    "pub fn guarded(data: &[u8], index: usize) -> u8 {\n"
    "    if index < data.len() {\n"
    "        unsafe { *data.get_unchecked(index) }\n"
    "    } else {\n"
    "        0\n"
    "    }\n"
    "}\n"
)


class ToolIntegration(unittest.IsolatedAsyncioTestCase):
    async def exercise_line_entry(self, connection, mode="auto"):
        original = TARGET.read_bytes()
        async with Client(connection, mode=mode, read_timeout_seconds=120) as client:
            tools = {tool.name: tool for tool in (await client.list_tools()).tools}
            self.assertEqual(sorted(tools), EXPECTED_TOOLS)
            for name, required in [
                ("mirsa_read_lines", {"file", "start"}),
                ("mirsa_variables", {"line"}),
                ("mirsa_query", {"line", "dsl"}),
            ]:
                self.assertEqual(set(tools[name].input_schema["required"]), required, name)

            result = await client.call_tool("mirsa_read_lines", {"file": str(TARGET), "start": 1})
            self.assertFalse(result.is_error, result)
            body = result.structured_content
            self.assertEqual(body["total_lines"], len(original.decode().splitlines()))
            self.assertTrue(body["lines"][0].strip().startswith("1:"))

            result = await client.call_tool(
                "mirsa_variables",
                {"file": str(TARGET), "line": ACCESS_LINE, "candidates": ["i", "s", "value", "nope"]},
            )
            self.assertFalse(result.is_error, result)
            facts = result.structured_content
            self.assertIn("i", facts["visible"])
            self.assertIn("s", facts["visible"])
            self.assertIn("nope", facts["not_visible"])

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE, "dsl": f"$i < len($s) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            output = result.structured_content
            self.assertNotIn("output", output)
            self.assertTrue(output["conclusion"]["proved"])
            self.assertIs(output["conclusion"]["result"], True)
            self.assertIn("definite_violation", output["conclusion"])
            self.assertIn("unreachable", output["conclusion"])
            self.assertEqual(output["resolved_bindings"], {"i": "i", "s": "s"})
            self.assertEqual(output["location"]["line"], ACCESS_LINE)
            self.assertIn("get_unchecked", TARGET.read_text(encoding="utf-8").splitlines()[ACCESS_LINE - 1])

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE,
                 "dsl": f"$idx < len($tbl) at '{MARKER}",
                 "bindings": {"tbl": "s", "idx": "i"}},
            )
            self.assertFalse(result.is_error, result)
            self.assertIs(result.structured_content["conclusion"]["result"], True)

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE, "dsl": f"$i >= len($s) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            self.assertTrue(result.structured_content["conclusion"]["definite_violation"])

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE, "dsl": f"$nope < len($s) at '{MARKER}"},
            )
            self.assertTrue(result.is_error, result)
            self.assertIn("mirsa_variables", result.content[0].text)

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE, "dsl": "$i < len($s) at 'access"},
            )
            self.assertTrue(result.is_error, result)
            self.assertIn(MARKER, result.content[0].text)

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": 5, "dsl": f"$i < len($s) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            self.assertTrue(result.structured_content["conclusion"]["proved"])

            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": 1,
                 "dsl": f"$i < len($s) at '{MARKER}", "bindings": {"i": "i", "s": "s"}},
            )
            self.assertTrue(result.is_error, result)

            with tempfile.TemporaryDirectory() as directory:
                invalid = Path(directory) / "invalid.rs"
                invalid.write_text("invalid rust")
                result = await client.call_tool(
                    "mirsa_query",
                    {"file": str(invalid), "line": 1, "dsl": f"$i < len($s) at '{MARKER}"},
                )
                self.assertTrue(result.is_error, result)
                self.assertIn("compilation:", result.content[0].text)
            result = await client.call_tool(
                "mirsa_query",
                {"file": str(TARGET), "line": ACCESS_LINE, "dsl": f"$i < len($s) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
        self.assertEqual(TARGET.read_bytes(), original)

    async def exercise_pasted_code(self, connection, mode="auto"):
        async with Client(connection, mode=mode, read_timeout_seconds=120) as client:
            result = await client.call_tool(
                "mirsa_query",
                {"code": PASTED, "line": 4, "dsl": f"$slot < len($table) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            body = result.structured_content
            self.assertEqual(body["location"]["source"], "<pasted code>")
            self.assertIn("get_unchecked", body["location"]["annotated_statement"])
            conclusion = body["conclusion"]
            self.assertIs(conclusion["definite_violation"], True)
            self.assertIs(conclusion["proved"], False)
            self.assertIsNone(conclusion["reason"])
            self.assertIn("$slot = [7, 7]", conclusion["evidence"]["slot"])
            self.assertIn("Length($table) = [3, 3]", conclusion["evidence"]["table"])

            result = await client.call_tool(
                "mirsa_query",
                {"code": PASTED, "line": 9, "dsl": f"$index < len($data) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            conclusion = result.structured_content["conclusion"]
            self.assertIs(conclusion["proved"], True)
            self.assertNotIn("evidence", conclusion)
            self.assertNotIn("analysis_limitation", conclusion)

            result = await client.call_tool(
                "mirsa_query",
                {"code": PASTED, "line": 9, "dsl": f"$index >= len($data) at '{MARKER}"},
            )
            self.assertFalse(result.is_error, result)
            conclusion = result.structured_content["conclusion"]
            self.assertIs(conclusion["definite_violation"], True)

            vars_body = (await client.call_tool(
                "mirsa_variables",
                {"code": PASTED, "line": 4, "candidates": ["slot", "table", "nope"]},
            )).structured_content
            self.assertIn("slot", vars_body["visible"])
            self.assertIn("table", vars_body["visible"])
            self.assertIn("nope", vars_body["not_visible"])
            self.assertEqual(vars_body["source"], "<pasted code>")

            result = await client.call_tool(
                "mirsa_variables", {"line": 4}
            )
            self.assertTrue(result.is_error, result)

            result = await client.call_tool(
                "mirsa_query", {"code": "   ", "line": 1, "dsl": f"$x != 0 at '{MARKER}"}
            )
            self.assertTrue(result.is_error, result)

    async def test_line_entry_stdio(self):
        await self.exercise_line_entry(
            StdioServerParameters(command="/bin/bash", args=[str(ROOT / "scripts/start-mcp.sh")])
        )

    async def test_line_entry_legacy_stdio(self):
        await self.exercise_line_entry(
            StdioServerParameters(command="/bin/bash", args=[str(ROOT / "scripts/start-mcp.sh")]), "legacy"
        )

    async def test_pasted_code_stdio(self):
        await self.exercise_pasted_code(
            StdioServerParameters(command="/bin/bash", args=[str(ROOT / "scripts/start-mcp.sh")])
        )

    async def test_pasted_code_legacy_stdio(self):
        await self.exercise_pasted_code(
            StdioServerParameters(command="/bin/bash", args=[str(ROOT / "scripts/start-mcp.sh")]), "legacy"
        )

    @unittest.skipUnless(shutil.which("wsl.exe") and os.environ.get("WSL_DISTRO_NAME"), "requires WSL interop")
    async def test_windows_wsl_bridge(self):
        await self.exercise_line_entry(StdioServerParameters(
            command=shutil.which("wsl.exe"),
            args=["-d", os.environ["WSL_DISTRO_NAME"], "-u", os.environ["USER"], "--exec",
                  "/bin/bash", str(ROOT / "scripts/start-mcp.sh")],
            env={"WSL_INTEROP": os.environ["WSL_INTEROP"]},
        ))

    async def test_timeout_and_cancellation_stop_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            worker = directory / "slow-worker"
            pid_file = directory / "pid"
            worker.write_text(
                "#!/usr/bin/python3\nimport os, time\n"
                f"open({str(pid_file)!r}, 'w').write(str(os.getpid()))\n"
                "time.sleep(60)\n"
            )
            worker.chmod(0o755)
            with patch.object(server, "WORKER", worker):
                for cancel in (False, True):
                    with patch.object(server, "QUERY_SECONDS", 90 if cancel else 0.3):
                        if cancel:
                            with anyio.move_on_after(0.3) as scope:
                                await server.run_check(
                                    source=TARGET.read_text(encoding="utf-8"),
                                    source_label=str(TARGET), file=str(TARGET),
                                    line=ACCESS_LINE, dsl=f"true at '{MARKER}",
                                    bindings=None, rustc_args=None, tool_hint="",
                                )
                            self.assertTrue(scope.cancel_called)
                        else:
                            with self.assertRaisesRegex(ToolError, "timeout:"):
                                await server.run_check(
                                    source=TARGET.read_text(encoding="utf-8"),
                                    source_label=str(TARGET), file=str(TARGET),
                                    line=ACCESS_LINE, dsl=f"true at '{MARKER}",
                                    bindings=None, rustc_args=None, tool_hint="",
                                )
                    pid = int(pid_file.read_text())
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
                    pid_file.unlink()


if __name__ == "__main__":
    unittest.main(verbosity=2)
