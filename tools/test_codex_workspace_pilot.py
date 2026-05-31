#!/usr/bin/env python3
"""Unit tests for the Codex workspace pilot harness."""

from __future__ import annotations

import importlib.util
import json
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

    def test_policy_fixture_exposes_related_config_and_docs(self) -> None:
        workspace_binary = ROOT / "target" / "debug" / "workspace"
        if not workspace_binary.is_file():
            self.skipTest("workspace binary has not been built")

        with tempfile.TemporaryDirectory() as tmp_dir:
            repo = Path(tmp_dir) / "fixture"
            run_codex_workspace_pilot.create_policy_fixture_repo(
                repo,
                workspace_binary,
            )

            test_result = subprocess.run(
                ["python3", "-m", "unittest", "discover", "-s", "tests"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            index_result = subprocess.run(
                ["./bin/workspace", "index", "cochange", "--json"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            related_result = subprocess.run(
                [
                    "./bin/workspace",
                    "related",
                    "tests/test_discounts.py",
                    "--by",
                    "cochange",
                    "--use-index",
                    "--rank",
                    "hybrid",
                    "--json",
                ],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(test_result.returncode, 0)
            self.assertIn("2000 != 2500", test_result.stderr)
            self.assertEqual(index_result.returncode, 0, index_result.stderr)
            self.assertEqual(related_result.returncode, 0, related_result.stderr)
            self.assertIn("config/discount_policy.json", related_result.stdout)
            self.assertIn("docs/discount_policy.md", related_result.stdout)

    def test_rollback_fixture_recovers_from_bad_proposed_patch(self) -> None:
        workspace_binary = ROOT / "target" / "debug" / "workspace"
        if not workspace_binary.is_file():
            self.skipTest("workspace binary has not been built")

        with tempfile.TemporaryDirectory() as tmp_dir:
            repo = Path(tmp_dir) / "fixture"
            run_codex_workspace_pilot.create_rollback_fixture_repo(
                repo,
                workspace_binary,
            )

            initial_test = subprocess.run(
                ["python3", "-m", "unittest", "discover", "-s", "tests"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            patch_result = subprocess.run(
                [
                    "./bin/workspace",
                    "patch",
                    "--description",
                    "Validate proposed late-fee patch",
                    "docs/proposed_late_fee_fix.patch",
                    "--json",
                ],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            bad_patch_test = subprocess.run(
                ["python3", "-m", "unittest", "discover", "-s", "tests"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertNotEqual(initial_test.returncode, 0)
            self.assertIn("1000 != 1500", initial_test.stderr)
            self.assertEqual(patch_result.returncode, 0, patch_result.stderr)
            patch = json.loads(patch_result.stdout)
            transaction_id = patch["data"]["transaction_id"]
            self.assertNotEqual(bad_patch_test.returncode, 0)
            self.assertIn("600 != 450", bad_patch_test.stderr)

            rollback_result = subprocess.run(
                ["./bin/workspace", "rollback", transaction_id, "--json"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )
            diff_result = subprocess.run(
                ["git", "diff", "--name-only"],
                cwd=repo,
                text=True,
                capture_output=True,
                check=False,
            )

            self.assertEqual(rollback_result.returncode, 0, rollback_result.stderr)
            rollback = json.loads(rollback_result.stdout)
            self.assertEqual(rollback["kind"], "workspace_rollback")
            self.assertEqual(rollback["data"]["transaction_id"], transaction_id)
            self.assertEqual(diff_result.stdout, "")
            self.assertEqual(
                run_codex_workspace_pilot.workspace_operation_counts(repo),
                {"patch": 1, "rollback": 1},
            )

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

    def test_collect_condition_result_marks_expected_diff_scope(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            repo = Path(tmp_dir) / "fixture"
            repo.mkdir()
            run_codex_workspace_pilot.init_repo(repo)
            run_codex_workspace_pilot.write_text(repo / "src" / "__init__.py", "")
            run_codex_workspace_pilot.write_text(
                repo / "src" / "example.py",
                "VALUE = 1\n",
            )
            run_codex_workspace_pilot.write_text(
                repo / "tests" / "test_example.py",
                """\
import unittest

from src.example import VALUE


class ExampleTests(unittest.TestCase):
    def test_value(self):
        self.assertEqual(VALUE, 2)


if __name__ == "__main__":
    unittest.main()
""",
            )
            run_codex_workspace_pilot.commit_all(repo, "initial fixture")
            (repo / "src" / "example.py").write_text("VALUE = 2\n", encoding="utf-8")

            result = run_codex_workspace_pilot.collect_condition_result(
                {
                    "condition": "shell_only",
                    "elapsed_seconds": 1.0,
                    "codex_exit_code": 0,
                    "codex_stdout": "",
                    "codex_stderr": "",
                },
                repo,
                expected_changed_files=("src/example.py",),
            )

            self.assertTrue(result["test_passed"])
            self.assertEqual(result["changed_files"], ["src/example.py"])
            self.assertTrue(result["changed_files_match_expected"])

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
        self.assertIn("| workspace_cli | true | 15.5 | 6 | 5 | 7 | 0 |", rendered)
        self.assertIn("took 5.500s longer", rendered)
        self.assertIn("evidence of overhead", rendered)

    def test_render_markdown_reports_faster_workspace_run(self) -> None:
        summary = {
            "task": "rollback_recovery",
            "generated_at": "2026-01-01T00:00:00+00:00",
            "codex_binary": "codex",
            "workspace_binary": "target/debug/workspace",
            "results": [
                {
                    "condition": "shell_only",
                    "test_passed": True,
                    "elapsed_seconds": 20.0,
                    "command_count": 10,
                    "workspace_command_count": 0,
                    "workspace_log_entries": 0,
                    "changed_files": ["src/billing.py"],
                },
                {
                    "condition": "workspace_cli",
                    "test_passed": True,
                    "elapsed_seconds": 12.5,
                    "command_count": 8,
                    "workspace_command_count": 7,
                    "workspace_log_entries": 9,
                    "workspace_rollback_count": 1,
                    "changed_files": ["src/billing.py"],
                },
            ],
        }

        rendered = run_codex_workspace_pilot.render_markdown(summary)

        self.assertIn("| workspace_cli | true | 12.5 | 8 | 7 | 9 | 1 |", rendered)
        self.assertIn("finished 7.500s faster", rendered)
        self.assertIn("not a statistically powered efficiency claim", rendered)

    def test_policy_prompt_requests_related_and_impact(self) -> None:
        task = run_codex_workspace_pilot.task_specs()["policy_threshold_sync"]
        prompts = run_codex_workspace_pilot.condition_prompts(task)
        workspace_prompt = next(
            prompt.prompt for prompt in prompts if prompt.name == "workspace_cli"
        )

        self.assertIn("index cochange", workspace_prompt)
        self.assertIn("related tests/test_discounts.py", workspace_prompt)
        self.assertIn("impact --diff", workspace_prompt)

    def test_rollback_prompt_requests_patch_transaction_rollback(self) -> None:
        task = run_codex_workspace_pilot.task_specs()["rollback_recovery"]
        prompts = run_codex_workspace_pilot.condition_prompts(task)
        workspace_prompt = next(
            prompt.prompt for prompt in prompts if prompt.name == "workspace_cli"
        )

        self.assertIn("docs/proposed_late_fee_fix.patch", workspace_prompt)
        self.assertIn("data.transaction_id", workspace_prompt)
        self.assertIn("rollback <transaction_id>", workspace_prompt)


if __name__ == "__main__":
    unittest.main()
