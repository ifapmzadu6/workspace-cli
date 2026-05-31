#!/usr/bin/env python3
"""Render compact metrics for paper effect artifacts."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def load_summary(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def nested_get(value: Any, path: tuple[Any, ...]) -> Any:
    current = value
    for part in path:
        if isinstance(part, int):
            if not isinstance(current, list) or part >= len(current):
                return None
            current = current[part]
        else:
            if not isinstance(current, dict) or part not in current:
                return None
            current = current[part]
    return current


def threshold_failure_count(summary: dict[str, Any]) -> int | None:
    margins = summary.get("threshold_margins")
    if not isinstance(margins, list):
        return None
    return sum(
        1
        for margin in margins
        if isinstance(margin, dict) and margin.get("status") != "pass"
    )


def residual_count(summary: dict[str, Any], path: tuple[Any, ...]) -> int | None:
    value = nested_get(summary, path)
    return len(value) if isinstance(value, list) else None


def top_pair_conflict(summary: dict[str, Any], path: tuple[Any, ...]) -> str | None:
    conflict = nested_get(summary, path)
    if not isinstance(conflict, dict):
        return None
    repo_name = conflict.get("repo_name")
    seed = conflict.get("seed")
    candidate = conflict.get("candidate")
    true_count = conflict.get("true_target_count")
    false_count = conflict.get("residual_false_positive_count")
    if (
        not isinstance(repo_name, str)
        or not isinstance(seed, str)
        or not isinstance(candidate, str)
        or not isinstance(true_count, int)
        or isinstance(true_count, bool)
        or not isinstance(false_count, int)
        or isinstance(false_count, bool)
    ):
        return None
    return f"{repo_name} {seed}->{candidate} true={true_count} false={false_count}"


def top_residual_cluster(summary: dict[str, Any]) -> str | None:
    cluster = nested_get(
        summary,
        ("repo_temporal_holdout", "residual_gap_clusters", 0),
    )
    if not isinstance(cluster, dict):
        return None
    repo_name = cluster.get("repo_name")
    commit = cluster.get("heldout_commit")
    if not isinstance(repo_name, str) or not isinstance(commit, str):
        return None
    return f"{repo_name}@{commit}"


def metric_rows(summary: dict[str, Any]) -> list[tuple[str, Any]]:
    return [
        ("workspace commit", nested_get(summary, ("metadata", "workspace_commit"))),
        ("result summary schema", summary.get("schema_version")),
        (
            "hybrid AP@5",
            nested_get(
                summary,
                (
                    "repo_temporal_holdout",
                    "methods",
                    "workspace_related_hybrid",
                    "average_precision_at_5",
                    "mean",
                ),
            ),
        ),
        (
            "predictable hybrid AP@5",
            nested_get(
                summary,
                (
                    "repo_temporal_holdout",
                    "predictable_only",
                    "methods",
                    "workspace_related_hybrid",
                    "average_precision_at_5",
                    "mean",
                ),
            ),
        ),
        (
            "oracle-normalized AP@5",
            nested_get(
                summary,
                (
                    "repo_temporal_holdout",
                    "oracle_normalized",
                    "workspace_related_hybrid",
                    "oracle_normalized_average_precision_at_5",
                ),
            ),
        ),
        ("threshold failures", threshold_failure_count(summary)),
        (
            "residual clusters",
            residual_count(
                summary,
                ("repo_temporal_holdout", "residual_gap_clusters"),
            ),
        ),
        (
            "predictable residual clusters",
            residual_count(
                summary,
                (
                    "repo_temporal_holdout",
                    "predictable_only",
                    "residual_gap_clusters",
                ),
            ),
        ),
        (
            "residual pair conflicts",
            residual_count(
                summary,
                ("repo_temporal_holdout", "residual_pair_conflicts"),
            ),
        ),
        (
            "predictable residual pair conflicts",
            residual_count(
                summary,
                (
                    "repo_temporal_holdout",
                    "predictable_only",
                    "residual_pair_conflicts",
                ),
            ),
        ),
        ("top residual cluster", top_residual_cluster(summary)),
        (
            "top residual pair conflict",
            top_pair_conflict(
                summary,
                ("repo_temporal_holdout", "residual_pair_conflicts", 0),
            ),
        ),
    ]


def format_value(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, float):
        return f"{value:.6g}"
    if isinstance(value, str) and len(value) == 40 and all(
        character in "0123456789abcdefABCDEF" for character in value
    ):
        return value[:12]
    return str(value)


def table_cell(value: Any) -> str:
    return format_value(value).replace("|", "\\|").replace("\n", " ")


def render_summary(summary: dict[str, Any]) -> str:
    lines = [
        "### Paper effect metrics",
        "| metric | value |",
        "| --- | ---: |",
    ]
    for label, value in metric_rows(summary):
        lines.append(f"| {table_cell(label)} | {table_cell(value)} |")
    return "\n".join(lines) + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result_summary", type=Path, help="result_summary.json")
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    print(render_summary(load_summary(args.result_summary)), end="")


if __name__ == "__main__":
    main()
