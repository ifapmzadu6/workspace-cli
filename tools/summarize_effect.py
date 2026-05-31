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
    "baseline_path_locality": "path locality",
    "baseline_lexical_similarity": "lexical similarity",
    "baseline_content_similarity": "content similarity",
    "baseline_recent_activity": "recent activity",
    "baseline_global_pagerank": "global PageRank",
    "history_oracle_ceiling": "history oracle ceiling",
    "workspace_related_direct": "related direct",
    "workspace_related_pagerank": "related PageRank",
    "workspace_related_hybrid": "related hybrid",
    "workspace_related_hybrid_loro": "related LORO hybrid",
    "workspace_impact_direct": "impact direct",
    "workspace_impact_pagerank": "impact PageRank",
    "workspace_impact_hybrid": "impact hybrid",
}
METHOD_ORDER = [
    "baseline_git_diff_only",
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    "history_oracle_ceiling",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
    "workspace_related_hybrid_loro",
    "workspace_impact_direct",
    "workspace_impact_pagerank",
    "workspace_impact_hybrid",
]
RELATED_METHODS = [
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
]
HOLDOUT_RELATED_METHODS = [
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    "history_oracle_ceiling",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
]
IMPACT_METHODS = [
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    "workspace_impact_direct",
    "workspace_impact_pagerank",
    "workspace_impact_hybrid",
]
RELATED_COMPARISONS = [
    "workspace_related_hybrid_minus_workspace_related_direct",
    "workspace_related_hybrid_minus_workspace_related_pagerank",
    "workspace_related_hybrid_minus_baseline_path_locality",
    "workspace_related_hybrid_minus_baseline_lexical_similarity",
    "workspace_related_hybrid_minus_baseline_content_similarity",
    "workspace_related_hybrid_minus_baseline_recent_activity",
    "workspace_related_hybrid_minus_baseline_global_pagerank",
    "workspace_related_pagerank_minus_workspace_related_direct",
]
RELATED_LORO_COMPARISONS = [
    "workspace_related_hybrid_loro_minus_workspace_related_direct",
    "workspace_related_hybrid_loro_minus_workspace_related_pagerank",
    "workspace_related_hybrid_loro_minus_baseline_path_locality",
    "workspace_related_hybrid_loro_minus_baseline_lexical_similarity",
    "workspace_related_hybrid_loro_minus_baseline_content_similarity",
    "workspace_related_hybrid_loro_minus_baseline_recent_activity",
    "workspace_related_hybrid_loro_minus_baseline_global_pagerank",
    "workspace_related_hybrid_loro_minus_workspace_related_hybrid",
]
IMPACT_COMPARISONS = [
    "workspace_impact_hybrid_minus_workspace_impact_direct",
    "workspace_impact_hybrid_minus_workspace_impact_pagerank",
    "workspace_impact_hybrid_minus_baseline_path_locality",
    "workspace_impact_hybrid_minus_baseline_lexical_similarity",
    "workspace_impact_hybrid_minus_baseline_content_similarity",
    "workspace_impact_hybrid_minus_baseline_recent_activity",
    "workspace_impact_hybrid_minus_baseline_global_pagerank",
    "workspace_impact_pagerank_minus_workspace_impact_direct",
]
CASE_DELTA_COMPARISONS = [
    ("workspace_related_hybrid", "workspace_related_direct"),
    ("workspace_related_hybrid", "baseline_path_locality"),
    ("workspace_related_hybrid", "baseline_lexical_similarity"),
    ("workspace_related_hybrid", "baseline_content_similarity"),
    ("workspace_related_hybrid", "baseline_recent_activity"),
    ("workspace_related_hybrid", "baseline_global_pagerank"),
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


def fmt_p_value(summary: dict[str, Any], key: str) -> str:
    if key not in summary:
        return ""
    value = float(summary[key])
    if 0.0 < value < 0.0001:
        return "<0.0001"
    return fmt_number(value, 4)


def fmt_distribution(summary: dict[str, Any]) -> str:
    return (
        f"{fmt_number(summary['mean'])} "
        f"({summary['min']}/{fmt_number(summary['median'])}/{summary['max']})"
    )


def fmt_skipped(skipped: dict[str, Any]) -> str:
    return ", ".join(
        [
            f"root={skipped.get('root_commit', 0)}",
            f"few={skipped.get('too_few_files', 0)}",
            f"broad={skipped.get('too_many_files', 0)}",
            f"new_seed={skipped.get('new_seed_file', 0)}",
        ]
    )


def repo_label(repo: str) -> str:
    return Path(repo).name


def markdown_table(headers: list[str], rows: list[list[str]]) -> str:
    if not rows:
        return "_No rows._"
    escaped_headers = [table_cell(header) for header in headers]
    lines = [
        "| " + " | ".join(escaped_headers) + " |",
        "| " + " | ".join("---" for _ in escaped_headers) + " |",
    ]
    lines.extend(
        "| " + " | ".join(table_cell(cell) for cell in row) + " |"
        for row in rows
    )
    return "\n".join(lines)


def table_cell(value: str) -> str:
    return value.replace("|", "\\|").replace("\n", " ")


def present_methods(aggregate: dict[str, Any], preferred: list[str]) -> list[str]:
    preferred_present = [method for method in preferred if method in aggregate]
    extra = sorted(set(aggregate) - set(preferred_present))
    return preferred_present + extra


def average_precision_at_k(top: list[str], expected: set[str], k: int) -> float:
    if not expected:
        return 1.0
    seen_hits: set[str] = set()
    precision_sum = 0.0
    for index, path in enumerate(top[:k], start=1):
        if path not in expected or path in seen_hits:
            continue
        seen_hits.add(path)
        precision_sum += len(seen_hits) / index
    return round(precision_sum / len(expected), 3)


def hits_at_k(top: list[str], expected: set[str], k: int) -> list[str]:
    hits = []
    seen: set[str] = set()
    for path in top[:k]:
        if path in expected and path not in seen:
            hits.append(path)
            seen.add(path)
    return hits


def fmt_path_list(paths: list[str], limit: int = 2) -> str:
    if not paths:
        return ""
    visible = paths[:limit]
    suffix = f", +{len(paths) - limit} more" if len(paths) > limit else ""
    return ", ".join(visible) + suffix


def short_commit(commit: Any) -> str:
    return str(commit)[:10] if commit else ""


def render_metadata_table(report: dict[str, Any]) -> str:
    metadata = report.get("metadata")
    if not isinstance(metadata, dict):
        if "workspace_bin" not in report:
            return ""
        metadata = {"workspace_bin": report["workspace_bin"]}

    holdouts = metadata.get("repo_holdouts", [])
    holdout_text = ""
    if isinstance(holdouts, list):
        holdout_parts = []
        for holdout in holdouts:
            if not isinstance(holdout, dict):
                continue
            repo = holdout.get("repo", "")
            ref = holdout.get("ref", "")
            remote_url = holdout.get("remote_url")
            label = f"{repo}@{short_commit(ref)}"
            if remote_url:
                label = f"{label} ({remote_url})"
            holdout_parts.append(label)
        holdout_text = ", ".join(holdout_parts)

    rows = [
        ["workspace commit", short_commit(metadata.get("workspace_commit"))],
        ["workspace dirty", "yes" if metadata.get("workspace_dirty") else "no"],
        ["workspace bin", str(metadata.get("workspace_bin", ""))],
        ["primary k", str(metadata.get("primary_k", ""))],
        ["bootstrap samples", str(metadata.get("bootstrap_samples", ""))],
        ["sign-flip samples", str(metadata.get("sign_flip_samples", ""))],
        ["repo holdouts", holdout_text or "none"],
    ]
    if metadata.get("sign_flip_method"):
        rows.insert(-1, ["sign-flip method", str(metadata["sign_flip_method"])])
    if metadata.get("repo_holdout_manifest"):
        rows.append(["holdout manifest", str(metadata["repo_holdout_manifest"])])
    if metadata.get("repo_holdout_manifest_sha256"):
        rows.append(
            [
                "manifest sha256",
                str(metadata["repo_holdout_manifest_sha256"])[:16],
            ]
        )
    if metadata.get("repo_holdout_source_manifest"):
        rows.append(
            [
                "source manifest",
                str(metadata["repo_holdout_source_manifest"]),
            ]
        )
    if metadata.get("repo_holdout_source_manifest_sha256"):
        rows.append(
            [
                "source manifest sha256",
                str(metadata["repo_holdout_source_manifest_sha256"])[:16],
            ]
        )
    return "\n".join(
        [
            "## Reproducibility Metadata",
            markdown_table(["field", "value"], rows),
        ]
    )


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


def render_oracle_normalized_table(
    measurement: dict[str, Any],
    title: str,
    methods: list[str],
) -> str:
    aggregate = measurement.get("aggregate", {})
    oracle = aggregate.get("history_oracle_ceiling")
    if not oracle:
        return ""
    k = measurement["k"]
    metric = f"average_precision_at_{k}"
    oracle_ap = float(oracle[f"mean_{metric}"])
    if oracle_ap <= 0.0:
        return ""

    rows = []
    for method in methods:
        summary = aggregate.get(method)
        if not summary:
            continue
        ap = float(summary[f"mean_{metric}"])
        rows.append(
            [
                label_method(method),
                fmt_number(ap),
                fmt_number(oracle_ap),
                fmt_number(ap / oracle_ap),
                fmt_number(oracle_ap - ap),
            ]
        )

    if not rows:
        return ""
    return "\n".join(
        [
            f"## {title} Oracle-Normalized AP @{k}",
            markdown_table(
                ["method", f"AP@{k}", "oracle AP", "AP/oracle", "oracle gap"],
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
                fmt_p_value(summary, f"p_greater_delta_{ap_metric}"),
                fmt_p_value(summary, f"p_greater_holm_delta_{ap_metric}"),
                fmt_delta_ci(summary, ndcg_metric),
                fmt_wtl(summary, ndcg_metric),
                fmt_p_value(summary, f"p_greater_delta_{ndcg_metric}"),
                fmt_p_value(summary, f"p_greater_holm_delta_{ndcg_metric}"),
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
                    "AP Holm p>",
                    f"delta nDCG@{k}",
                    "nDCG W/T/L",
                    "nDCG p>",
                    "nDCG Holm p>",
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
                    fmt_p_value(deltas[comparison], f"p_greater_delta_{metric}")
                )
                row.append(
                    fmt_p_value(deltas[comparison], f"p_greater_holm_delta_{metric}")
                )
            else:
                row.extend(["", "", ""])
        rows.append(row)

    headers = ["k"]
    headers.extend(f"{label_method(method)} AP" for method in methods)
    for comparison in comparisons:
        headers.append(f"{label_comparison(comparison)} delta AP")
        headers.append("p>")
        headers.append("Holm p>")
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
                    fmt_p_value(deltas[comparison], f"p_greater_delta_{ap_metric}")
                )
                row.append(
                    fmt_p_value(deltas[comparison], f"p_greater_holm_delta_{ap_metric}")
                )
            else:
                row.extend(["", "", ""])
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
                    "Holm p>",
                    "delta AP vs PageRank",
                    "p>",
                    "Holm p>",
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
        optional_methods = {
            "baseline_path_locality",
            "baseline_global_pagerank",
            "history_oracle_ceiling",
        }
        required_methods = [
            method
            for method in HOLDOUT_RELATED_METHODS
            if method not in optional_methods
        ]
        if not all(method in aggregate for method in required_methods):
            continue
        ap_metric = f"average_precision_at_{k}"
        ndcg_metric = f"ndcg_at_{k}"
        hybrid_direct = "workspace_related_hybrid_minus_workspace_related_direct"
        hybrid_pagerank = "workspace_related_hybrid_minus_workspace_related_pagerank"
        hybrid_path = "workspace_related_hybrid_minus_baseline_path_locality"
        hybrid_lexical = "workspace_related_hybrid_minus_baseline_lexical_similarity"
        hybrid_content = "workspace_related_hybrid_minus_baseline_content_similarity"
        hybrid_recent = "workspace_related_hybrid_minus_baseline_recent_activity"
        hybrid_global = "workspace_related_hybrid_minus_baseline_global_pagerank"
        path_locality = aggregate.get("baseline_path_locality")
        lexical_similarity = aggregate.get("baseline_lexical_similarity")
        content_similarity = aggregate.get("baseline_content_similarity")
        global_pagerank = aggregate.get("baseline_global_pagerank")
        history_oracle = aggregate.get("history_oracle_ceiling")
        rows.append(
            [
                repo_label(holdout["repo"]),
                str(summary["case_count"]),
                str(summary.get("target_count", "")),
                fmt_mean(history_oracle, ap_metric) if history_oracle else "",
                fmt_mean(path_locality, ap_metric) if path_locality else "",
                fmt_mean(lexical_similarity, ap_metric) if lexical_similarity else "",
                fmt_mean(content_similarity, ap_metric) if content_similarity else "",
                fmt_mean(aggregate["baseline_recent_activity"], ap_metric),
                fmt_mean(global_pagerank, ap_metric) if global_pagerank else "",
                fmt_mean(aggregate["workspace_related_direct"], ap_metric),
                fmt_mean(aggregate["workspace_related_pagerank"], ap_metric),
                fmt_mean(aggregate["workspace_related_hybrid"], ap_metric),
                fmt_mean(aggregate["workspace_related_hybrid"], ndcg_metric),
                fmt_delta(deltas[hybrid_path], ap_metric)
                if hybrid_path in deltas
                else "",
                fmt_delta(deltas[hybrid_lexical], ap_metric)
                if hybrid_lexical in deltas
                else "",
                fmt_delta(deltas[hybrid_content], ap_metric)
                if hybrid_content in deltas
                else "",
                fmt_delta(deltas[hybrid_recent], ap_metric)
                if hybrid_recent in deltas
                else "",
                fmt_delta(deltas[hybrid_global], ap_metric)
                if hybrid_global in deltas
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
                    "history oracle AP",
                    "path AP",
                    "lexical AP",
                    "content AP",
                    "recent AP",
                    "global PR AP",
                    "direct AP",
                    "PageRank AP",
                    "hybrid AP",
                    "hybrid nDCG",
                    "hybrid-path delta AP",
                    "hybrid-lexical delta AP",
                    "hybrid-content delta AP",
                    "hybrid-recent delta AP",
                    "hybrid-global delta AP",
                    "hybrid-direct delta AP",
                    "hybrid-PageRank delta AP",
                ],
                rows,
            ),
        ]
    )


def case_delta_entries(
    report: dict[str, Any],
    *,
    expected_key: str,
    left_method: str,
    right_method: str,
    k: int,
) -> list[dict[str, Any]]:
    entries = []
    for holdout in report["measurements"]:
        if holdout.get("metric") != "repo_temporal_holdout":
            continue
        for case in holdout.get("cases", []):
            expected = set(case.get(expected_key, []))
            methods = case.get("methods", {})
            if (
                not expected
                or left_method not in methods
                or right_method not in methods
            ):
                continue
            left_top = [str(path) for path in methods[left_method].get("top", [])]
            right_top = [str(path) for path in methods[right_method].get("top", [])]
            left_ap = average_precision_at_k(left_top, expected, k)
            right_ap = average_precision_at_k(right_top, expected, k)
            entries.append(
                {
                    "repo": repo_label(
                        str(case.get("repo", holdout.get("repo", "")))
                    ),
                    "seed": str(case.get("seed", "")),
                    "commit": short_commit(case.get("heldout_commit")),
                    "expected": sorted(expected),
                    "left_ap": left_ap,
                    "right_ap": right_ap,
                    "delta": round(left_ap - right_ap, 3),
                    "left_hits": hits_at_k(left_top, expected, k),
                    "right_hits": hits_at_k(right_top, expected, k),
                }
            )
    return entries


def render_case_delta_table(
    report: dict[str, Any],
    title: str,
    *,
    expected_key: str = "expected",
    comparisons: list[tuple[str, str]] = CASE_DELTA_COMPARISONS,
    limit: int = 2,
) -> str:
    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if holdout is None:
        return ""
    k = holdout["k"]
    rows = []
    for left_method, right_method in comparisons:
        entries = case_delta_entries(
            report,
            expected_key=expected_key,
            left_method=left_method,
            right_method=right_method,
            k=k,
        )
        wins = sorted(
            [entry for entry in entries if entry["delta"] > 0],
            key=lambda entry: (
                -entry["delta"],
                entry["repo"],
                entry["seed"],
                entry["commit"],
            ),
        )[:limit]
        losses = sorted(
            [entry for entry in entries if entry["delta"] < 0],
            key=lambda entry: (
                entry["delta"],
                entry["repo"],
                entry["seed"],
                entry["commit"],
            ),
        )[:limit]
        for direction, selected in [("hybrid win", wins), ("hybrid loss", losses)]:
            for entry in selected:
                rows.append(
                    [
                        label_comparison(f"{left_method}_minus_{right_method}"),
                        direction,
                        entry["repo"],
                        entry["seed"],
                        entry["commit"],
                        fmt_path_list(entry["expected"]),
                        fmt_number(entry["delta"], signed=True),
                        fmt_number(entry["left_ap"]),
                        fmt_number(entry["right_ap"]),
                        fmt_path_list(entry["left_hits"]),
                        fmt_path_list(entry["right_hits"]),
                    ]
                )
    if not rows:
        return ""
    return "\n".join(
        [
            f"## {title} Case Deltas @{k}",
            markdown_table(
                [
                    "comparison",
                    "direction",
                    "repo",
                    "seed",
                    "commit",
                    "targets",
                    "delta AP",
                    "hybrid AP",
                    "baseline AP",
                    "hybrid hits",
                    "baseline hits",
                ],
                rows,
            ),
        ]
    )


def residual_gap_cluster_entries(
    report: dict[str, Any],
    *,
    expected_key: str = "expected",
    retarget_metrics: bool = False,
    method: str = "workspace_related_hybrid",
    oracle_method: str = "history_oracle_ceiling",
    limit: int = 8,
) -> list[dict[str, Any]]:
    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if holdout is None:
        return []
    k = holdout["k"]
    metric = f"average_precision_at_{k}"
    clusters: dict[tuple[str, str], dict[str, Any]] = {}

    for measurement in report["measurements"]:
        if measurement.get("metric") != "repo_temporal_holdout":
            continue
        for case in measurement.get("cases", []):
            expected = set(case.get(expected_key, []))
            if not expected:
                continue
            methods = case.get("methods", {})
            method_summary = methods.get(method)
            oracle_summary = methods.get(oracle_method)
            if not method_summary or not oracle_summary:
                continue
            method_top = [str(path) for path in method_summary.get("top", [])]
            oracle_top = [str(path) for path in oracle_summary.get("top", [])]
            if retarget_metrics:
                method_ap = average_precision_at_k(method_top, expected, k)
                oracle_ap = average_precision_at_k(oracle_top, expected, k)
            else:
                method_ap = float(method_summary[metric])
                oracle_ap = float(oracle_summary[metric])
            gap = round(oracle_ap - method_ap, 3)
            if gap <= 0.0:
                continue

            repo = repo_label(str(case.get("repo", measurement.get("repo", ""))))
            commit = short_commit(case.get("heldout_commit"))
            cluster = clusters.setdefault(
                (repo, commit),
                {
                    "repo": repo,
                    "commit": commit,
                    "case_count": 0,
                    "seeds": set(),
                    "target_count": 0,
                    "gap": 0.0,
                    "method_ap_sum": 0.0,
                    "oracle_ap_sum": 0.0,
                    "cases": [],
                },
            )
            hits = set(hits_at_k(method_top, expected, k))
            missing = sorted(expected - hits)
            false_positives = [path for path in method_top[:k] if path not in expected]
            predictable_expected = set(case.get("predictable_expected", []))
            unpredictable_expected = set(case.get("unpredictable_expected", []))
            if expected_key == "predictable_expected":
                missing_predictable = missing
                missing_unpredictable = []
            elif expected_key == "unpredictable_expected":
                missing_predictable = []
                missing_unpredictable = missing
            else:
                missing_predictable = sorted(
                    path for path in missing if path in predictable_expected
                )
                missing_unpredictable = sorted(
                    path for path in missing if path in unpredictable_expected
                )
            cluster["case_count"] += 1
            cluster["seeds"].add(str(case.get("seed", "")))
            cluster["target_count"] += len(expected)
            cluster["gap"] += gap
            cluster["method_ap_sum"] += method_ap
            cluster["oracle_ap_sum"] += oracle_ap
            cluster["cases"].append(
                {
                    "seed": str(case.get("seed", "")),
                    "gap": gap,
                    "missing": missing,
                    "missing_predictable": missing_predictable,
                    "missing_unpredictable": missing_unpredictable,
                    "false_positives": false_positives,
                    "method_top": method_top[:k],
                }
            )

    rows = []
    for cluster in clusters.values():
        case_count = int(cluster["case_count"])
        if case_count == 0:
            continue
        cases = sorted(
            cluster["cases"],
            key=lambda case: (-case["gap"], case["seed"]),
        )
        top_case = cases[0]
        rows.append(
            {
                "repo": cluster["repo"],
                "commit": cluster["commit"],
                "case_count": case_count,
                "seed_count": len(cluster["seeds"]),
                "target_count": cluster["target_count"],
                "gap": round(cluster["gap"], 3),
                "mean_gap": round(cluster["gap"] / case_count, 3),
                "mean_method_ap": round(cluster["method_ap_sum"] / case_count, 3),
                "mean_oracle_ap": round(cluster["oracle_ap_sum"] / case_count, 3),
                "top_seed": top_case["seed"],
                "top_missing": top_case["missing"],
                "top_missing_predictable": top_case["missing_predictable"],
                "top_missing_unpredictable": top_case["missing_unpredictable"],
                "top_false_positives": top_case["false_positives"],
                "top_method_top": top_case["method_top"],
            }
        )
    return sorted(
        rows,
        key=lambda row: (-row["gap"], row["repo"], row["commit"]),
    )[:limit]


def render_residual_gap_cluster_table(
    report: dict[str, Any],
    title: str,
    *,
    expected_key: str = "expected",
    retarget_metrics: bool = False,
    limit: int = 8,
) -> str:
    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if holdout is None:
        return ""
    k = holdout["k"]
    entries = residual_gap_cluster_entries(
        report,
        expected_key=expected_key,
        retarget_metrics=retarget_metrics,
        limit=limit,
    )
    if not entries:
        return ""
    rows = [
        [
            entry["repo"],
            entry["commit"],
            str(entry["case_count"]),
            str(entry["seed_count"]),
            str(entry["target_count"]),
            fmt_number(entry["gap"]),
            fmt_number(entry["mean_gap"]),
            fmt_number(entry["mean_method_ap"]),
            fmt_number(entry["mean_oracle_ap"]),
            entry["top_seed"],
            fmt_path_list(entry["top_missing"], limit=3),
            fmt_path_list(entry["top_missing_predictable"], limit=3),
            fmt_path_list(entry["top_missing_unpredictable"], limit=3),
            fmt_path_list(entry["top_false_positives"], limit=3),
            fmt_path_list(entry["top_method_top"], limit=3),
        ]
        for entry in entries
    ]
    return "\n".join(
        [
            f"## {title} Residual Gap Clusters @{k}",
            markdown_table(
                [
                    "repo",
                    "commit",
                    "cases",
                    "seeds",
                    "targets",
                    "oracle gap",
                    "mean gap",
                    "hybrid AP",
                    "oracle AP",
                    "top seed",
                    "missing targets",
                    "missing predictable",
                    "missing new",
                    "top non-targets",
                    "hybrid top",
                ],
                rows,
            ),
        ]
    )


def render_holdout_dataset_table(
    report: dict[str, Any],
    aggregate: dict[str, Any],
) -> str:
    rows = []

    def add_row(scope: str, repo_count: str, dataset: dict[str, Any]) -> None:
        rows.append(
            [
                scope,
                repo_count,
                str(dataset["candidate_commit_count"]),
                str(dataset["examined_commit_count"]),
                str(dataset["heldout_commit_count"]),
                str(dataset["case_count"]),
                str(dataset["target_count"]),
                str(dataset["predictable_case_count"]),
                str(dataset["predictable_target_count"]),
                str(dataset["unpredictable_target_count"]),
                fmt_distribution(dataset["target_count_distribution"]),
                fmt_distribution(dataset["predictable_target_count_distribution"]),
                fmt_skipped(dataset["skipped"]),
            ]
        )

    if "dataset" in aggregate:
        add_row("cross-repo", str(aggregate["repo_count"]), aggregate["dataset"])
    for holdout in report["measurements"]:
        if holdout["metric"] == "repo_temporal_holdout" and "dataset" in holdout:
            add_row(repo_label(holdout["repo"]), "1", holdout["dataset"])

    if not rows:
        return ""
    return "\n".join(
        [
            "## Temporal Holdout Dataset",
            markdown_table(
                [
                    "scope",
                    "repos",
                    "candidates",
                    "examined",
                    "heldout",
                    "cases",
                    "targets",
                    "predictable cases",
                    "predictable targets",
                    "unpredictable targets",
                    "targets/case",
                    "predictable/case",
                    "skipped",
                ],
                rows,
            ),
        ]
    )


def render_temporal_leakage_audit_table(
    report: dict[str, Any],
    aggregate: dict[str, Any],
) -> str:
    rows = []

    def add_row(scope: str, repo_count: str, audit: dict[str, Any]) -> None:
        rows.append(
            [
                scope,
                repo_count,
                str(audit.get("case_count", "")),
                str(audit.get("checked_case_count", "")),
                str(audit.get("head_matches_parent_count", "")),
                str(audit.get("failure_count", "")),
                str(audit.get("omitted_failures", "")),
            ]
        )

    aggregate_audit = aggregate.get("temporal_leakage_audit")
    if isinstance(aggregate_audit, dict):
        add_row("cross-repo", str(aggregate["repo_count"]), aggregate_audit)
    for holdout in report["measurements"]:
        if holdout["metric"] != "repo_temporal_holdout":
            continue
        audit = holdout.get("temporal_leakage_audit")
        if isinstance(audit, dict):
            add_row(repo_label(holdout["repo"]), "1", audit)

    if not rows:
        return ""
    return "\n".join(
        [
            "## Temporal Holdout Leakage Audit",
            markdown_table(
                [
                    "scope",
                    "repos",
                    "cases",
                    "checked",
                    "index head = parent",
                    "failures",
                    "omitted failures",
                ],
                rows,
            ),
        ]
    )


def render_loro_weight_selection_table(
    measurement: dict[str, Any],
    title: str,
) -> str:
    rows = []
    k = measurement["k"]
    for selection in measurement.get("selections", []):
        rows.append(
            [
                repo_label(selection["repo"]),
                fmt_number(selection["selected_hybrid_direct_weight"], 3),
                str(selection["train_case_count"]),
                str(selection["test_case_count"]),
                str(selection["test_target_count"]),
                fmt_number(selection[f"train_average_precision_at_{k}"]),
                fmt_number(selection[f"test_average_precision_at_{k}"]),
                fmt_number(selection[f"test_ndcg_at_{k}"]),
            ]
        )
    if not rows:
        return ""
    return "\n".join(
        [
            f"## {title} Leave-One-Repo-Out Weight Selection @{k}",
            markdown_table(
                [
                    "test repo",
                    "selected weight",
                    "train cases",
                    "test cases",
                    "test targets",
                    "train AP",
                    "test AP",
                    "test nDCG",
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
    metadata = render_metadata_table(report)
    if metadata:
        sections.append(metadata)

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
        dataset = render_holdout_dataset_table(report, holdout)
        if dataset:
            sections.append(dataset)
        leakage_audit = render_temporal_leakage_audit_table(report, holdout)
        if leakage_audit:
            sections.append(leakage_audit)
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
                        HOLDOUT_RELATED_METHODS,
                        RELATED_COMPARISONS[:2],
                    )
                ],
            )
        )
        oracle_normalized = render_oracle_normalized_table(
            holdout,
            "Cross-Repo Temporal Holdout",
            [
                "baseline_path_locality",
                "baseline_lexical_similarity",
                "baseline_content_similarity",
                "baseline_recent_activity",
                "baseline_global_pagerank",
                "workspace_related_direct",
                "workspace_related_pagerank",
                "workspace_related_hybrid",
            ],
        )
        if oracle_normalized:
            sections.append(oracle_normalized)
        residual_clusters = render_residual_gap_cluster_table(
            report,
            "Cross-Repo Temporal Holdout",
        )
        if residual_clusters:
            sections.append(residual_clusters)
        case_deltas = render_case_delta_table(
            report,
            "Cross-Repo Temporal Holdout",
        )
        if case_deltas:
            sections.append(case_deltas)
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
        loro = holdout.get("leave_one_repo_out_weight_selection")
        if loro and loro.get("case_count", 0) > 0:
            loro_selection = render_loro_weight_selection_table(
                loro,
                "Cross-Repo Temporal Holdout",
            )
            if loro_selection:
                sections.append(loro_selection)
            sections.append(
                render_aggregate_table(
                    loro,
                    "Cross-Repo Temporal Holdout LORO Selected",
                )
            )
            sections.append(
                render_delta_table(
                    loro,
                    "Cross-Repo Temporal Holdout LORO Selected",
                    RELATED_LORO_COMPARISONS,
                )
            )

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
                            HOLDOUT_RELATED_METHODS,
                            RELATED_COMPARISONS[:2],
                        )
                    ],
                )
            )
            predictable_oracle_normalized = render_oracle_normalized_table(
                predictable,
                "Predictable Cross-Repo Temporal Holdout",
                [
                    "baseline_path_locality",
                    "baseline_lexical_similarity",
                    "baseline_content_similarity",
                    "baseline_recent_activity",
                    "baseline_global_pagerank",
                    "workspace_related_direct",
                    "workspace_related_pagerank",
                    "workspace_related_hybrid",
                ],
            )
            if predictable_oracle_normalized:
                sections.append(predictable_oracle_normalized)
            predictable_residual_clusters = render_residual_gap_cluster_table(
                report,
                "Predictable Cross-Repo Temporal Holdout",
                expected_key="predictable_expected",
                retarget_metrics=True,
            )
            if predictable_residual_clusters:
                sections.append(predictable_residual_clusters)
            predictable_case_deltas = render_case_delta_table(
                report,
                "Predictable Cross-Repo Temporal Holdout",
                expected_key="predictable_expected",
            )
            if predictable_case_deltas:
                sections.append(predictable_case_deltas)
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
            predictable_loro = predictable.get(
                "leave_one_repo_out_weight_selection"
            )
            if predictable_loro and predictable_loro.get("case_count", 0) > 0:
                predictable_loro_selection = render_loro_weight_selection_table(
                    predictable_loro,
                    "Predictable Cross-Repo Temporal Holdout",
                )
                if predictable_loro_selection:
                    sections.append(predictable_loro_selection)
                sections.append(
                    render_aggregate_table(
                        predictable_loro,
                        "Predictable Cross-Repo Temporal Holdout LORO Selected",
                    )
                )
                sections.append(
                    render_delta_table(
                        predictable_loro,
                        "Predictable Cross-Repo Temporal Holdout LORO Selected",
                        RELATED_LORO_COMPARISONS,
                    )
                )

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
