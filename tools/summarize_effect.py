#!/usr/bin/env python3
"""Render effect measurement JSON as Markdown tables."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


METHOD_LABELS = {
    "baseline_git_diff_only": "git diff",
    "baseline_recent_activity": "recent activity",
    "workspace_related_direct": "related direct",
    "workspace_related_pagerank": "related PageRank",
    "workspace_related_hybrid": "related hybrid",
    "workspace_impact_direct": "impact direct",
    "workspace_impact_pagerank": "impact PageRank",
    "workspace_impact_hybrid": "impact hybrid",
}
METHOD_ORDER = [
    "baseline_git_diff_only",
    "baseline_recent_activity",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
    "workspace_impact_direct",
    "workspace_impact_pagerank",
    "workspace_impact_hybrid",
]
RELATED_METHODS = [
    "baseline_recent_activity",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
]
IMPACT_METHODS = [
    "baseline_recent_activity",
    "workspace_impact_direct",
    "workspace_impact_pagerank",
    "workspace_impact_hybrid",
]
RELATED_COMPARISONS = [
    "workspace_related_hybrid_minus_workspace_related_direct",
    "workspace_related_hybrid_minus_workspace_related_pagerank",
    "workspace_related_hybrid_minus_baseline_recent_activity",
    "workspace_related_pagerank_minus_workspace_related_direct",
]
IMPACT_COMPARISONS = [
    "workspace_impact_hybrid_minus_workspace_impact_direct",
    "workspace_impact_hybrid_minus_workspace_impact_pagerank",
    "workspace_impact_hybrid_minus_baseline_recent_activity",
    "workspace_impact_pagerank_minus_workspace_impact_direct",
]


def load_report(path: str) -> dict[str, Any]:
    if path == "-":
        return json.load(sys.stdin)
    return json.loads(Path(path).read_text())


def measurement_by_name(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    for measurement in report["measurements"]:
        if measurement["metric"] == name:
            return measurement
    return None


def label_method(method: str) -> str:
    return METHOD_LABELS.get(method, method)


def label_comparison(comparison: str) -> str:
    left, separator, right = comparison.partition("_minus_")
    if not separator:
        return comparison
    return f"{label_method(left)} - {label_method(right)}"


def fmt_number(value: Any, digits: int = 3, *, signed: bool = False) -> str:
    numeric = float(value)
    prefix = "+" if signed and numeric > 0 else ""
    return f"{prefix}{numeric:.{digits}f}"


def fmt_mean_ci(summary: dict[str, Any], metric: str) -> str:
    return (
        f"{fmt_number(summary[f'mean_{metric}'])} "
        f"({fmt_number(summary[f'ci95_low_{metric}'])}, "
        f"{fmt_number(summary[f'ci95_high_{metric}'])})"
    )


def fmt_mean(summary: dict[str, Any], metric: str) -> str:
    return fmt_number(summary[f"mean_{metric}"])


def fmt_delta_ci(summary: dict[str, Any], metric: str) -> str:
    return (
        f"{fmt_number(summary[f'mean_delta_{metric}'], signed=True)} "
        f"({fmt_number(summary[f'ci95_low_delta_{metric}'])}, "
        f"{fmt_number(summary[f'ci95_high_delta_{metric}'])})"
    )


def fmt_delta(summary: dict[str, Any], metric: str) -> str:
    return fmt_number(summary[f"mean_delta_{metric}"], signed=True)


def fmt_wtl(summary: dict[str, Any], metric: str) -> str:
    return (
        f"{summary[f'win_count_delta_{metric}']}/"
        f"{summary[f'tie_count_delta_{metric}']}/"
        f"{summary[f'loss_count_delta_{metric}']}"
    )


def repo_label(repo: str) -> str:
    return Path(repo).name


def markdown_table(headers: list[str], rows: list[list[str]]) -> str:
    if not rows:
        return "_No rows._"
    lines = [
        "| " + " | ".join(headers) + " |",
        "| " + " | ".join("---" for _ in headers) + " |",
    ]
    lines.extend("| " + " | ".join(row) + " |" for row in rows)
    return "\n".join(lines)


def present_methods(aggregate: dict[str, Any], preferred: list[str]) -> list[str]:
    preferred_present = [method for method in preferred if method in aggregate]
    extra = sorted(set(aggregate) - set(preferred_present))
    return preferred_present + extra


def render_aggregate_table(measurement: dict[str, Any], title: str) -> str:
    k = measurement["k"]
    aggregate = measurement["aggregate"]
    rows = []
    for method in present_methods(aggregate, METHOD_ORDER):
        summary = aggregate[method]
        rows.append(
            [
                label_method(method),
                str(summary["scenario_count"]),
                fmt_mean_ci(summary, f"recall_at_{k}"),
                fmt_mean_ci(summary, f"average_precision_at_{k}"),
                fmt_mean_ci(summary, f"ndcg_at_{k}"),
            ]
        )
    return "\n".join(
        [
            f"## {title} @{k}",
            markdown_table(
                ["method", "n", f"recall@{k}", f"AP@{k}", f"nDCG@{k}"],
                rows,
            ),
        ]
    )


def render_delta_table(
    measurement: dict[str, Any],
    title: str,
    comparisons: list[str],
) -> str:
    k = measurement["k"]
    deltas = measurement.get("paired_deltas", {})
    rows = []
    for comparison in comparisons:
        if comparison not in deltas:
            continue
        summary = deltas[comparison]
        ap_metric = f"average_precision_at_{k}"
        ndcg_metric = f"ndcg_at_{k}"
        rows.append(
            [
                label_comparison(comparison),
                str(summary["scenario_count"]),
                fmt_delta_ci(summary, ap_metric),
                fmt_wtl(summary, ap_metric),
                fmt_number(summary[f"p_greater_delta_{ap_metric}"], 4),
                fmt_delta_ci(summary, ndcg_metric),
                fmt_wtl(summary, ndcg_metric),
                fmt_number(summary[f"p_greater_delta_{ndcg_metric}"], 4),
            ]
        )
    return "\n".join(
        [
            f"## {title} Paired Deltas @{k}",
            markdown_table(
                [
                    "comparison",
                    "n",
                    f"delta AP@{k}",
                    "AP W/T/L",
                    "AP p>",
                    f"delta nDCG@{k}",
                    "nDCG W/T/L",
                    "nDCG p>",
                ],
                rows,
            ),
        ]
    )


def render_cutoff_table(
    measurement: dict[str, Any],
    title: str,
    methods: list[str],
    comparisons: list[str],
) -> str:
    rows = []
    for cutoff in measurement.get("cutoff_sweep", []):
        k = cutoff["k"]
        aggregate = cutoff["aggregate"]
        deltas = cutoff.get("paired_deltas", {})
        row = [str(k)]
        for method in methods:
            if method in aggregate:
                row.append(
                    fmt_number(aggregate[method][f"mean_average_precision_at_{k}"])
                )
            else:
                row.append("")
        for comparison in comparisons:
            if comparison in deltas:
                metric = f"average_precision_at_{k}"
                row.append(fmt_delta_ci(deltas[comparison], metric))
                row.append(
                    fmt_number(deltas[comparison][f"p_greater_delta_{metric}"], 4)
                )
            else:
                row.extend(["", ""])
        rows.append(row)

    headers = ["k"]
    headers.extend(f"{label_method(method)} AP" for method in methods)
    for comparison in comparisons:
        headers.append(f"{label_comparison(comparison)} delta AP")
        headers.append("p>")
    return "\n".join(
        [
            f"## {title} Cutoff Sweep",
            markdown_table(headers, rows),
        ]
    )


def render_hybrid_weight_sweep_table(
    measurement: dict[str, Any],
    title: str,
    group: str,
    direct_method: str,
    pagerank_method: str,
) -> str:
    rows = []
    k = measurement["k"]
    for entry in measurement.get("hybrid_weight_sweep", []):
        if group not in entry:
            continue
        group_data = entry[group]
        method = group_data["method"]
        aggregate = group_data["aggregate"][method]
        deltas = group_data.get("paired_deltas", {})
        direct_comparison = f"{method}_minus_{direct_method}"
        pagerank_comparison = f"{method}_minus_{pagerank_method}"
        ap_metric = f"average_precision_at_{k}"
        row = [
            fmt_number(entry["hybrid_direct_weight"], 3),
            fmt_mean_ci(aggregate, ap_metric),
            fmt_mean_ci(aggregate, f"ndcg_at_{k}"),
        ]
        for comparison in [direct_comparison, pagerank_comparison]:
            if comparison in deltas:
                row.append(fmt_delta_ci(deltas[comparison], ap_metric))
                row.append(
                    fmt_number(deltas[comparison][f"p_greater_delta_{ap_metric}"], 4)
                )
            else:
                row.extend(["", ""])
        rows.append(row)

    if not rows:
        return ""
    return "\n".join(
        [
            f"## {title} Hybrid Weight Sweep @{k}",
            markdown_table(
                [
                    "direct weight",
                    f"AP@{k}",
                    f"nDCG@{k}",
                    "delta AP vs direct",
                    "p>",
                    "delta AP vs PageRank",
                    "p>",
                ],
                rows,
            ),
        ]
    )


def render_repo_holdout_table(
    report: dict[str, Any],
    title: str,
    summary_key: str | None = None,
) -> str:
    holdouts = [
        measurement
        for measurement in report["measurements"]
        if measurement["metric"] == "repo_temporal_holdout"
    ]
    rows = []
    for holdout in holdouts:
        summary = holdout.get(summary_key, {}) if summary_key else holdout
        if not summary:
            continue
        k = summary["k"]
        aggregate = summary.get("aggregate", {})
        deltas = summary.get("paired_deltas", {})
        if not all(method in aggregate for method in RELATED_METHODS):
            continue
        ap_metric = f"average_precision_at_{k}"
        ndcg_metric = f"ndcg_at_{k}"
        hybrid_direct = "workspace_related_hybrid_minus_workspace_related_direct"
        hybrid_pagerank = "workspace_related_hybrid_minus_workspace_related_pagerank"
        hybrid_recent = "workspace_related_hybrid_minus_baseline_recent_activity"
        rows.append(
            [
                repo_label(holdout["repo"]),
                str(summary["case_count"]),
                str(summary.get("target_count", "")),
                fmt_mean(aggregate["baseline_recent_activity"], ap_metric),
                fmt_mean(aggregate["workspace_related_direct"], ap_metric),
                fmt_mean(aggregate["workspace_related_pagerank"], ap_metric),
                fmt_mean(aggregate["workspace_related_hybrid"], ap_metric),
                fmt_mean(aggregate["workspace_related_hybrid"], ndcg_metric),
                fmt_delta(deltas[hybrid_recent], ap_metric)
                if hybrid_recent in deltas
                else "",
                fmt_delta(deltas[hybrid_direct], ap_metric)
                if hybrid_direct in deltas
                else "",
                fmt_delta(deltas[hybrid_pagerank], ap_metric)
                if hybrid_pagerank in deltas
                else "",
            ]
        )

    if not rows:
        return ""
    return "\n".join(
        [
            f"## {title}",
            markdown_table(
                [
                    "repo",
                    "cases",
                    "targets",
                    "recent AP",
                    "direct AP",
                    "PageRank AP",
                    "hybrid AP",
                    "hybrid nDCG",
                    "hybrid-recent delta AP",
                    "hybrid-direct delta AP",
                    "hybrid-PageRank delta AP",
                ],
                rows,
            ),
        ]
    )


def render_measurement(
    measurement: dict[str, Any],
    title: str,
    comparisons: list[str],
    cutoff_groups: list[tuple[str, list[str], list[str]]],
) -> list[str]:
    sections = [
        render_aggregate_table(measurement, title),
        render_delta_table(measurement, title, comparisons),
    ]
    for group_title, methods, group_comparisons in cutoff_groups:
        sections.append(
            render_cutoff_table(
                measurement,
                group_title,
                methods,
                group_comparisons,
            )
        )
    return sections


def render_report(report: dict[str, Any]) -> str:
    sections = ["# Effect Measurement Summary"]

    retrieval = measurement_by_name(report, "retrieval_suite")
    if retrieval is not None:
        sections.extend(
            render_measurement(
                retrieval,
                "Retrieval Suite",
                RELATED_COMPARISONS + IMPACT_COMPARISONS,
                [
                    (
                        "Retrieval Suite Related",
                        RELATED_METHODS,
                        RELATED_COMPARISONS[:2],
                    ),
                    (
                        "Retrieval Suite Impact",
                        IMPACT_METHODS,
                        IMPACT_COMPARISONS[:2],
                    ),
                ],
            )
        )
        for weight_sweep_table in [
            render_hybrid_weight_sweep_table(
                retrieval,
                "Retrieval Suite Related",
                "related",
                "workspace_related_direct",
                "workspace_related_pagerank",
            ),
            render_hybrid_weight_sweep_table(
                retrieval,
                "Retrieval Suite Impact",
                "impact",
                "workspace_impact_direct",
                "workspace_impact_pagerank",
            ),
        ]:
            if weight_sweep_table:
                sections.append(weight_sweep_table)

    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if holdout is not None:
        per_repo = render_repo_holdout_table(report, "Per-Repo Temporal Holdout")
        if per_repo:
            sections.append(per_repo)
        sections.extend(
            render_measurement(
                holdout,
                "Cross-Repo Temporal Holdout",
                RELATED_COMPARISONS,
                [
                    (
                        "Cross-Repo Temporal Holdout",
                        RELATED_METHODS,
                        RELATED_COMPARISONS[:2],
                    )
                ],
            )
        )
        macro_average = holdout.get("repo_macro_average")
        if macro_average and macro_average.get("repo_count", 0) > 0:
            sections.append(
                render_aggregate_table(
                    macro_average,
                    "Repo-Macro Temporal Holdout",
                )
            )
            sections.append(
                render_delta_table(
                    macro_average,
                    "Repo-Macro Temporal Holdout",
                    RELATED_COMPARISONS,
                )
            )
        weight_sweep_table = render_hybrid_weight_sweep_table(
            holdout,
            "Cross-Repo Temporal Holdout",
            "related",
            "workspace_related_direct",
            "workspace_related_pagerank",
        )
        if weight_sweep_table:
            sections.append(weight_sweep_table)

        predictable = holdout.get("predictable_only")
        if predictable and predictable.get("case_count", 0) > 0:
            predictable_per_repo = render_repo_holdout_table(
                report,
                "Per-Repo Predictable Temporal Holdout",
                summary_key="predictable_only",
            )
            if predictable_per_repo:
                sections.append(predictable_per_repo)
            sections.extend(
                render_measurement(
                    predictable,
                    "Predictable Cross-Repo Temporal Holdout",
                    RELATED_COMPARISONS,
                    [
                        (
                            "Predictable Cross-Repo Temporal Holdout",
                            RELATED_METHODS,
                            RELATED_COMPARISONS[:2],
                        )
                    ],
                )
            )
            predictable_macro = predictable.get("repo_macro_average")
            if predictable_macro and predictable_macro.get("repo_count", 0) > 0:
                sections.append(
                    render_aggregate_table(
                        predictable_macro,
                        "Predictable Repo-Macro Temporal Holdout",
                    )
                )
                sections.append(
                    render_delta_table(
                        predictable_macro,
                        "Predictable Repo-Macro Temporal Holdout",
                        RELATED_COMPARISONS,
                    )
                )
            predictable_weight_sweep_table = render_hybrid_weight_sweep_table(
                predictable,
                "Predictable Cross-Repo Temporal Holdout",
                "related",
                "workspace_related_direct",
                "workspace_related_pagerank",
            )
            if predictable_weight_sweep_table:
                sections.append(predictable_weight_sweep_table)

    return "\n\n".join(sections) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "report",
        nargs="?",
        default="-",
        help="effect measurement JSON path; reads stdin when omitted or '-'",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    print(render_report(load_report(args.report)), end="")


if __name__ == "__main__":
    main()
