#!/usr/bin/env python3
"""Extract paper-facing headline effect metrics from a measurement report."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


TOOLS_DIR = Path(__file__).resolve().parent
if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import check_effect_thresholds  # noqa: E402


SCHEMA_VERSION = 3


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


def average_precision_at_k(top: list[str], expected: list[str], k: int) -> float | None:
    expected_set = set(expected)
    if not expected_set:
        return None
    hits = 0
    precision_sum = 0.0
    seen = set()
    for rank, path in enumerate(top[:k], start=1):
        if path in seen:
            continue
        seen.add(path)
        if path in expected_set:
            hits += 1
            precision_sum += hits / rank
    return round(precision_sum / len(expected_set), 3)


def is_json_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def diagnostic_ranked_candidates(
    case: dict[str, Any],
    method: str,
) -> list[dict[str, Any]]:
    diagnostics = case.get("diagnostics")
    if not isinstance(diagnostics, dict):
        return []
    method_diagnostics = diagnostics.get(method)
    if not isinstance(method_diagnostics, dict):
        return []
    candidates = method_diagnostics.get("ranked_candidates")
    if not isinstance(candidates, list):
        return []

    result = []
    seen: set[str] = set()
    for entry in candidates:
        if not isinstance(entry, dict):
            continue
        path = entry.get("path")
        rank = entry.get("rank")
        if not isinstance(path, str) or path in seen:
            continue
        if not isinstance(rank, int) or isinstance(rank, bool):
            continue
        seen.add(path)
        candidate: dict[str, Any] = {"path": path, "rank": rank}
        score = entry.get("score")
        if is_json_number(score):
            candidate["score"] = rounded(score)
        result.append(candidate)
    return result


def missing_expected_rank_diagnostics(
    case: dict[str, Any],
    method: str,
    missing_expected: list[str],
) -> list[dict[str, Any]]:
    diagnostics = case.get("diagnostics")
    if not isinstance(diagnostics, dict):
        return []
    method_diagnostics = diagnostics.get(method)
    if not isinstance(method_diagnostics, dict):
        return []
    ranks = method_diagnostics.get("missing_expected_ranks")
    if not isinstance(ranks, list):
        return []

    missing_set = set(missing_expected)
    score_by_path = {
        entry["path"]: entry["score"]
        for entry in diagnostic_ranked_candidates(case, method)
        if "score" in entry
    }
    result = []
    for entry in ranks:
        if not isinstance(entry, dict):
            continue
        path = entry.get("path")
        rank = entry.get("rank")
        if not isinstance(path, str) or path not in missing_set:
            continue
        if rank is not None and (not isinstance(rank, int) or isinstance(rank, bool)):
            continue
        result_entry: dict[str, Any] = {"path": path, "rank": rank}
        if path in score_by_path:
            result_entry["score"] = score_by_path[path]
        result.append(result_entry)
    return result


def residual_gap_clusters(
    measurements: list[dict[str, Any]],
    *,
    method: str = "workspace_related_hybrid",
    oracle_method: str = "history_oracle_ceiling",
    expected_key: str = "expected",
    retarget_metrics: bool = False,
    limit: int = 8,
    case_limit: int = 5,
) -> list[dict[str, Any]]:
    clusters: dict[tuple[str, str], dict[str, Any]] = {}
    for measurement in measurements:
        k = measurement.get("k", 5)
        metric = f"average_precision_at_{k}"
        cases = measurement.get("cases", [])
        if not isinstance(cases, list):
            continue
        for case in cases:
            if not isinstance(case, dict):
                continue
            methods = case.get("methods", {})
            if not isinstance(methods, dict):
                continue
            method_summary = methods.get(method)
            oracle_summary = methods.get(oracle_method)
            if not isinstance(method_summary, dict) or not isinstance(
                oracle_summary,
                dict,
            ):
                continue
            method_ap = method_summary.get(metric)
            oracle_ap = oracle_summary.get(metric)
            method_top = [
                path for path in method_summary.get("top", []) if isinstance(path, str)
            ]
            oracle_top = [
                path for path in oracle_summary.get("top", []) if isinstance(path, str)
            ]

            repo = case.get("repo") or measurement.get("repo")
            commit = case.get("heldout_commit")
            if not isinstance(commit, str) or not commit:
                continue
            repo_key = repo if isinstance(repo, str) else ""
            key = (repo_key, commit)
            cluster = clusters.setdefault(
                key,
                {
                    "repo": repo if isinstance(repo, str) else None,
                    "repo_name": repo_name(repo),
                    "heldout_commit": commit,
                    "k": k,
                    "metric": metric,
                    "method": method,
                    "oracle_method": oracle_method,
                    "expected_key": expected_key,
                    "retarget_metrics": retarget_metrics,
                    "_gap_sum": 0.0,
                    "_method_ap_sum": 0.0,
                    "_oracle_ap_sum": 0.0,
                    "_seed_set": set(),
                    "_target_count": 0,
                    "_predictable_target_count": 0,
                    "_unpredictable_target_count": 0,
                    "_cases": [],
                },
            )

            expected = [
                path for path in case.get(expected_key, []) if isinstance(path, str)
            ]
            if not expected:
                continue
            if retarget_metrics:
                computed_method_ap = average_precision_at_k(method_top, expected, k)
                computed_oracle_ap = average_precision_at_k(oracle_top, expected, k)
                if computed_method_ap is None or computed_oracle_ap is None:
                    continue
                method_ap = computed_method_ap
                oracle_ap = computed_oracle_ap
            elif method_ap is None or oracle_ap is None:
                continue
            gap = float(oracle_ap) - float(method_ap)
            if gap <= 0.0:
                continue

            predictable_expected = [
                path
                for path in case.get("predictable_expected", [])
                if isinstance(path, str)
            ]
            unpredictable_expected = [
                path
                for path in case.get("unpredictable_expected", [])
                if isinstance(path, str)
            ]
            expected_set = set(expected)
            hits = [path for path in method_top[:k] if path in expected_set]
            if expected_key == "predictable_expected":
                predictable_target_count = len(expected)
                unpredictable_target_count = 0
            elif expected_key == "unpredictable_expected":
                predictable_target_count = 0
                unpredictable_target_count = len(expected)
            else:
                predictable_target_count = len(predictable_expected)
                unpredictable_target_count = len(unpredictable_expected)
            hit_set = set(hits)
            missing_expected = [path for path in expected if path not in hit_set]
            method_false_positives = [
                path for path in method_top[:k] if path not in expected_set
            ]
            method_top_ranked = diagnostic_ranked_candidates(case, method)[:k]
            if expected_key == "predictable_expected":
                missing_predictable_expected = missing_expected
                missing_unpredictable_expected = []
            elif expected_key == "unpredictable_expected":
                missing_predictable_expected = []
                missing_unpredictable_expected = missing_expected
            else:
                predictable_expected_set = set(predictable_expected)
                unpredictable_expected_set = set(unpredictable_expected)
                missing_predictable_expected = [
                    path for path in missing_expected if path in predictable_expected_set
                ]
                missing_unpredictable_expected = [
                    path for path in missing_expected if path in unpredictable_expected_set
                ]
            seed = case.get("seed")
            if isinstance(seed, str):
                cluster["_seed_set"].add(seed)

            cluster["_gap_sum"] += gap
            cluster["_method_ap_sum"] += float(method_ap)
            cluster["_oracle_ap_sum"] += float(oracle_ap)
            cluster["_target_count"] += len(expected)
            cluster["_predictable_target_count"] += predictable_target_count
            cluster["_unpredictable_target_count"] += unpredictable_target_count
            cluster["_cases"].append(
                {
                    "seed": seed,
                    f"oracle_gap_{metric}": rounded(gap),
                    f"method_{metric}": rounded(method_ap),
                    f"oracle_{metric}": rounded(oracle_ap),
                    "expected_count": len(expected),
                    "missing_expected": missing_expected,
                    "missing_expected_ranks": missing_expected_rank_diagnostics(
                        case,
                        method,
                        missing_expected,
                    ),
                    "missing_predictable_expected": missing_predictable_expected,
                    "missing_unpredictable_expected": missing_unpredictable_expected,
                    "method_hits": hits,
                    "method_false_positives": method_false_positives,
                    "method_top": method_top,
                    "method_top_ranked": method_top_ranked,
                },
            )

    rows = []
    for cluster in clusters.values():
        cases = sorted(
            cluster["_cases"],
            key=lambda case: (
                -float(case[f"oracle_gap_{cluster['metric']}"]),
                str(case.get("seed") or ""),
            ),
        )
        case_count = len(cases)
        if case_count == 0:
            continue
        metric = cluster["metric"]
        row = {
            "repo": cluster["repo"],
            "repo_name": cluster["repo_name"],
            "heldout_commit": cluster["heldout_commit"],
            "k": cluster["k"],
            "metric": metric,
            "method": cluster["method"],
            "oracle_method": cluster["oracle_method"],
            "expected_key": cluster["expected_key"],
            "retarget_metrics": cluster["retarget_metrics"],
            "case_count": case_count,
            "seed_count": len(cluster["_seed_set"]),
            "target_count": cluster["_target_count"],
            "predictable_target_count": cluster["_predictable_target_count"],
            "unpredictable_target_count": cluster["_unpredictable_target_count"],
            f"oracle_gap_{metric}": rounded(cluster["_gap_sum"]),
            f"mean_oracle_gap_{metric}": rounded(
                cluster["_gap_sum"] / case_count
            ),
            f"mean_method_{metric}": rounded(
                cluster["_method_ap_sum"] / case_count
            ),
            f"mean_oracle_{metric}": rounded(
                cluster["_oracle_ap_sum"] / case_count
            ),
            "top_residual_cases": cases[:case_limit],
        }
        rows.append(row)

    return sorted(
        rows,
        key=lambda row: (
            -float(row[f"oracle_gap_{row['metric']}"]),
            str(row.get("repo_name") or ""),
            str(row.get("heldout_commit") or ""),
        ),
    )[:limit]


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
    clusters = residual_gap_clusters([holdout])
    if clusters:
        result["residual_gap_clusters"] = clusters
    predictable = holdout.get("predictable_only")
    if isinstance(predictable, dict):
        result["predictable_only"] = headline_holdout_summary(predictable)
    return result


def extract_summary(report: dict[str, Any]) -> dict[str, Any]:
    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    map_recall = measurement_by_name(report, "map_fact_recall") or {}
    transaction = measurement_by_name(report, "transaction_audit_signal_recall") or {}
    holdout_summary = headline_holdout_summary(holdout or {})
    repo_measurements = measurements_by_name(report, "repo_temporal_holdout")
    if holdout:
        holdout_summary["per_repo"] = []
        for measurement in repo_measurements:
            repo_summary = headline_holdout_summary(measurement)
            repo_summary["repo"] = measurement.get("repo")
            repo_summary["repo_name"] = repo_name(measurement.get("repo"))
            predictable = repo_summary.get("predictable_only")
            if isinstance(predictable, dict):
                clusters = residual_gap_clusters(
                    [measurement],
                    expected_key="predictable_expected",
                    retarget_metrics=True,
                )
                if clusters:
                    predictable["residual_gap_clusters"] = clusters
            holdout_summary["per_repo"].append(repo_summary)
        clusters = residual_gap_clusters(repo_measurements)
        if clusters:
            holdout_summary["residual_gap_clusters"] = clusters
        predictable = holdout_summary.get("predictable_only")
        if isinstance(predictable, dict):
            clusters = residual_gap_clusters(
                repo_measurements,
                expected_key="predictable_expected",
                retarget_metrics=True,
            )
            if clusters:
                predictable["residual_gap_clusters"] = clusters
    return {
        "schema_version": SCHEMA_VERSION,
        "metadata": report.get("metadata", {}),
        "observation_recall": {
            "map_fact_recall": rounded(map_recall.get("recall")),
            "transaction_audit_signal_recall": rounded(transaction.get("recall")),
        },
        "retrieval_suite": headline_retrieval_summary(report),
        "repo_temporal_holdout": holdout_summary,
        "threshold_margins": check_effect_thresholds.threshold_margin_entries(
            report,
            require_holdout=holdout is not None,
        ),
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
