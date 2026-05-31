#!/usr/bin/env python3
"""Compare two extracted effect result summaries."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


METRICS: list[tuple[str, tuple[Any, ...]]] = [
    ("result summary schema", ("schema_version",)),
    ("holdout cases", ("repo_temporal_holdout", "case_count")),
    ("holdout targets", ("repo_temporal_holdout", "target_count")),
    (
        "hybrid AP@5",
        (
            "repo_temporal_holdout",
            "methods",
            "workspace_related_hybrid",
            "average_precision_at_5",
            "mean",
        ),
    ),
    (
        "direct AP@5",
        (
            "repo_temporal_holdout",
            "methods",
            "workspace_related_direct",
            "average_precision_at_5",
            "mean",
        ),
    ),
    (
        "pagerank AP@5",
        (
            "repo_temporal_holdout",
            "methods",
            "workspace_related_pagerank",
            "average_precision_at_5",
            "mean",
        ),
    ),
    (
        "predictable hybrid AP@5",
        (
            "repo_temporal_holdout",
            "predictable_only",
            "methods",
            "workspace_related_hybrid",
            "average_precision_at_5",
            "mean",
        ),
    ),
    (
        "oracle-normalized AP@5",
        (
            "repo_temporal_holdout",
            "oracle_normalized",
            "workspace_related_hybrid",
            "oracle_normalized_average_precision_at_5",
        ),
    ),
    (
        "hybrid-direct delta AP@5",
        (
            "repo_temporal_holdout",
            "key_deltas",
            "workspace_related_hybrid_minus_workspace_related_direct",
            "mean_delta",
        ),
    ),
    (
        "hybrid-direct Holm p",
        (
            "repo_temporal_holdout",
            "key_deltas",
            "workspace_related_hybrid_minus_workspace_related_direct",
            "p_greater_holm",
        ),
    ),
    (
        "hybrid-pagerank Holm p",
        (
            "repo_temporal_holdout",
            "key_deltas",
            "workspace_related_hybrid_minus_workspace_related_pagerank",
            "p_greater_holm",
        ),
    ),
    (
        "top residual gap",
        (
            "repo_temporal_holdout",
            "residual_gap_clusters",
            0,
            "oracle_gap_average_precision_at_5",
        ),
    ),
]


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


def compare_summaries(
    old: dict[str, Any],
    new: dict[str, Any],
) -> list[dict[str, Any]]:
    rows = []
    for label, path in METRICS:
        old_value = nested_get(old, path)
        new_value = nested_get(new, path)
        rows.append(make_row(label, old_value, new_value))
    rows.append(
        make_row(
            "threshold failures",
            threshold_failure_count(old),
            threshold_failure_count(new),
        )
    )
    return rows


def make_row(label: str, old_value: Any, new_value: Any) -> dict[str, Any]:
    delta = None
    if is_number(old_value) and is_number(new_value):
        delta = float(new_value) - float(old_value)
        if isinstance(old_value, int) and isinstance(new_value, int):
            delta = int(delta)
    return {
        "metric": label,
        "old": old_value,
        "new": new_value,
        "delta": delta,
    }


def is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def format_cell(value: Any, *, signed: bool = False) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, int):
        return f"{value:+d}" if signed else str(value)
    if isinstance(value, float):
        prefix = "+" if signed and value > 0 else ""
        return f"{prefix}{value:.6g}"
    return str(value)


def render_markdown(rows: list[dict[str, Any]]) -> str:
    lines = [
        "| metric | old | new | delta |",
        "| --- | ---: | ---: | ---: |",
    ]
    for row in rows:
        lines.append(
            "| "
            + " | ".join(
                [
                    str(row["metric"]),
                    format_cell(row["old"]),
                    format_cell(row["new"]),
                    format_cell(row["delta"], signed=True),
                ]
            )
            + " |"
        )
    return "\n".join(lines) + "\n"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("old", type=Path, help="baseline result_summary.json")
    parser.add_argument("new", type=Path, help="candidate result_summary.json")
    return parser.parse_args(argv)


def main() -> None:
    args = parse_args()
    rows = compare_summaries(load_summary(args.old), load_summary(args.new))
    print(render_markdown(rows), end="")


if __name__ == "__main__":
    main()
