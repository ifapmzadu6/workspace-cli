#!/usr/bin/env python3
"""Unit tests for the Codex workspace suite aggregator."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


TOOLS_DIR = Path(__file__).resolve().parent


def load_tool(name: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS_DIR / f"{name}.py")
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


run_codex_workspace_suite = load_tool("run_codex_workspace_suite")


def result(
    condition: str,
    *,
    passed: bool,
    seconds: float,
    commands: int,
    workspace_commands: int = 0,
    log_entries: int = 0,
    rollbacks: int = 0,
) -> dict:
    return {
        "condition": condition,
        "test_passed": passed,
        "elapsed_seconds": seconds,
        "command_count": commands,
        "workspace_command_count": workspace_commands,
        "workspace_log_entries": log_entries,
        "workspace_rollback_count": rollbacks,
        "changed_files": ["src/example.py"],
        "expected_changed_files": ["src/example.py"],
        "changed_files_match_expected": True,
        "codex_exit_code": 0,
        "timed_out": False,
    }


class CodexWorkspaceSuiteTests(unittest.TestCase):
    def test_aggregate_suite_reports_pass_rate_and_paired_deltas(self) -> None:
        runs = [
            {
                "task": "rollback_recovery",
                "repetition": 1,
                "summary_path": "run-01/summary.json",
                "artifact_dir": "run-01",
                "results": [
                    result("shell_only", passed=True, seconds=100.0, commands=18),
                    result(
                        "workspace_cli",
                        passed=True,
                        seconds=80.0,
                        commands=11,
                        workspace_commands=10,
                        log_entries=10,
                        rollbacks=1,
                    ),
                ],
            },
            {
                "task": "rollback_recovery",
                "repetition": 2,
                "summary_path": "run-02/summary.json",
                "artifact_dir": "run-02",
                "results": [
                    result("shell_only", passed=True, seconds=90.0, commands=16),
                    result(
                        "workspace_cli",
                        passed=True,
                        seconds=95.0,
                        commands=12,
                        workspace_commands=10,
                        log_entries=11,
                        rollbacks=1,
                    ),
                ],
            },
        ]

        summary = run_codex_workspace_suite.aggregate_suite(
            runs,
            bootstrap_samples=0,
        )

        self.assertEqual(summary["run_count"], 2)
        self.assertEqual(summary["conditions"]["workspace_cli"]["pass_rate"], 1.0)
        self.assertEqual(
            summary["conditions"]["workspace_cli"]["mean_workspace_rollback_count"],
            1.0,
        )
        self.assertEqual(
            summary["conditions"]["workspace_cli"]["diff_scope_correct_rate"],
            1.0,
        )
        paired = summary["paired"]["workspace_minus_shell_elapsed_seconds"]
        self.assertEqual(paired["mean"], -7.5)
        self.assertEqual(paired["wins"], 1)
        self.assertEqual(paired["losses"], 1)

    def test_aggregate_suite_excludes_failed_pairs_from_timing_delta(self) -> None:
        runs = [
            {
                "task": "discounted_tax_bug",
                "repetition": 1,
                "summary_path": "run-01/summary.json",
                "artifact_dir": "run-01",
                "results": [
                    result("shell_only", passed=True, seconds=60.0, commands=10),
                    result(
                        "workspace_cli",
                        passed=False,
                        seconds=120.0,
                        commands=15,
                        workspace_commands=12,
                    ),
                ],
            }
        ]

        summary = run_codex_workspace_suite.aggregate_suite(
            runs,
            bootstrap_samples=0,
        )

        self.assertEqual(summary["conditions"]["workspace_cli"]["pass_rate"], 0.0)
        self.assertEqual(summary["paired"]["paired_passed_runs"], 0)
        self.assertEqual(
            summary["paired"]["workspace_minus_shell_elapsed_seconds"]["mean"],
            0.0,
        )

    def test_render_suite_markdown_includes_ci_and_task_rows(self) -> None:
        runs = [
            {
                "task": "rollback_recovery",
                "repetition": 1,
                "summary_path": "run-01/summary.json",
                "artifact_dir": "run-01",
                "results": [
                    result("shell_only", passed=True, seconds=100.0, commands=18),
                    result(
                        "workspace_cli",
                        passed=True,
                        seconds=80.0,
                        commands=11,
                        workspace_commands=10,
                        log_entries=10,
                        rollbacks=1,
                    ),
                ],
            }
        ]
        summary = run_codex_workspace_suite.aggregate_suite(
            runs,
            bootstrap_samples=0,
        )

        rendered = run_codex_workspace_suite.render_suite_markdown(summary)

        self.assertIn("# Codex Workspace Suite", rendered)
        self.assertIn("workspace_cli - shell_only", rendered)
        self.assertIn("-20.000 (-20.000, -20.000)", rendered)
        self.assertIn("`rollback_recovery`", rendered)

    def test_resolve_tasks_supports_all_and_rejects_unknown(self) -> None:
        tasks = run_codex_workspace_suite.resolve_tasks(["all"])

        self.assertIn("discounted_tax_bug", tasks)
        self.assertIn("rollback_recovery", tasks)
        with self.assertRaisesRegex(ValueError, "unknown task"):
            run_codex_workspace_suite.resolve_tasks(["missing"])


if __name__ == "__main__":
    unittest.main()
