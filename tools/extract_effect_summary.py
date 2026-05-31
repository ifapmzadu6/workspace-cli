#!/usr/bin/env python3
"""Extract paper-facing headline effect metrics from a measurement report."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1


def load_report(path: str) -> dict[str, Any]:
    if path == "-":
        return json.load(sys.stdin)
    return json.loads(Path(path).read_text())


def measurement_by_name(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    for measurement in report.get("measurements", []):
        if measurement.get("metric") == name:
            return measurement
    return None


def measurements_by_name(report: dict[str, Any], name: str) -> list[dict[str, Any]]:
    return [
        measurement
        for measurement in report.get("measurements", [])
        if measurement.get("metric") == name
    ]


def repo_name(repo: Any) -> str | None:
    if not isinstance(repo, str) or not repo:
        return None
    return Path(repo).name or repo


def rounded(value: Any) -> float | None:
    if value is None:
        return None
    return round(float(value), 6)


def metric_mean(summary: dict[str, Any], metric: str) -> dict[str, float] | None:
    mean_key = f"mean_{metric}"
    if mean_key not in summary:
        return None
    result = {"mean": rounded(summary[mean_key])}
    low_key = f"ci95_low_{metric}"
    high_key = f"ci95_high_{metric}"
    if low_key in summary and high_key in summary:
        result["ci95_low"] = rounded(summary[low_key])
        result["ci95_high"] = rounded(summary[high_key])
    return result


def method_metrics(
    measurement: dict[str, Any],
    method: str,
    *,
    metrics: list[str] | None = None,
) -> dict[str, Any]:
    k = measurement.get("k", 5)
    metrics = metrics or [
        f"recall_at_{k}",
        f"average_precision_at_{k}",
        f"ndcg_at_{k}",
    ]
    summary = measurement.get("aggregate", {}).get(method)
    if not isinstance(summary, dict):
        return {}
    return {
        metric: value
        for metric in metrics
        if (value := metric_mean(summary, metric)) is not None
    }


def delta_metrics(
    measurement: dict[str, Any],
    comparison: str,
    *,
    metric: str | None = None,
) -> dict[str, Any]:
    k = measurement.get("k", 5)
    metric = metric or f"average_precision_at_{k}"
    summary = measurement.get("paired_deltas", {}).get(comparison)
    if not isinstance(summary, dict):
        return {}
    mean_key = f"mean_delta_{metric}"
    if mean_key not in summary:
        return {}
    result: dict[str, Any] = {
        "mean_delta": rounded(summary[mean_key]),
    }
    for source, target in [
        (f"ci95_low_delta_{metric}", "ci95_low"),
        (f"ci95_high_delta_{metric}", "ci95_high"),
        (f"p_greater_delta_{metric}", "p_greater"),
        (f"p_greater_holm_delta_{metric}", "p_greater_holm"),
        (f"win_count_delta_{metric}", "wins"),
        (f"tie_count_delta_{metric}", "ties"),
        (f"loss_count_delta_{metric}", "losses"),
    ]:
        if source not in summary:
            continue
        value = summary[source]
        result[target] = (
            int(value) if target in {"wins", "ties", "losses"} else rounded(value)
        )
    return result


def oracle_normalized_metrics(
    measurement: dict[str, Any],
    methods: list[str],
) -> dict[str, Any]:
    k = measurement.get("k", 5)
    metric = f"average_precision_at_{k}"
    aggregate = measurement.get("aggregate", {})
    if not isinstance(aggregate, dict):
        return {}
    oracle = aggregate.get("history_oracle_ceiling", {})
    if not isinstance(oracle, dict):
        return {}
    oracle_mean = oracle.get(f"mean_{metric}")
    if oracle_mean is None or float(oracle_mean) <= 0.0:
        return {}

    result = {}
    for method in methods:
        summary = aggregate.get(method)
        if not isinstance(summary, dict):
            continue
        method_mean = summary.get(f"mean_{metric}")
        if method_mean is None:
            continue
        result[method] = {
            metric: rounded(method_mean),
            f"oracle_{metric}": rounded(oracle_mean),
            f"oracle_normalized_{metric}": rounded(
                float(method_mean) / float(oracle_mean)
            ),
            f"oracle_gap_{metric}": rounded(float(oracle_mean) - float(method_mean)),
        }
    return result


def best_weight_sweep(measurement: dict[str, Any], group: str) -> dict[str, Any]:
    k = measurement.get("k", 5)
    metric = f"mean_average_precision_at_{k}"
    best: dict[str, Any] | None = None
    for entry in measurement.get("hybrid_weight_sweep", []):
        group_data = entry.get(group)
        if not isinstance(group_data, dict):
            continue
        method = group_data.get("method")
        aggregate = group_data.get("aggregate", {})
        if not isinstance(method, str) or method not in aggregate:
            continue
        summary = aggregate[method]
        if metric not in summary:
            continue
        candidate = {
            "direct_weight": rounded(entry.get("hybrid_direct_weight")),
            f"average_precision_at_{k}": rounded(summary[metric]),
            "method": method,
        }
        if (
            best is None
            or candidate[f"average_precision_at_{k}"]
            > best[f"average_precision_at_{k}"]
        ):
            best = candidate
    return best or {}


def weight_sweep_summary(
    measurement: dict[str, Any],
    group: str,
    *,
    direct_method: str,
    pagerank_method: str,
) -> list[dict[str, Any]]:
    k = measurement.get("k", 5)
    ap_metric = f"average_precision_at_{k}"
    ndcg_metric = f"ndcg_at_{k}"
    rows = []
    for entry in measurement.get("hybrid_weight_sweep", []):
        group_data = entry.get(group)
        if not isinstance(group_data, dict):
            continue
        method = group_data.get("method")
        aggregate = group_data.get("aggregate", {})
        if not isinstance(method, str) or method not in aggregate:
            continue
        summary = aggregate[method]
        if f"mean_{ap_metric}" not in summary:
            continue
        row: dict[str, Any] = {
            "direct_weight": rounded(entry.get("hybrid_direct_weight")),
            "method": method,
            ap_metric: rounded(summary[f"mean_{ap_metric}"]),
        }
        if f"mean_{ndcg_metric}" in summary:
            row[ndcg_metric] = rounded(summary[f"mean_{ndcg_metric}"])

        deltas = group_data.get("paired_deltas", {})
        for label, baseline in [
            ("delta_vs_direct", direct_method),
            ("delta_vs_pagerank", pagerank_method),
        ]:
            comparison = f"{method}_minus_{baseline}"
            if isinstance(deltas, dict) and comparison in deltas:
                delta = delta_metrics(
                    {"k": k, "paired_deltas": deltas},
                    comparison,
                    metric=ap_metric,
                )
                if delta:
                    row[label] = delta
        rows.append(row)
    return rows


def headline_retrieval_summary(report: dict[str, Any]) -> dict[str, Any]:
    retrieval = measurement_by_name(report, "retrieval_suite")
    if not retrieval:
        return {}
    return {
        "k": retrieval.get("k"),
        "scenario_count": retrieval.get("scenario_count"),
        "methods": {
            method: method_metrics(retrieval, method)
            for method in [
                "workspace_related_hybrid",
                "workspace_impact_hybrid",
                "workspace_related_direct",
                "workspace_impact_direct",
                "baseline_path_locality",
                "baseline_lexical_similarity",
                "baseline_content_similarity",
                "baseline_recent_activity",
                "baseline_global_pagerank",
            ]
        },
        "key_deltas": {
            comparison: delta_metrics(retrieval, comparison)
            for comparison in [
                "workspace_related_hybrid_minus_workspace_related_direct",
                "workspace_related_hybrid_minus_baseline_content_similarity",
                "workspace_related_hybrid_minus_baseline_recent_activity",
                "workspace_impact_hybrid_minus_workspace_impact_direct",
                "workspace_impact_hybrid_minus_baseline_content_similarity",
            ]
        },
    }


def headline_holdout_summary(holdout: dict[str, Any]) -> dict[str, Any]:
    if not holdout:
        return {}
    k = holdout.get("k", 5)
    macro = holdout.get("repo_macro_average", {})
    loro = holdout.get("leave_one_repo_out_weight_selection", {})
    result = {
        "k": k,
        "repo_count": holdout.get("repo_count"),
        "case_count": holdout.get("case_count"),
        "target_count": holdout.get("target_count"),
        "heldout_commit_count": holdout.get("heldout_commit_count"),
        "temporal_leakage_audit": holdout.get("temporal_leakage_audit", {}),
        "methods": {
            method: method_metrics(holdout, method)
            for method in [
                "workspace_related_hybrid",
                "workspace_related_direct",
                "workspace_related_pagerank",
                "baseline_path_locality",
                "baseline_lexical_similarity",
                "baseline_content_similarity",
                "baseline_recent_activity",
                "baseline_global_pagerank",
                "history_oracle_ceiling",
            ]
        },
        "repo_macro_methods": {
            method: method_metrics(
                macro,
                method,
                metrics=[f"average_precision_at_{k}"],
            )
            for method in [
                "workspace_related_hybrid",
                "workspace_related_direct",
                "workspace_related_pagerank",
                "baseline_path_locality",
                "baseline_content_similarity",
                "baseline_recent_activity",
                "baseline_global_pagerank",
            ]
        },
        "loro_methods": {
            "workspace_related_hybrid_loro": method_metrics(
                loro,
                "workspace_related_hybrid_loro",
            )
        },
        "key_deltas": {
            comparison: delta_metrics(holdout, comparison)
            for comparison in [
                "workspace_related_hybrid_minus_workspace_related_direct",
                "workspace_related_hybrid_minus_workspace_related_pagerank",
                "workspace_related_hybrid_minus_baseline_path_locality",
                "workspace_related_hybrid_minus_baseline_content_similarity",
                "workspace_related_hybrid_minus_baseline_recent_activity",
                "workspace_related_hybrid_minus_baseline_global_pagerank",
            ]
        },
        "oracle_normalized": oracle_normalized_metrics(
            holdout,
            [
                "workspace_related_hybrid",
                "workspace_related_direct",
                "workspace_related_pagerank",
                "baseline_path_locality",
                "baseline_content_similarity",
                "baseline_recent_activity",
                "baseline_global_pagerank",
            ],
        ),
        "best_weight_sweep": best_weight_sweep(holdout, "related"),
        "weight_sweep": weight_sweep_summary(
            holdout,
            "related",
            direct_method="workspace_related_direct",
            pagerank_method="workspace_related_pagerank",
        ),
    }
    predictable = holdout.get("predictable_only")
    if isinstance(predictable, dict):
        result["predictable_only"] = headline_holdout_summary(predictable)
    return result


def extract_summary(report: dict[str, Any]) -> dict[str, Any]:
    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    map_recall = measurement_by_name(report, "map_fact_recall") or {}
    transaction = measurement_by_name(report, "transaction_audit_signal_recall") or {}
    holdout_summary = headline_holdout_summary(holdout or {})
    if holdout:
        holdout_summary["per_repo"] = []
        for measurement in measurements_by_name(report, "repo_temporal_holdout"):
            repo_summary = headline_holdout_summary(measurement)
            repo_summary["repo"] = measurement.get("repo")
            repo_summary["repo_name"] = repo_name(measurement.get("repo"))
            holdout_summary["per_repo"].append(repo_summary)
    return {
        "schema_version": SCHEMA_VERSION,
        "metadata": report.get("metadata", {}),
        "observation_recall": {
            "map_fact_recall": rounded(map_recall.get("recall")),
            "transaction_audit_signal_recall": rounded(transaction.get("recall")),
        },
        "retrieval_suite": headline_retrieval_summary(report),
        "repo_temporal_holdout": holdout_summary,
    }


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
    print(
        json.dumps(
            extract_summary(load_report(args.report)),
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
