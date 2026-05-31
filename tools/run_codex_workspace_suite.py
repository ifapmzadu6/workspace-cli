#!/usr/bin/env python3
"""Run and aggregate repeated Codex workspace pilot tasks."""

from __future__ import annotations

import argparse
import json
import os
import random
import statistics
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

TOOLS_DIR = Path(__file__).resolve().parent
if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import run_codex_workspace_pilot


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT_DIR = ROOT / "target" / "codex-workspace-suite"
DEFAULT_REPETITIONS = 3
BOOTSTRAP_SAMPLES = 1000


def rounded(value: float) -> float:
    return round(value, 3)


def mean(values: list[float]) -> float:
    if not values:
        return 0.0
    return sum(values) / len(values)


def median(values: list[float]) -> float:
    if not values:
        return 0.0
    return float(statistics.median(values))


def percentile_sorted(values: list[float], percentile: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    index = percentile * (len(values) - 1)
    lower = int(index)
    upper = min(lower + 1, len(values) - 1)
    fraction = index - lower
    return values[lower] * (1.0 - fraction) + values[upper] * fraction


def bootstrap_mean_ci(
    values: list[float],
    *,
    label: str,
    samples: int = BOOTSTRAP_SAMPLES,
) -> dict[str, float]:
    if not values:
        return {"mean": 0.0, "ci95_low": 0.0, "ci95_high": 0.0}
    observed = mean(values)
    if len(values) == 1 or samples <= 0:
        return {
            "mean": rounded(observed),
            "ci95_low": rounded(observed),
            "ci95_high": rounded(observed),
        }
    rng = random.Random(f"codex-suite:{label}:{len(values)}:{sum(values):.6f}")
    bootstrapped = []
    for _ in range(samples):
        bootstrapped.append(mean([values[rng.randrange(len(values))] for _ in values]))
    bootstrapped.sort()
    return {
        "mean": rounded(observed),
        "ci95_low": rounded(percentile_sorted(bootstrapped, 0.025)),
        "ci95_high": rounded(percentile_sorted(bootstrapped, 0.975)),
    }


def compact_result(result: dict[str, Any]) -> dict[str, Any]:
    return {
        "condition": result["condition"],
        "test_passed": bool(result.get("test_passed")),
        "elapsed_seconds": result.get("elapsed_seconds"),
        "command_count": result.get("command_count", 0),
        "workspace_command_count": result.get("workspace_command_count", 0),
        "workspace_log_entries": result.get("workspace_log_entries", 0),
        "workspace_rollback_count": result.get("workspace_rollback_count", 0),
        "changed_files": result.get("changed_files", []),
        "expected_changed_files": result.get("expected_changed_files", []),
        "changed_files_match_expected": result.get("changed_files_match_expected"),
        "codex_exit_code": result.get("codex_exit_code"),
        "timed_out": bool(result.get("timed_out")),
    }


def compact_run(
    *,
    task: str,
    repetition: int,
    output_dir: Path,
    summary: dict[str, Any],
) -> dict[str, Any]:
    return {
        "task": task,
        "repetition": repetition,
        "summary_path": str(output_dir / "summary.json"),
        "artifact_dir": str(output_dir),
        "results": [compact_result(result) for result in summary["results"]],
    }


def condition_stats(
    runs: list[dict[str, Any]],
    *,
    condition: str,
    bootstrap_samples: int = BOOTSTRAP_SAMPLES,
) -> dict[str, Any]:
    rows = [
        result
        for run in runs
        for result in run["results"]
        if result["condition"] == condition
    ]
    elapsed = [float(row["elapsed_seconds"]) for row in rows if row["elapsed_seconds"] is not None]
    command_counts = [float(row["command_count"]) for row in rows]
    workspace_command_counts = [float(row["workspace_command_count"]) for row in rows]
    log_entries = [float(row["workspace_log_entries"]) for row in rows]
    rollbacks = [float(row["workspace_rollback_count"]) for row in rows]
    passed = sum(1 for row in rows if row["test_passed"])
    scoped_rows = [
        row for row in rows if row.get("changed_files_match_expected") is not None
    ]
    correct_diffs = sum(
        1 for row in scoped_rows if row.get("changed_files_match_expected") is True
    )
    return {
        "condition": condition,
        "runs": len(rows),
        "passed": passed,
        "pass_rate": rounded(passed / len(rows)) if rows else 0.0,
        "diff_scope_checked": len(scoped_rows),
        "diff_scope_correct": correct_diffs,
        "diff_scope_correct_rate": (
            rounded(correct_diffs / len(scoped_rows)) if scoped_rows else 0.0
        ),
        "elapsed_seconds": {
            **bootstrap_mean_ci(
                elapsed,
                label=f"{condition}:elapsed",
                samples=bootstrap_samples,
            ),
            "median": rounded(median(elapsed)),
            "min": rounded(min(elapsed)) if elapsed else 0.0,
            "max": rounded(max(elapsed)) if elapsed else 0.0,
        },
        "mean_command_count": rounded(mean(command_counts)),
        "mean_workspace_command_count": rounded(mean(workspace_command_counts)),
        "mean_workspace_log_entries": rounded(mean(log_entries)),
        "mean_workspace_rollback_count": rounded(mean(rollbacks)),
    }


def paired_deltas(
    runs: list[dict[str, Any]],
    *,
    bootstrap_samples: int = BOOTSTRAP_SAMPLES,
) -> dict[str, Any]:
    deltas = []
    command_deltas = []
    passed_pairs = 0
    for run in runs:
        by_condition = {result["condition"]: result for result in run["results"]}
        shell = by_condition.get("shell_only")
        workspace = by_condition.get("workspace_cli")
        if not shell or not workspace:
            continue
        if shell["test_passed"] and workspace["test_passed"]:
            passed_pairs += 1
            deltas.append(
                float(workspace["elapsed_seconds"]) - float(shell["elapsed_seconds"])
            )
            command_deltas.append(
                float(workspace["command_count"]) - float(shell["command_count"])
            )
    wins = sum(1 for delta in deltas if delta < 0)
    losses = sum(1 for delta in deltas if delta > 0)
    ties = len(deltas) - wins - losses
    return {
        "paired_passed_runs": passed_pairs,
        "workspace_minus_shell_elapsed_seconds": {
            **bootstrap_mean_ci(
                deltas,
                label="workspace-minus-shell-elapsed",
                samples=bootstrap_samples,
            ),
            "median": rounded(median(deltas)),
            "wins": wins,
            "ties": ties,
            "losses": losses,
        },
        "workspace_minus_shell_command_count": {
            **bootstrap_mean_ci(
                command_deltas,
                label="workspace-minus-shell-commands",
                samples=bootstrap_samples,
            ),
            "median": rounded(median(command_deltas)),
        },
    }


def aggregate_suite(
    runs: list[dict[str, Any]],
    *,
    bootstrap_samples: int = BOOTSTRAP_SAMPLES,
) -> dict[str, Any]:
    conditions = sorted(
        {
            result["condition"]
            for run in runs
            for result in run["results"]
        }
    )
    by_task: dict[str, dict[str, Any]] = {}
    for task in sorted({run["task"] for run in runs}):
        task_runs = [run for run in runs if run["task"] == task]
        by_task[task] = {
            "runs": len(task_runs),
            "conditions": {
                condition: condition_stats(
                    task_runs,
                    condition=condition,
                    bootstrap_samples=bootstrap_samples,
                )
                for condition in conditions
            },
            "paired": paired_deltas(
                task_runs,
                bootstrap_samples=bootstrap_samples,
            ),
        }
    return {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "bootstrap_samples": bootstrap_samples,
        "tasks": sorted(by_task),
        "run_count": len(runs),
        "condition_count": sum(len(run["results"]) for run in runs),
        "conditions": {
            condition: condition_stats(
                runs,
                condition=condition,
                bootstrap_samples=bootstrap_samples,
            )
            for condition in conditions
        },
        "paired": paired_deltas(runs, bootstrap_samples=bootstrap_samples),
        "by_task": by_task,
        "runs": runs,
    }


def elapsed_ci(summary: dict[str, Any]) -> str:
    elapsed = summary["elapsed_seconds"]
    return (
        f"{elapsed['mean']:.3f} "
        f"({elapsed['ci95_low']:.3f}, {elapsed['ci95_high']:.3f})"
    )


def render_suite_markdown(summary: dict[str, Any]) -> str:
    lines = [
        "# Codex Workspace Suite",
        "",
        f"- generated_at: `{summary['generated_at']}`",
        f"- workspace commit: `{summary.get('workspace_commit', '')}`",
        f"- workspace dirty: `{str(summary.get('workspace_dirty', '')).lower()}`",
        f"- tasks: `{', '.join(summary['tasks'])}`",
        f"- pilot runs: `{summary['run_count']}`",
        f"- bootstrap samples: `{summary['bootstrap_samples']}`",
        "",
        "## Overall Conditions",
        "",
        "| condition | runs | pass rate | diff-scope correct | elapsed seconds mean (95% CI) | mean commands | mean workspace commands | mean log entries | mean rollback ops |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for condition, stats in summary["conditions"].items():
        lines.append(
            "| {condition} | {runs} | {pass_rate:.3f} | {diff_correct:.3f} | "
            "{elapsed} | {commands:.3f} | {workspace_commands:.3f} | "
            "{log_entries:.3f} | {rollbacks:.3f} |".format(
                condition=condition,
                runs=stats["runs"],
                pass_rate=stats["pass_rate"],
                diff_correct=stats["diff_scope_correct_rate"],
                elapsed=elapsed_ci(stats),
                commands=stats["mean_command_count"],
                workspace_commands=stats["mean_workspace_command_count"],
                log_entries=stats["mean_workspace_log_entries"],
                rollbacks=stats["mean_workspace_rollback_count"],
            )
        )

    paired = summary["paired"]["workspace_minus_shell_elapsed_seconds"]
    lines.extend(
        [
            "",
            "## Paired Timing",
            "",
            "| comparison | paired passing runs | mean seconds (95% CI) | median seconds | workspace faster | tie | shell faster |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
            (
                "| workspace_cli - shell_only | {pairs} | {mean:.3f} ({low:.3f}, {high:.3f}) | "
                "{median:.3f} | {wins} | {ties} | {losses} |"
            ).format(
                pairs=summary["paired"]["paired_passed_runs"],
                mean=paired["mean"],
                low=paired["ci95_low"],
                high=paired["ci95_high"],
                median=paired["median"],
                wins=paired["wins"],
                ties=paired["ties"],
                losses=paired["losses"],
            ),
            "",
            "Negative seconds mean `workspace_cli` was faster on paired passing runs.",
            "",
            "## By Task",
            "",
        ]
    )
    for task, task_summary in summary["by_task"].items():
        task_paired = task_summary["paired"]["workspace_minus_shell_elapsed_seconds"]
        lines.extend(
            [
                f"### `{task}`",
                "",
                "| paired passing runs | mean seconds delta | workspace faster | shell faster |",
                "| ---: | ---: | ---: | ---: |",
                (
                    "| {pairs} | {mean:.3f} ({low:.3f}, {high:.3f}) | "
                    "{wins} | {losses} |"
                ).format(
                    pairs=task_summary["paired"]["paired_passed_runs"],
                    mean=task_paired["mean"],
                    low=task_paired["ci95_low"],
                    high=task_paired["ci95_high"],
                    wins=task_paired["wins"],
                    losses=task_paired["losses"],
                ),
                "",
            ]
        )
    return "\n".join(lines)


def resolve_tasks(raw_tasks: list[str]) -> list[str]:
    available = sorted(run_codex_workspace_pilot.task_specs())
    if raw_tasks == ["all"]:
        return available
    unknown = [task for task in raw_tasks if task not in available]
    if unknown:
        raise ValueError(
            "unknown task(s): "
            + ", ".join(unknown)
            + "; available tasks: "
            + ", ".join(available)
        )
    return raw_tasks


def run_suite(args: argparse.Namespace) -> dict[str, Any]:
    tasks = resolve_tasks(args.tasks)
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    runs = []
    for task in tasks:
        for repetition in range(1, args.repetitions + 1):
            pilot_output_dir = output_dir / task / f"run-{repetition:02d}"
            summary_path = pilot_output_dir / "summary.json"
            if args.resume and summary_path.is_file():
                summary = json.loads(summary_path.read_text(encoding="utf-8"))
            else:
                pilot_args = argparse.Namespace(
                    task=task,
                    codex_binary=args.codex_binary,
                    workspace_bin=args.workspace_bin,
                    output_dir=pilot_output_dir,
                    timeout_seconds=args.timeout_seconds,
                )
                summary = run_codex_workspace_pilot.run_pilot(pilot_args)
            runs.append(
                compact_run(
                    task=task,
                    repetition=repetition,
                    output_dir=pilot_output_dir,
                    summary=summary,
                )
            )

    summary = aggregate_suite(runs, bootstrap_samples=args.bootstrap_samples)
    summary["codex_binary"] = args.codex_binary
    summary["workspace_binary"] = str(
        run_codex_workspace_pilot.resolve_workspace_binary(args.workspace_bin)
    )
    commit = run_codex_workspace_pilot.run_command(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
    )
    status = run_codex_workspace_pilot.run_command(
        ["git", "status", "--short"],
        cwd=ROOT,
    )
    summary["workspace_commit"] = commit.stdout.strip() if commit.returncode == 0 else ""
    summary["workspace_dirty"] = bool(status.stdout.strip())
    (output_dir / "suite_summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (output_dir / "suite_summary.md").write_text(
        render_suite_markdown(summary),
        encoding="utf-8",
    )
    return summary


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--tasks",
        nargs="+",
        default=[run_codex_workspace_pilot.DEFAULT_TASK],
        help="pilot task names to run, or `all`",
    )
    parser.add_argument(
        "--repetitions",
        type=int,
        default=DEFAULT_REPETITIONS,
        help="number of repetitions per task",
    )
    parser.add_argument(
        "--codex-binary",
        default=os.environ.get("CODEX_BINARY", "codex"),
        help="codex executable to run",
    )
    parser.add_argument(
        "--workspace-bin",
        type=Path,
        help="workspace binary to copy into pilot repositories",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help="directory for suite and per-run pilot artifacts",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=int,
        default=420,
        help="timeout for each Codex condition",
    )
    parser.add_argument(
        "--bootstrap-samples",
        type=int,
        default=BOOTSTRAP_SAMPLES,
        help="bootstrap samples for mean confidence intervals",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="reuse existing per-run summary.json files when present",
    )
    args = parser.parse_args(argv)
    if args.repetitions < 1:
        parser.error("--repetitions must be at least 1")
    if args.bootstrap_samples < 0:
        parser.error("--bootstrap-samples must be non-negative")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        summary = run_suite(args)
    except Exception as error:
        print(f"codex workspace suite failed: {error}", file=sys.stderr)
        return 1
    print(render_suite_markdown(summary), end="")
    print(f"wrote codex workspace suite artifacts to {args.output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
