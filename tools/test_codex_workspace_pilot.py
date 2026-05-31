#!/usr/bin/env python3
"""Unit tests for the Codex workspace pilot harness."""

from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path


TOOLS_DIR = Path(__file__).resolve().parent
ROOT = TOOLS_DIR.parent


def load_tool(name: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS_DIR / f"{name}.py")
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


run_codex_workspace_pilot = load_tool("run_codex_workspace_pilot")


class CodexWorkspacePilotTests(unittest.TestCase):
    def test_fixture_repo_starts_with_failing_checkout_test(self) -> None:
        workspace_binary = ROOT / "target" / "debug" / "workspace"
        if not workspace_binary.is_file():
            self.skipTest("workspace binary has not been built")

        with tempfile.TemporaryDirectory() as tmp_dir:
            repo = Path(tmp_dir) / "fixture"
            run_codex_workspace_pilot.create_fixture_repo(repo, workspace_binary)

            result = subprocess.run(
                ["python3", "-m", "unittest", "discover", "-s", "tests"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("95.0 != 93.0", result.stderr)
            self.assertTrue((repo / "bin" / "workspace").is_file())

    def test_command_like_values_extracts_codex_command_events(self) -> None:
        events = [
            {
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": "./bin/workspace status --json",
                },
            },
            {
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": ["python3", "-m", "unittest"],
                },
            },
            {
                "type": "item.completed",
                "item": {
                    "type": "command_execution",
                    "command": "./bin/workspace status --json",
                },
            },
        ]

        commands = run_codex_workspace_pilot.command_like_values(events)

        self.assertEqual(
            commands,
            ["./bin/workspace status --json", "python3 -m unittest"],
        )

    def test_render_markdown_calls_out_tiny_task_overhead(self) -> None:
        summary = {
            "task": "discounted_tax_bug",
            "generated_at": "2026-01-01T00:00:00+00:00",
            "codex_binary": "codex",
            "workspace_binary": "target/debug/workspace",
            "results": [
                {
                    "condition": "shell_only",
                    "test_passed": True,
                    "elapsed_seconds": 10.0,
                    "command_count": 4,
                    "workspace_command_count": 0,
                    "workspace_log_entries": 0,
                    "changed_files": ["src/checkout.py"],
                },
                {
                    "condition": "workspace_cli",
                    "test_passed": True,
                    "elapsed_seconds": 15.5,
                    "command_count": 6,
                    "workspace_command_count": 5,
                    "workspace_log_entries": 7,
                    "changed_files": ["src/checkout.py"],
                },
            ],
        }

        rendered = run_codex_workspace_pilot.render_markdown(summary)

        self.assertIn("| workspace_cli | true | 15.5 | 6 | 5 | 7 |", rendered)
        self.assertIn("took 5.500s longer", rendered)
        self.assertIn("evidence of overhead", rendered)


if __name__ == "__main__":
    unittest.main()
