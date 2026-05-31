#!/usr/bin/env python3
"""Measure workspace-cli effects against small reproducible fixtures.

This is not a correctness smoke test. It measures whether the CLI improves
workspace observability compared with narrow baseline signals such as the
current git diff or direct co-change only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import re
import subprocess
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
BOOTSTRAP_SAMPLES = 1000
SIGN_FLIP_SAMPLES = 10000
SIGN_FLIP_EXACT_SCALE = 1000
SIGN_FLIP_EXACT_MAX_STATES = 200_001
HISTORY_ORACLE_METHOD = "history_oracle_ceiling"
DEFAULT_CUTOFF_SWEEP = [1, 3, 5]
RECENT_ACTIVITY_MAX_COMMITS = 500
GLOBAL_PAGERANK_ITERATIONS = 50
CONTENT_SIMILARITY_MAX_BYTES = 200_000
RELATED_HYBRID_DEFAULT_DIRECT_WEIGHT = 0.9
RELATED_HYBRID_LORO_METHOD = "workspace_related_hybrid_loro"
LORO_WEIGHT_SELECTION_AP_TOLERANCE = 0.002
LORO_WEIGHT_SELECTION_NDCG_TOLERANCE = 0.002
RELATED_DIAGNOSTIC_MAX_RESULTS = 20
RELATED_COMPARISON_PAIRS = [
    ("workspace_related_hybrid", "workspace_related_direct"),
    ("workspace_related_hybrid", "workspace_related_pagerank"),
    ("workspace_related_hybrid", "baseline_path_locality"),
    ("workspace_related_hybrid", "baseline_lexical_similarity"),
    ("workspace_related_hybrid", "baseline_content_similarity"),
    ("workspace_related_hybrid", "baseline_recent_activity"),
    ("workspace_related_hybrid", "baseline_global_pagerank"),
    ("workspace_related_pagerank", "workspace_related_direct"),
]
RELATED_LORO_COMPARISON_PAIRS = [
    (RELATED_HYBRID_LORO_METHOD, "workspace_related_direct"),
    (RELATED_HYBRID_LORO_METHOD, "workspace_related_pagerank"),
    (RELATED_HYBRID_LORO_METHOD, "baseline_path_locality"),
    (RELATED_HYBRID_LORO_METHOD, "baseline_lexical_similarity"),
    (RELATED_HYBRID_LORO_METHOD, "baseline_content_similarity"),
    (RELATED_HYBRID_LORO_METHOD, "baseline_recent_activity"),
    (RELATED_HYBRID_LORO_METHOD, "baseline_global_pagerank"),
    (RELATED_HYBRID_LORO_METHOD, "workspace_related_hybrid"),
]
IMPACT_COMPARISON_PAIRS = [
    ("workspace_impact_hybrid", "workspace_impact_direct"),
    ("workspace_impact_hybrid", "workspace_impact_pagerank"),
    ("workspace_impact_hybrid", "baseline_path_locality"),
    ("workspace_impact_hybrid", "baseline_lexical_similarity"),
    ("workspace_impact_hybrid", "baseline_content_similarity"),
    ("workspace_impact_hybrid", "baseline_recent_activity"),
    ("workspace_impact_hybrid", "baseline_global_pagerank"),
    ("workspace_impact_pagerank", "workspace_impact_direct"),
]
RETRIEVAL_BASE_METHODS = [
    "baseline_git_diff_only",
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
    "workspace_impact_direct",
    "workspace_impact_pagerank",
    "workspace_impact_hybrid",
]
REPO_HOLDOUT_BASE_METHODS = [
    "baseline_path_locality",
    "baseline_lexical_similarity",
    "baseline_content_similarity",
    "baseline_recent_activity",
    "baseline_global_pagerank",
    HISTORY_ORACLE_METHOD,
    "workspace_related_direct",
    "workspace_related_pagerank",
    "workspace_related_hybrid",
]


def run(cmd: list[str], cwd: Path, *, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(cmd, cwd=cwd, text=True, capture_output=True)
    if check and result.returncode != 0:
        raise RuntimeError(
            f"command failed: {cmd}\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def workspace_bin() -> Path:
    explicit = os.environ.get("WORKSPACE_BIN")
    if explicit:
        return Path(explicit)
    run(["cargo", "build"], ROOT)
    return ROOT / "target" / "debug" / "workspace"


def workspace_json(bin_path: Path, cwd: Path, *args: str) -> dict[str, Any]:
    result = run([str(bin_path), *args], cwd)
    return json.loads(result.stdout)


def cli_weight(value: float) -> str:
    return f"{value:.3f}".rstrip("0").rstrip(".")


def hybrid_weight_method(prefix: str, weight: float) -> str:
    return f"{prefix}_w_{cli_weight(weight).replace('.', '_')}"


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def append(path: Path, content: str) -> None:
    with path.open("a") as file:
        file.write(content)


def git(cwd: Path, *args: str) -> None:
    run(["git", *args], cwd)


def git_text(cwd: Path, *args: str, check: bool = True) -> str:
    return run(["git", *args], cwd, check=check).stdout


def git_text_or_none(cwd: Path, *args: str) -> str | None:
    result = run(["git", *args], cwd, check=False)
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def commit_all(cwd: Path, message: str) -> None:
    git(cwd, "add", ".")
    git(cwd, "commit", "-m", message, "-q")


def precision_recall(results: list[str], expected: set[str], k: int) -> dict[str, Any]:
    top = results[:k]
    hits = [path for path in top if path in expected]
    precision = len(hits) / len(top) if top else 0.0
    recall = len(set(top) & expected) / len(expected) if expected else 1.0
    return {
        "top": top,
        "hits": hits,
        "precision": round(precision, 3),
        "recall": round(recall, 3),
        "hit_count": len(set(top) & expected),
        "expected_count": len(expected),
    }


def ranking_metrics(results: list[str], expected: set[str], k: int) -> dict[str, Any]:
    top = results[:k]
    seen_hits: set[str] = set()
    precision_sum = 0.0
    reciprocal_rank = 0.0
    dcg = 0.0

    for index, path in enumerate(top, start=1):
        if path not in expected or path in seen_hits:
            continue
        seen_hits.add(path)
        precision_sum += len(seen_hits) / index
        if reciprocal_rank == 0.0:
            reciprocal_rank = 1.0 / index
        dcg += 1.0 / math.log2(index + 1)

    ideal_hits = min(len(expected), k)
    idcg = sum(1.0 / math.log2(index + 1) for index in range(1, ideal_hits + 1))
    return {
        "top": top,
        "hits": [path for path in top if path in expected],
        "returned": len(top),
        f"precision_at_{k}": round(len(seen_hits) / k, 3) if k else 0.0,
        f"recall_at_{k}": round(len(seen_hits) / len(expected), 3) if expected else 1.0,
        f"average_precision_at_{k}": round(precision_sum / len(expected), 3)
        if expected
        else 1.0,
        "mrr": round(reciprocal_rank, 3),
        f"ndcg_at_{k}": round(dcg / idcg, 3) if idcg else 1.0,
        "hit_count": len(seen_hits),
        "expected_count": len(expected),
    }


def ranking_diagnostics(
    results: list[str],
    expected: set[str],
    k: int,
) -> dict[str, Any]:
    rank_by_path: dict[str, int] = {}
    for rank, path in enumerate(results, start=1):
        rank_by_path.setdefault(path, rank)

    hits_at_k = set(results[:k]) & expected
    missing_expected = sorted(expected - hits_at_k)
    return {
        "candidate_count": len(results),
        "diagnostic_limit": len(results),
        "missing_expected_ranks": [
            {
                "path": path,
                "rank": rank_by_path.get(path),
            }
            for path in missing_expected
        ],
        "top_false_positives": [
            path for path in results[:k] if path not in expected
        ],
    }


def aggregate_metric_sets(scenarios: list[dict[str, Any]], k: int) -> dict[str, Any]:
    method_names = sorted(
        {
            method
            for scenario in scenarios
            for method in scenario["methods"].keys()
        }
    )
    metric_names = [
        f"precision_at_{k}",
        f"recall_at_{k}",
        f"average_precision_at_{k}",
        "mrr",
        f"ndcg_at_{k}",
    ]
    aggregate: dict[str, Any] = {}
    for method in method_names:
        values = [
            scenario["methods"][method]
            for scenario in scenarios
            if method in scenario["methods"]
        ]
        method_summary: dict[str, Any] = {}
        for name in metric_names:
            metric_values = [value[name] for value in values]
            method_summary[f"mean_{name}"] = round(mean(metric_values), 3)
            low, high = bootstrap_mean_ci(metric_values, method, name)
            method_summary[f"ci95_low_{name}"] = round(low, 3)
            method_summary[f"ci95_high_{name}"] = round(high, 3)
        method_summary["scenario_count"] = len(values)
        aggregate[method] = method_summary
    return aggregate


def distribution(values: list[int]) -> dict[str, Any]:
    if not values:
        return {
            "count": 0,
            "min": 0,
            "median": 0.0,
            "mean": 0.0,
            "max": 0,
        }
    sorted_values = sorted(values)
    return {
        "count": len(sorted_values),
        "min": sorted_values[0],
        "median": round(percentile_sorted(sorted_values, 0.5), 3),
        "mean": round(mean(sorted_values), 3),
        "max": sorted_values[-1],
    }


def holdout_dataset_summary(
    *,
    candidate_commit_count: int,
    examined_commit_count: int,
    heldout_commit_count: int,
    cases: list[dict[str, Any]],
    skipped: dict[str, int],
    limits: dict[str, int],
) -> dict[str, Any]:
    target_counts = [len(case["expected"]) for case in cases]
    predictable_target_counts = [
        len(case.get("predictable_expected", [])) for case in cases
    ]
    unpredictable_target_counts = [
        len(case.get("unpredictable_expected", [])) for case in cases
    ]
    predictable_cases = [
        case for case in cases if case.get("predictable_expected")
    ]
    return {
        "candidate_commit_count": candidate_commit_count,
        "examined_commit_count": examined_commit_count,
        "heldout_commit_count": heldout_commit_count,
        "case_count": len(cases),
        "target_count": sum(target_counts),
        "predictable_case_count": len(predictable_cases),
        "predictable_target_count": sum(predictable_target_counts),
        "unpredictable_target_count": sum(unpredictable_target_counts),
        "skipped": dict(sorted(skipped.items())),
        "limits": limits,
        "target_count_distribution": distribution(target_counts),
        "predictable_target_count_distribution": distribution(
            [
                len(case["predictable_expected"])
                for case in predictable_cases
            ]
        ),
    }


def macro_average_repo_holdouts(
    holdouts: list[dict[str, Any]],
    k: int,
    pairs: list[tuple[str, str]],
    summary_key: str | None = None,
) -> dict[str, Any]:
    summaries = []
    for holdout in holdouts:
        summary = holdout[summary_key] if summary_key is not None else holdout
        if summary["case_count"] > 0:
            summaries.append(summary)
    return {
        "k": k,
        "repo_count": len(summaries),
        "aggregate": macro_average_metric_sets(summaries, k),
        "paired_deltas": macro_paired_delta_metric_sets(summaries, k, pairs),
    }


def macro_average_metric_sets(holdouts: list[dict[str, Any]], k: int) -> dict[str, Any]:
    method_names = sorted(
        {
            method
            for holdout in holdouts
            for method in holdout["aggregate"].keys()
        }
    )
    metric_names = [
        f"precision_at_{k}",
        f"recall_at_{k}",
        f"average_precision_at_{k}",
        "mrr",
        f"ndcg_at_{k}",
    ]
    aggregate: dict[str, Any] = {}
    for method in method_names:
        method_holdouts = [
            holdout
            for holdout in holdouts
            if method in holdout["aggregate"]
        ]
        method_summary: dict[str, Any] = {
            "repo_count": len(method_holdouts),
            "scenario_count": len(method_holdouts),
        }
        for name in metric_names:
            metric_values = [
                holdout["aggregate"][method][f"mean_{name}"]
                for holdout in method_holdouts
            ]
            method_summary[f"mean_{name}"] = round(mean(metric_values), 3)
            low, high = bootstrap_mean_ci(metric_values, f"repo_macro:{method}", name)
            method_summary[f"ci95_low_{name}"] = round(low, 3)
            method_summary[f"ci95_high_{name}"] = round(high, 3)
        aggregate[method] = method_summary
    return aggregate


def macro_paired_delta_metric_sets(
    holdouts: list[dict[str, Any]],
    k: int,
    pairs: list[tuple[str, str]],
) -> dict[str, Any]:
    metric_names = [
        f"precision_at_{k}",
        f"recall_at_{k}",
        f"average_precision_at_{k}",
        "mrr",
        f"ndcg_at_{k}",
    ]
    deltas: dict[str, Any] = {}
    for left, right in pairs:
        comparison_name = f"{left}_minus_{right}"
        matching_holdouts = [
            holdout
            for holdout in holdouts
            if comparison_name in holdout["paired_deltas"]
        ]
        if not matching_holdouts:
            continue
        comparison: dict[str, Any] = {
            "repo_count": len(matching_holdouts),
            "scenario_count": len(matching_holdouts),
        }
        for metric_name in metric_names:
            values = [
                holdout["paired_deltas"][comparison_name][
                    f"mean_delta_{metric_name}"
                ]
                for holdout in matching_holdouts
            ]
            comparison[f"mean_delta_{metric_name}"] = round(mean(values), 3)
            low, high = bootstrap_mean_ci(
                values,
                f"repo_macro:{comparison_name}",
                metric_name,
            )
            comparison[f"ci95_low_delta_{metric_name}"] = round(low, 3)
            comparison[f"ci95_high_delta_{metric_name}"] = round(high, 3)
            wins = sum(1 for value in values if value > 0)
            ties = sum(1 for value in values if value == 0)
            losses = sum(1 for value in values if value < 0)
            comparison[f"win_count_delta_{metric_name}"] = wins
            comparison[f"tie_count_delta_{metric_name}"] = ties
            comparison[f"loss_count_delta_{metric_name}"] = losses
            comparison[f"win_rate_delta_{metric_name}"] = round(
                wins / len(values),
                3,
            )
            p_greater, p_two_sided = paired_sign_flip_p_values(
                values,
                f"repo_macro:{comparison_name}",
                metric_name,
            )
            comparison[f"p_greater_delta_{metric_name}"] = bounded_p_value(p_greater)
            comparison[f"p_two_sided_delta_{metric_name}"] = bounded_p_value(
                p_two_sided
            )
        deltas[comparison_name] = comparison
    apply_holm_adjusted_p_values(deltas, metric_names)
    return deltas


def default_cutoffs(k: int) -> list[int]:
    return sorted({cutoff for cutoff in DEFAULT_CUTOFF_SWEEP if cutoff <= k} | {k})


def cutoff_sweep_metric_sets(
    scenarios: list[dict[str, Any]],
    cutoffs: list[int],
    pairs: list[tuple[str, str]],
) -> list[dict[str, Any]]:
    sweep = []
    for cutoff in cutoffs:
        cutoff_scenarios = []
        for scenario in scenarios:
            expected = set(scenario["expected"])
            cutoff_scenarios.append(
                {
                    "methods": {
                        method: ranking_metrics(metrics["top"], expected, cutoff)
                        for method, metrics in scenario["methods"].items()
                    }
                }
            )
        sweep.append(
            {
                "k": cutoff,
                "sample_count": len(cutoff_scenarios),
                "aggregate": aggregate_metric_sets(cutoff_scenarios, cutoff)
                if cutoff_scenarios
                else {},
                "paired_deltas": paired_delta_metric_sets(
                    cutoff_scenarios,
                    cutoff,
                    pairs,
                )
                if cutoff_scenarios
                else {},
            }
        )
    return sweep


def selected_metric_scenarios(
    scenarios: list[dict[str, Any]],
    methods: list[str],
) -> list[dict[str, Any]]:
    selected = []
    for scenario in scenarios:
        scenario_methods = {
            method: scenario["methods"][method]
            for method in methods
            if method in scenario["methods"]
        }
        if scenario_methods:
            selected_scenario = {"methods": scenario_methods}
            if "expected" in scenario:
                selected_scenario["expected"] = scenario["expected"]
            selected.append(selected_scenario)
    return selected


def retarget_metric_scenarios(
    scenarios: list[dict[str, Any]],
    k: int,
    expected_key: str,
    methods: list[str] | None = None,
) -> list[dict[str, Any]]:
    retargeted = []
    for scenario in scenarios:
        expected = set(scenario.get(expected_key, []))
        if not expected:
            continue
        method_names = methods or sorted(scenario["methods"])
        scenario_methods = {
            method: ranking_metrics(
                scenario["methods"][method]["top"],
                expected,
                k,
            )
            for method in method_names
            if method in scenario["methods"]
        }
        if scenario_methods:
            retargeted.append(
                {
                    "expected": sorted(expected),
                    "methods": scenario_methods,
                }
            )
    return retargeted


def hybrid_weight_sweep_metric_sets(
    scenarios: list[dict[str, Any]],
    k: int,
    weights: list[float],
) -> list[dict[str, Any]]:
    sweep = []
    for weight in weights:
        related_method = hybrid_weight_method("workspace_related_hybrid", weight)
        impact_method = hybrid_weight_method("workspace_impact_hybrid", weight)
        related_methods = [
            related_method,
            "workspace_related_direct",
            "workspace_related_pagerank",
        ]
        impact_methods = [
            impact_method,
            "workspace_impact_direct",
            "workspace_impact_pagerank",
        ]
        related_scenarios = selected_metric_scenarios(scenarios, related_methods)
        impact_scenarios = selected_metric_scenarios(scenarios, impact_methods)
        entry: dict[str, Any] = {"hybrid_direct_weight": weight}
        if related_scenarios:
            entry["related"] = {
                "method": related_method,
                "aggregate": aggregate_metric_sets(related_scenarios, k),
                "paired_deltas": paired_delta_metric_sets(
                    related_scenarios,
                    k,
                    [
                        (related_method, "workspace_related_direct"),
                        (related_method, "workspace_related_pagerank"),
                    ],
                ),
            }
        if impact_scenarios:
            entry["impact"] = {
                "method": impact_method,
                "aggregate": aggregate_metric_sets(impact_scenarios, k),
                "paired_deltas": paired_delta_metric_sets(
                    impact_scenarios,
                    k,
                    [
                        (impact_method, "workspace_impact_direct"),
                        (impact_method, "workspace_impact_pagerank"),
                    ],
                ),
            }
        sweep.append(entry)
    return sweep


def repo_holdout_metric_summary(
    cases: list[dict[str, Any]],
    k: int,
    hybrid_weights: list[float],
    *,
    expected_key: str = "expected",
) -> dict[str, Any]:
    scenarios = retarget_metric_scenarios(cases, k, expected_key)
    base_cases = selected_metric_scenarios(scenarios, REPO_HOLDOUT_BASE_METHODS)
    return {
        "k": k,
        "case_count": len(scenarios),
        "target_count": sum(len(scenario["expected"]) for scenario in scenarios),
        "aggregate": aggregate_metric_sets(base_cases, k) if base_cases else {},
        "cutoff_sweep": cutoff_sweep_metric_sets(
            base_cases,
            default_cutoffs(k),
            RELATED_COMPARISON_PAIRS,
        )
        if base_cases
        else [],
        "hybrid_weight_sweep": hybrid_weight_sweep_metric_sets(
            scenarios,
            k,
            hybrid_weights,
        )
        if scenarios
        else [],
        "paired_deltas": paired_delta_metric_sets(
            base_cases,
            k,
            RELATED_COMPARISON_PAIRS,
        )
        if base_cases
        else {},
    }


def temporal_leakage_audit(cases: list[dict[str, Any]]) -> dict[str, Any]:
    failures = []
    checked = 0
    matched = 0
    for case in cases:
        index = case.get("index", {})
        if not isinstance(index, dict) or "head_matches_parent" not in index:
            continue
        checked += 1
        if index["head_matches_parent"]:
            matched += 1
            continue
        failures.append(
            {
                "repo": case.get("repo", ""),
                "seed": case.get("seed", ""),
                "heldout_commit": case.get("heldout_commit", ""),
                "parent": case.get("parent", ""),
                "index_head": index.get("head"),
            }
        )
    failure_limit = 10
    return {
        "case_count": len(cases),
        "checked_case_count": checked,
        "head_matches_parent_count": matched,
        "failure_count": len(failures),
        "failures": failures[:failure_limit],
        "omitted_failures": max(0, len(failures) - failure_limit),
    }


def repo_holdout_leave_one_repo_out_weight_selection(
    holdouts: list[dict[str, Any]],
    k: int,
    weights: list[float],
    *,
    expected_key: str = "expected",
) -> dict[str, Any]:
    if len(holdouts) < 2 or not weights:
        return {
            "k": k,
            "case_count": 0,
            "target_count": 0,
            "candidate_weights": weights,
            "selections": [],
            "aggregate": {},
            "cutoff_sweep": [],
            "paired_deltas": {},
        }

    candidate_methods = [
        (weight, hybrid_weight_method("workspace_related_hybrid", weight))
        for weight in weights
    ]
    selected_scenarios: list[dict[str, Any]] = []
    selections: list[dict[str, Any]] = []
    for test_holdout in holdouts:
        train_cases = [
            case
            for holdout in holdouts
            if holdout["repo"] != test_holdout["repo"]
            for case in holdout["cases"]
        ]
        train_scenarios = retarget_metric_scenarios(
            train_cases,
            k,
            expected_key,
            [method for _, method in candidate_methods],
        )
        test_scenarios = retarget_metric_scenarios(
            test_holdout["cases"],
            k,
            expected_key,
        )
        if not train_scenarios or not test_scenarios:
            continue

        weight_summaries = []
        for weight, method in candidate_methods:
            method_scenarios = selected_metric_scenarios(train_scenarios, [method])
            if not method_scenarios:
                continue
            aggregate = aggregate_metric_sets(method_scenarios, k)
            summary = aggregate[method]
            weight_summaries.append(
                {
                    "hybrid_direct_weight": weight,
                    "method": method,
                    "train_case_count": len(method_scenarios),
                    f"train_average_precision_at_{k}": summary[
                        f"mean_average_precision_at_{k}"
                    ],
                    f"train_ndcg_at_{k}": summary[f"mean_ndcg_at_{k}"],
                }
            )
        if not weight_summaries:
            continue

        selected = select_loro_weight_summary(weight_summaries, k)
        selected_method = selected["method"]
        repo_selected_scenarios = []
        for scenario in test_scenarios:
            if selected_method not in scenario["methods"]:
                continue
            methods = {
                method: scenario["methods"][method]
                for method in REPO_HOLDOUT_BASE_METHODS
                if method in scenario["methods"]
            }
            methods[RELATED_HYBRID_LORO_METHOD] = scenario["methods"][selected_method]
            repo_selected_scenarios.append(
                {
                    "expected": scenario["expected"],
                    "methods": methods,
                }
            )
        if not repo_selected_scenarios:
            continue

        selected_scenarios.extend(repo_selected_scenarios)
        test_aggregate = aggregate_metric_sets(repo_selected_scenarios, k)
        test_summary = test_aggregate[RELATED_HYBRID_LORO_METHOD]
        selections.append(
            {
                "repo": test_holdout["repo"],
                "selected_hybrid_direct_weight": selected[
                    "hybrid_direct_weight"
                ],
                "train_case_count": selected["train_case_count"],
                "test_case_count": len(repo_selected_scenarios),
                "test_target_count": sum(
                    len(scenario["expected"])
                    for scenario in repo_selected_scenarios
                ),
                f"train_average_precision_at_{k}": selected[
                    f"train_average_precision_at_{k}"
                ],
                f"train_ndcg_at_{k}": selected[f"train_ndcg_at_{k}"],
                f"test_average_precision_at_{k}": test_summary[
                    f"mean_average_precision_at_{k}"
                ],
                f"test_ndcg_at_{k}": test_summary[f"mean_ndcg_at_{k}"],
            }
        )

    return {
        "k": k,
        "case_count": len(selected_scenarios),
        "target_count": sum(
            len(scenario["expected"]) for scenario in selected_scenarios
        ),
        "candidate_weights": weights,
        "selections": selections,
        "aggregate": aggregate_metric_sets(selected_scenarios, k)
        if selected_scenarios
        else {},
        "cutoff_sweep": cutoff_sweep_metric_sets(
            selected_scenarios,
            default_cutoffs(k),
            RELATED_LORO_COMPARISON_PAIRS,
        )
        if selected_scenarios
        else [],
        "paired_deltas": paired_delta_metric_sets(
            selected_scenarios,
            k,
            RELATED_LORO_COMPARISON_PAIRS,
        )
        if selected_scenarios
        else {},
    }


def select_loro_weight_summary(
    weight_summaries: list[dict[str, Any]],
    k: int,
) -> dict[str, Any]:
    ap_metric = f"train_average_precision_at_{k}"
    ndcg_metric = f"train_ndcg_at_{k}"
    best_ap = max(float(summary[ap_metric]) for summary in weight_summaries)
    ap_candidates = [
        summary
        for summary in weight_summaries
        if float(summary[ap_metric])
        >= best_ap - LORO_WEIGHT_SELECTION_AP_TOLERANCE
    ]
    best_ndcg = max(float(summary[ndcg_metric]) for summary in ap_candidates)
    ndcg_candidates = [
        summary
        for summary in ap_candidates
        if float(summary[ndcg_metric])
        >= best_ndcg - LORO_WEIGHT_SELECTION_NDCG_TOLERANCE
    ]
    return max(
        ndcg_candidates,
        key=lambda summary: (
            -abs(
                summary["hybrid_direct_weight"]
                - RELATED_HYBRID_DEFAULT_DIRECT_WEIGHT
            ),
            -summary["hybrid_direct_weight"],
            summary[ap_metric],
            summary[ndcg_metric],
        ),
    )


def paired_delta_metric_sets(
    scenarios: list[dict[str, Any]],
    k: int,
    pairs: list[tuple[str, str]],
) -> dict[str, Any]:
    metric_names = [
        f"precision_at_{k}",
        f"recall_at_{k}",
        f"average_precision_at_{k}",
        "mrr",
        f"ndcg_at_{k}",
    ]
    deltas: dict[str, Any] = {}
    for left, right in pairs:
        common = [
            scenario
            for scenario in scenarios
            if left in scenario["methods"] and right in scenario["methods"]
        ]
        if not common:
            continue

        comparison_name = f"{left}_minus_{right}"
        comparison: dict[str, Any] = {"scenario_count": len(common)}
        for metric_name in metric_names:
            values = [
                scenario["methods"][left][metric_name]
                - scenario["methods"][right][metric_name]
                for scenario in common
            ]
            comparison[f"mean_delta_{metric_name}"] = round(mean(values), 3)
            low, high = bootstrap_mean_ci(values, comparison_name, metric_name)
            comparison[f"ci95_low_delta_{metric_name}"] = round(low, 3)
            comparison[f"ci95_high_delta_{metric_name}"] = round(high, 3)
            wins = sum(1 for value in values if value > 0)
            ties = sum(1 for value in values if value == 0)
            losses = sum(1 for value in values if value < 0)
            comparison[f"win_count_delta_{metric_name}"] = wins
            comparison[f"tie_count_delta_{metric_name}"] = ties
            comparison[f"loss_count_delta_{metric_name}"] = losses
            comparison[f"win_rate_delta_{metric_name}"] = round(
                wins / len(values), 3
            )
            p_greater, p_two_sided = paired_sign_flip_p_values(
                values,
                comparison_name,
                metric_name,
            )
            comparison[f"p_greater_delta_{metric_name}"] = bounded_p_value(p_greater)
            comparison[f"p_two_sided_delta_{metric_name}"] = bounded_p_value(
                p_two_sided
            )
        deltas[comparison_name] = comparison
    apply_holm_adjusted_p_values(deltas, metric_names)
    return deltas


def apply_holm_adjusted_p_values(
    deltas: dict[str, Any],
    metric_names: list[str],
) -> None:
    for metric_name in metric_names:
        for prefix in ["p_greater", "p_two_sided"]:
            source_key = f"{prefix}_delta_{metric_name}"
            adjusted_key = f"{prefix}_holm_delta_{metric_name}"
            entries = [
                (comparison_name, summary[source_key])
                for comparison_name, summary in deltas.items()
                if source_key in summary
            ]
            adjusted = holm_adjusted_p_values(
                [float(value) for _, value in entries]
            )
            for (comparison_name, _), adjusted_value in zip(entries, adjusted):
                deltas[comparison_name][adjusted_key] = adjusted_value


def holm_adjusted_p_values(values: list[float]) -> list[float]:
    if not values:
        return []
    adjusted = [0.0 for _ in values]
    previous = 0.0
    ordered = sorted(enumerate(values), key=lambda item: (item[1], item[0]))
    total = len(values)
    for rank, (original_index, value) in enumerate(ordered, start=1):
        multiplier = total - rank + 1
        adjusted_value = min(max(value, 0.0) * multiplier, 1.0)
        adjusted_value = max(previous, adjusted_value)
        previous = adjusted_value
        adjusted[original_index] = bounded_p_value(adjusted_value)
    return adjusted


def bounded_p_value(value: float) -> float:
    return min(max(value, 0.0), 1.0)


def mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def bootstrap_mean_ci(
    values: list[float],
    method: str,
    metric_name: str,
    samples: int = BOOTSTRAP_SAMPLES,
) -> tuple[float, float]:
    if not values:
        return (0.0, 0.0)
    if len(values) == 1:
        return (values[0], values[0])

    seed = int.from_bytes(
        hashlib.sha256(f"{method}:{metric_name}:{len(values)}".encode()).digest()[:8],
        "big",
    )
    rng = random.Random(seed)
    means = []
    for _ in range(samples):
        means.append(mean([values[rng.randrange(len(values))] for _ in values]))
    means.sort()
    return (
        percentile_sorted(means, 0.025),
        percentile_sorted(means, 0.975),
    )


def paired_sign_flip_p_values(
    values: list[float],
    comparison_name: str,
    metric_name: str,
    samples: int = SIGN_FLIP_SAMPLES,
) -> tuple[float, float]:
    if not values:
        return (1.0, 1.0)

    exact = exact_grid_sign_flip_p_values(values)
    if exact is not None:
        return exact

    observed = mean(values)
    observed_abs = abs(observed)
    absolute_values = [abs(value) for value in values]
    tolerance = 1e-12

    if len(values) <= 16:
        total = 1 << len(values)
        greater_or_equal = 0
        two_sided_or_equal = 0
        for mask in range(total):
            signed_sum = 0.0
            for index, value in enumerate(absolute_values):
                signed_sum += value if (mask >> index) & 1 else -value
            signed_mean = signed_sum / len(values)
            if signed_mean >= observed - tolerance:
                greater_or_equal += 1
            if abs(signed_mean) >= observed_abs - tolerance:
                two_sided_or_equal += 1
        return (greater_or_equal / total, two_sided_or_equal / total)

    seed = int.from_bytes(
        hashlib.sha256(
            f"signflip:{comparison_name}:{metric_name}:{len(values)}".encode()
        ).digest()[:8],
        "big",
    )
    rng = random.Random(seed)
    greater_or_equal = 1
    two_sided_or_equal = 1
    for _ in range(samples):
        signed_sum = sum(
            value if rng.randrange(2) else -value for value in absolute_values
        )
        signed_mean = signed_sum / len(values)
        if signed_mean >= observed - tolerance:
            greater_or_equal += 1
        if abs(signed_mean) >= observed_abs - tolerance:
            two_sided_or_equal += 1
    denominator = samples + 1
    return (greater_or_equal / denominator, two_sided_or_equal / denominator)


def exact_grid_sign_flip_p_values(values: list[float]) -> tuple[float, float] | None:
    signed_values = []
    absolute_values = []
    for value in values:
        scaled_value = int(round(value * SIGN_FLIP_EXACT_SCALE))
        if abs(value - (scaled_value / SIGN_FLIP_EXACT_SCALE)) > 1e-9:
            return None
        signed_values.append(scaled_value)
        absolute_values.append(abs(scaled_value))

    max_abs_sum = sum(absolute_values)
    if (max_abs_sum * 2) + 1 > SIGN_FLIP_EXACT_MAX_STATES:
        return None

    counts = {0: 1}
    for value in absolute_values:
        next_counts: dict[int, int] = {}
        for signed_sum, count in counts.items():
            positive_sum = signed_sum + value
            negative_sum = signed_sum - value
            next_counts[positive_sum] = next_counts.get(positive_sum, 0) + count
            next_counts[negative_sum] = next_counts.get(negative_sum, 0) + count
        counts = next_counts

    observed_sum = sum(signed_values)
    observed_abs_sum = abs(observed_sum)
    greater_or_equal = sum(
        count for signed_sum, count in counts.items() if signed_sum >= observed_sum
    )
    two_sided_or_equal = sum(
        count
        for signed_sum, count in counts.items()
        if abs(signed_sum) >= observed_abs_sum
    )
    total = 1 << len(values)
    return (greater_or_equal / total, two_sided_or_equal / total)


def percentile_sorted(values: list[float], quantile: float) -> float:
    if not values:
        return 0.0
    position = quantile * (len(values) - 1)
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return values[lower]
    fraction = position - lower
    return values[lower] + ((values[upper] - values[lower]) * fraction)


def paths(value: dict[str, Any], *segments: str) -> list[str]:
    cursor: Any = value
    for segment in segments:
        cursor = cursor[segment]
    return [item["path"] for item in cursor]


def observable_repo_path(path: str) -> str | None:
    normalized = path.strip().replace("\\", "/").rstrip("/")
    while normalized.startswith("./"):
        normalized = normalized[2:]
    if (
        not normalized
        or normalized.startswith("/")
        or normalized == ".workspace"
        or normalized.startswith(".workspace/")
        or normalized.startswith(".git/")
    ):
        return None
    segments = normalized.split("/")
    if any(not segment or segment in {".", ".."} for segment in segments):
        return None
    return normalized


def parse_git_name_only_commits(output: str) -> list[dict[str, Any]]:
    commits: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for raw_line in output.splitlines():
        line = raw_line.strip()
        if line.startswith("commit:"):
            if current is not None:
                commits.append(current)
            current = {"hash": line.removeprefix("commit:"), "files": []}
            continue
        if current is None:
            continue
        path = observable_repo_path(line)
        if path is not None and path not in current["files"]:
            current["files"].append(path)
    if current is not None:
        commits.append(current)
    return commits


def tracked_repo_paths(root: Path) -> set[str]:
    return {
        path
        for path in git_text(root, "ls-files").splitlines()
        if observable_repo_path(path) is not None
    }


def path_parent(path: str) -> str:
    return path.rsplit("/", 1)[0] if "/" in path else ""


def path_extension(path: str) -> str:
    name = path.rsplit("/", 1)[-1]
    return name.rsplit(".", 1)[-1] if "." in name else ""


def common_directory_prefix(a: str, b: str) -> int:
    a_parts = path_parent(a).split("/") if path_parent(a) else []
    b_parts = path_parent(b).split("/") if path_parent(b) else []
    count = 0
    for left, right in zip(a_parts, b_parts):
        if left != right:
            break
        count += 1
    return count


def path_locality_score(candidate: str, seed: str) -> float:
    score = float(common_directory_prefix(candidate, seed) * 4)
    if path_parent(candidate) == path_parent(seed):
        score += 3.0
    candidate_extension = path_extension(candidate)
    if candidate_extension and candidate_extension == path_extension(seed):
        score += 1.0
    return score


def path_locality_paths(root: Path, seeds: set[str]) -> list[str]:
    tracked_paths = sorted(tracked_repo_paths(root))
    normalized_seeds = {path for path in seeds if path}
    ranked = []
    for path in tracked_paths:
        if path in normalized_seeds:
            continue
        score = max(
            (path_locality_score(path, seed) for seed in normalized_seeds),
            default=0.0,
        )
        ranked.append((score, path))
    return [
        path
        for _, path in sorted(ranked, key=lambda item: (-item[0], item[1]))
    ]


PATH_TOKEN_RE = re.compile(r"[A-Za-z0-9]+")
PATH_TOKEN_STOPWORDS = {
    "src",
    "test",
    "tests",
    "doc",
    "docs",
    "lib",
    "rs",
    "py",
    "js",
    "ts",
    "tsx",
    "md",
    "json",
    "toml",
    "yaml",
    "yml",
}


def path_tokens(path: str) -> set[str]:
    camel_split = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", path)
    return {
        token.lower()
        for token in PATH_TOKEN_RE.findall(camel_split)
        if token and token.lower() not in PATH_TOKEN_STOPWORDS
    }


def lexical_similarity_score(candidate: str, seed: str) -> float:
    candidate_tokens = path_tokens(candidate)
    seed_tokens = path_tokens(seed)
    if not candidate_tokens or not seed_tokens:
        return 0.0
    overlap = len(candidate_tokens & seed_tokens)
    return overlap / math.sqrt(len(candidate_tokens) * len(seed_tokens))


def lexical_similarity_paths(root: Path, seeds: set[str]) -> list[str]:
    tracked_paths = sorted(tracked_repo_paths(root))
    normalized_seeds = {path for path in seeds if path}
    ranked = []
    for path in tracked_paths:
        if path in normalized_seeds:
            continue
        score = max(
            (lexical_similarity_score(path, seed) for seed in normalized_seeds),
            default=0.0,
        )
        ranked.append((score, path))
    return [
        path
        for _, path in sorted(ranked, key=lambda item: (-item[0], item[1]))
    ]


CONTENT_TOKEN_RE = re.compile(r"[A-Za-z0-9_]+")
CONTENT_CORPUS_CACHE: dict[
    tuple[str, str],
    tuple[list[str], dict[str, Counter[str]], dict[str, float], dict[str, float]],
] = {}


def content_tokens(text: str) -> list[str]:
    camel_split = re.sub(r"([a-z0-9])([A-Z])", r"\1 \2", text)
    return [
        token.lower()
        for token in CONTENT_TOKEN_RE.findall(camel_split)
        if token
    ]


def read_similarity_text(root: Path, path: str) -> str:
    full_path = root / path
    try:
        data = full_path.read_bytes()
    except OSError:
        return ""
    if len(data) > CONTENT_SIMILARITY_MAX_BYTES or b"\x00" in data:
        return ""
    return data.decode("utf-8", errors="ignore")


def content_similarity_corpus(
    root: Path,
) -> tuple[list[str], dict[str, Counter[str]], dict[str, float], dict[str, float]]:
    root_key = str(root.resolve())
    head = git_text_or_none(root, "rev-parse", "HEAD") or ""
    cache_key = (root_key, head)
    if cache_key in CONTENT_CORPUS_CACHE:
        return CONTENT_CORPUS_CACHE[cache_key]

    tracked_paths = sorted(tracked_repo_paths(root))
    document_counts: dict[str, Counter[str]] = {}
    document_frequency: Counter[str] = Counter()
    for path in tracked_paths:
        counts = Counter(content_tokens(read_similarity_text(root, path)))
        document_counts[path] = counts
        document_frequency.update(counts.keys())

    document_count = max(len(tracked_paths), 1)
    idf = {
        token: math.log((document_count + 1.0) / (frequency + 1.0)) + 1.0
        for token, frequency in document_frequency.items()
    }
    norms = {
        path: tfidf_norm(counts, idf)
        for path, counts in document_counts.items()
    }
    corpus = (tracked_paths, document_counts, idf, norms)
    CONTENT_CORPUS_CACHE[cache_key] = corpus
    return corpus


def tfidf_norm(counts: Counter[str], idf: dict[str, float]) -> float:
    return math.sqrt(
        sum((count * idf.get(token, 0.0)) ** 2 for token, count in counts.items())
    )


def tfidf_dot(
    left: Counter[str],
    right: Counter[str],
    idf: dict[str, float],
) -> float:
    if len(left) > len(right):
        left, right = right, left
    total = 0.0
    for token, left_count in left.items():
        right_count = right.get(token, 0)
        if right_count:
            weight = idf.get(token, 0.0)
            total += left_count * right_count * weight * weight
    return total


def content_similarity_paths(root: Path, seeds: set[str]) -> list[str]:
    tracked_paths, document_counts, idf, norms = content_similarity_corpus(root)
    normalized_seeds = {path for path in seeds if path}
    seed_counts: Counter[str] = Counter()
    for seed in sorted(normalized_seeds):
        seed_counts.update(document_counts.get(seed, Counter()))
    seed_norm = tfidf_norm(seed_counts, idf)

    ranked = []
    for path in tracked_paths:
        if path in normalized_seeds:
            continue
        candidate_norm = norms.get(path, 0.0)
        if seed_norm == 0.0 or candidate_norm == 0.0:
            score = 0.0
        else:
            score = tfidf_dot(seed_counts, document_counts[path], idf) / (
                seed_norm * candidate_norm
            )
        ranked.append((score, path))
    return [
        path
        for _, path in sorted(ranked, key=lambda item: (-item[0], item[1]))
    ]


def recent_activity_paths(
    root: Path,
    exclude: set[str],
    *,
    max_commits: int = RECENT_ACTIVITY_MAX_COMMITS,
) -> list[str]:
    tracked_paths = tracked_repo_paths(root)
    output = git_text(
        root,
        "log",
        "--format=commit:%H",
        "--name-only",
        f"--max-count={max_commits}",
        "--",
    )
    ranked: list[str] = []
    seen: set[str] = set()
    for commit in parse_git_name_only_commits(output):
        for path in commit["files"]:
            if path in exclude or path in seen or path not in tracked_paths:
                continue
            ranked.append(path)
            seen.add(path)
    return ranked


def cochange_index(root: Path) -> dict[str, Any]:
    return json.loads((root / ".workspace/index/cochange.json").read_text())


def global_pagerank_paths(
    root: Path,
    exclude: set[str],
    *,
    iterations: int = GLOBAL_PAGERANK_ITERATIONS,
    damping: float = 0.85,
) -> list[str]:
    index = cochange_index(root)
    nodes = set(index["file_commit_counts"].keys())
    graph: dict[str, list[tuple[str, float]]] = {node: [] for node in nodes}
    for edge in index["edges"]:
        weight = max(float(edge["weighted_cochanges"]), 0.0)
        if weight == 0.0:
            continue
        a = edge["a"]
        b = edge["b"]
        nodes.add(a)
        nodes.add(b)
        graph.setdefault(a, []).append((b, weight))
        graph.setdefault(b, []).append((a, weight))

    if not nodes:
        return []

    node_list = sorted(nodes)
    graph = {
        node: sorted(graph.get(node, []), key=lambda item: item[0])
        for node in node_list
    }
    node_count = len(nodes)
    uniform = 1.0 / node_count
    rank = {node: uniform for node in node_list}
    outbound_weights = {
        node: sum(weight for _, weight in graph.get(node, []))
        for node in node_list
    }
    for _ in range(iterations):
        next_rank = {node: (1.0 - damping) * uniform for node in node_list}
        dangling_rank = 0.0
        for node in node_list:
            neighbors = graph.get(node, [])
            total_weight = outbound_weights.get(node, 0.0)
            if not neighbors or total_weight == 0.0:
                dangling_rank += rank[node]
                continue
            for neighbor, weight in neighbors:
                next_rank[neighbor] += damping * rank[node] * (weight / total_weight)
        if dangling_rank:
            dangling_share = damping * dangling_rank * uniform
            for node in node_list:
                next_rank[node] += dangling_share
        rank = next_rank

    return [
        path
        for path, _ in sorted(
            rank.items(),
            key=lambda item: (-item[1], item[0]),
        )
        if path not in exclude
    ]


def make_history_fixture() -> tempfile.TemporaryDirectory[str]:
    temp = tempfile.TemporaryDirectory()
    root = Path(temp.name)
    git(root, "init", "-q")
    git(root, "config", "user.email", "measure@example.com")
    git(root, "config", "user.name", "Measure")

    write(root / "README.md", "# effect fixture\n")
    write(
        root / "Cargo.toml",
        '[package]\nname = "effect-fixture"\nversion = "0.1.0"\nedition = "2024"\n',
    )
    write(root / "src/main.rs", "fn main() {}\n")
    commit_all(root, "initial project scaffold")

    write(root / "src/auth.rs", "auth module\n")
    write(root / "src/session.rs", "session module\n")
    commit_all(root, "auth with session")

    append(root / "src/session.rs", "session change\n")
    write(root / "src/cookie.rs", "cookie module\n")
    commit_all(root, "session with cookie")

    append(root / "src/cookie.rs", "cookie change\n")
    write(root / "tests/cookie_test.rs", "cookie tests\n")
    commit_all(root, "cookie with tests")

    write(root / "docs/auth.md", "auth docs\n")
    write(root / "src/unrelated.rs", "unrelated\n")
    commit_all(root, "unrelated docs")
    return temp


def make_broad_noise_fixture() -> tempfile.TemporaryDirectory[str]:
    temp = tempfile.TemporaryDirectory()
    root = Path(temp.name)
    git(root, "init", "-q")
    git(root, "config", "user.email", "measure@example.com")
    git(root, "config", "user.name", "Measure")

    write(root / "README.md", "# broad noise fixture\n")
    write(root / "src/main.rs", "fn main() {}\n")
    commit_all(root, "initial project scaffold")

    write(root / "src/core.rs", "core module\n")
    write(root / "tests/core_test.rs", "core tests\n")
    commit_all(root, "core with tests")

    append(root / "src/core.rs", "generated churn touch\n")
    for index in range(20):
        write(root / f"generated/snapshot_{index:02}.txt", f"snapshot {index}\n")
    commit_all(root, "large generated update")
    return temp


def make_multi_seed_fixture() -> tempfile.TemporaryDirectory[str]:
    temp = tempfile.TemporaryDirectory()
    root = Path(temp.name)
    git(root, "init", "-q")
    git(root, "config", "user.email", "measure@example.com")
    git(root, "config", "user.name", "Measure")

    write(root / "README.md", "# multi seed fixture\n")
    write(root / "src/main.rs", "fn main() {}\n")
    commit_all(root, "initial project scaffold")

    write(root / "src/api.rs", "api module\n")
    write(root / "src/shared.rs", "shared module\n")
    commit_all(root, "api with shared")

    write(root / "src/worker.rs", "worker module\n")
    append(root / "src/shared.rs", "worker shared change\n")
    commit_all(root, "worker with shared")

    append(root / "src/shared.rs", "tested behavior\n")
    write(root / "tests/shared_test.rs", "shared tests\n")
    commit_all(root, "shared with tests")
    return temp


def make_doc_noise_fixture() -> tempfile.TemporaryDirectory[str]:
    temp = tempfile.TemporaryDirectory()
    root = Path(temp.name)
    git(root, "init", "-q")
    git(root, "config", "user.email", "measure@example.com")
    git(root, "config", "user.name", "Measure")

    write(root / "README.md", "# doc noise fixture\n")
    write(root / "src/main.rs", "fn main() {}\n")
    commit_all(root, "initial project scaffold")

    write(root / "src/core.rs", "core module\n")
    write(root / "src/adapter.rs", "adapter module\n")
    commit_all(root, "core with adapter")

    append(root / "src/adapter.rs", "tested behavior\n")
    write(root / "tests/adapter_test.rs", "adapter tests\n")
    commit_all(root, "adapter with tests")

    for index in range(3):
        append(root / "src/core.rs", f"doc touch {index}\n")
        write(root / f"docs/core_{index}.md", f"core docs {index}\n")
        commit_all(root, f"core docs {index}")
    return temp


def measure_observation(bin_path: Path) -> dict[str, Any]:
    with make_history_fixture() as name:
        root = Path(name)
        result = workspace_json(bin_path, root, "map", "--json")
        data = result["data"]
        expected_facts = {
            "package_manager:cargo": "cargo" in data["stack"]["package_managers"],
            "entrypoint:src/main.rs": "src/main.rs" in data["structure"]["entrypoints"],
            "test:tests/cookie_test.rs": "tests/cookie_test.rs" in data["structure"]["tests"],
            "config:Cargo.toml": "Cargo.toml" in data["structure"]["configs"],
            "doc:README.md": "README.md" in data["structure"]["docs"],
            "command:test": "test" in data["commands"],
            "next:read README": "workspace read README.md" in result["next_observations"],
        }
        found = [name for name, ok in expected_facts.items() if ok]
        return {
            "metric": "map_fact_recall",
            "found": found,
            "missing": [name for name, ok in expected_facts.items() if not ok],
            "recall": round(len(found) / len(expected_facts), 3),
            "expected_count": len(expected_facts),
        }


def measure_related_and_impact(bin_path: Path) -> dict[str, Any]:
    with make_history_fixture() as name:
        root = Path(name)
        workspace_json(bin_path, root, "index", "cochange", "--json")
        expected = {"src/session.rs", "src/cookie.rs", "tests/cookie_test.rs"}
        path_locality = path_locality_paths(root, {"src/auth.rs"})
        lexical_similarity = lexical_similarity_paths(root, {"src/auth.rs"})
        content_similarity = content_similarity_paths(root, {"src/auth.rs"})
        recent_activity = recent_activity_paths(root, {"src/auth.rs"})
        global_pagerank = global_pagerank_paths(root, {"src/auth.rs"})

        direct = workspace_json(
            bin_path,
            root,
            "related",
            "src/auth.rs",
            "--by",
            "cochange",
            "--use-index",
            "--json",
        )
        pagerank = workspace_json(
            bin_path,
            root,
            "related",
            "src/auth.rs",
            "--by",
            "cochange",
            "--rank",
            "pagerank",
            "--json",
        )
        hybrid = workspace_json(
            bin_path,
            root,
            "related",
            "src/auth.rs",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--json",
        )

        append(root / "src/auth.rs", "local auth change\n")
        direct_impact = workspace_json(
            bin_path,
            root,
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--use-index",
            "--json",
        )
        impact = workspace_json(
            bin_path,
            root,
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--rank",
            "pagerank",
            "--json",
        )
        hybrid_impact = workspace_json(
            bin_path,
            root,
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--json",
        )
        git_diff = run(["git", "diff", "--name-only"], root).stdout.splitlines()

        return {
            "metric": "related_and_impact_recall_at_3",
            "expected_impacted_files": sorted(expected),
            "baseline_git_diff_only": precision_recall(git_diff, expected, 3),
            "baseline_path_locality": precision_recall(
                path_locality, expected, 3
            ),
            "baseline_lexical_similarity": precision_recall(
                lexical_similarity, expected, 3
            ),
            "baseline_content_similarity": precision_recall(
                content_similarity, expected, 3
            ),
            "baseline_recent_activity": precision_recall(
                recent_activity, expected, 3
            ),
            "baseline_global_pagerank": precision_recall(
                global_pagerank, expected, 3
            ),
            "workspace_related_direct": precision_recall(
                paths(direct, "data", "related"), expected, 3
            ),
            "workspace_related_pagerank": precision_recall(
                paths(pagerank, "data", "related"), expected, 3
            ),
            "workspace_related_hybrid": precision_recall(
                paths(hybrid, "data", "related"), expected, 3
            ),
            "workspace_impact_direct": precision_recall(
                paths(direct_impact, "data", "impacted"), expected, 3
            ),
            "workspace_impact_pagerank": precision_recall(
                paths(impact, "data", "impacted"), expected, 3
            ),
            "workspace_impact_hybrid": precision_recall(
                paths(hybrid_impact, "data", "impacted"), expected, 3
            ),
        }


def evaluate_related_case(
    bin_path: Path,
    *,
    name: str,
    root: Path,
    target: str,
    expected: set[str],
    max_files_per_commit: int = 40,
    k: int = 5,
    hybrid_weights: list[float] | None = None,
) -> dict[str, Any]:
    hybrid_weights = hybrid_weights or []
    index = workspace_json(
        bin_path,
        root,
        "index",
        "cochange",
        "--max-files-per-commit",
        str(max_files_per_commit),
        "--json",
    )
    path_locality = path_locality_paths(root, {target})
    lexical_similarity = lexical_similarity_paths(root, {target})
    content_similarity = content_similarity_paths(root, {target})
    recent_activity = recent_activity_paths(root, {target})
    global_pagerank = global_pagerank_paths(root, {target})
    direct = workspace_json(
        bin_path,
        root,
        "related",
        target,
        "--by",
        "cochange",
        "--use-index",
        "--json",
    )
    pagerank = workspace_json(
        bin_path,
        root,
        "related",
        target,
        "--by",
        "cochange",
        "--rank",
        "pagerank",
        "--json",
    )
    hybrid = workspace_json(
        bin_path,
        root,
        "related",
        target,
        "--by",
        "cochange",
        "--rank",
        "hybrid",
        "--json",
    )
    weighted_related = {
        hybrid_weight_method("workspace_related_hybrid", weight): workspace_json(
            bin_path,
            root,
            "related",
            target,
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--hybrid-direct-weight",
            cli_weight(weight),
            "--json",
        )
        for weight in hybrid_weights
    }

    append(root / target, "local evaluation change\n")
    direct_impact = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--use-index",
        "--json",
    )
    impact = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--rank",
        "pagerank",
        "--json",
    )
    hybrid_impact = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--rank",
        "hybrid",
        "--json",
    )
    weighted_impact = {
        hybrid_weight_method("workspace_impact_hybrid", weight): workspace_json(
            bin_path,
            root,
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--hybrid-direct-weight",
            cli_weight(weight),
            "--json",
        )
        for weight in hybrid_weights
    }
    git_diff = run(["git", "diff", "--name-only"], root).stdout.splitlines()
    methods = {
        "baseline_git_diff_only": ranking_metrics(git_diff, expected, k),
        "baseline_path_locality": ranking_metrics(
            path_locality,
            expected,
            k,
        ),
        "baseline_lexical_similarity": ranking_metrics(
            lexical_similarity,
            expected,
            k,
        ),
        "baseline_content_similarity": ranking_metrics(
            content_similarity,
            expected,
            k,
        ),
        "baseline_recent_activity": ranking_metrics(
            recent_activity,
            expected,
            k,
        ),
        "baseline_global_pagerank": ranking_metrics(
            global_pagerank,
            expected,
            k,
        ),
        "workspace_related_direct": ranking_metrics(
            paths(direct, "data", "related"), expected, k
        ),
        "workspace_related_pagerank": ranking_metrics(
            paths(pagerank, "data", "related"), expected, k
        ),
        "workspace_related_hybrid": ranking_metrics(
            paths(hybrid, "data", "related"), expected, k
        ),
        "workspace_impact_direct": ranking_metrics(
            paths(direct_impact, "data", "impacted"), expected, k
        ),
        "workspace_impact_pagerank": ranking_metrics(
            paths(impact, "data", "impacted"), expected, k
        ),
        "workspace_impact_hybrid": ranking_metrics(
            paths(hybrid_impact, "data", "impacted"), expected, k
        ),
    }
    methods.update(
        {
            method: ranking_metrics(paths(result, "data", "related"), expected, k)
            for method, result in weighted_related.items()
        }
    )
    methods.update(
        {
            method: ranking_metrics(paths(result, "data", "impacted"), expected, k)
            for method, result in weighted_impact.items()
        }
    )

    return {
        "name": name,
        "task": "single_seed_related_and_impact",
        "target": target,
        "expected": sorted(expected),
        "index": {
            "commits_indexed": index["data"]["commits_indexed"],
            "ignored_large_commits": index["data"]["ignored_large_commits"],
            "edge_count": index["data"]["edge_count"],
        },
        "methods": methods,
    }


def evaluate_impact_case(
    bin_path: Path,
    *,
    name: str,
    root: Path,
    seed_files: list[str],
    expected: set[str],
    max_files_per_commit: int = 40,
    k: int = 5,
    hybrid_weights: list[float] | None = None,
) -> dict[str, Any]:
    hybrid_weights = hybrid_weights or []
    index = workspace_json(
        bin_path,
        root,
        "index",
        "cochange",
        "--max-files-per-commit",
        str(max_files_per_commit),
        "--json",
    )
    seed_set = set(seed_files)
    path_locality = path_locality_paths(root, seed_set)
    lexical_similarity = lexical_similarity_paths(root, seed_set)
    content_similarity = content_similarity_paths(root, seed_set)
    recent_activity = recent_activity_paths(root, seed_set)
    global_pagerank = global_pagerank_paths(root, seed_set)
    for seed in seed_files:
        append(root / seed, "local evaluation change\n")

    direct = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--use-index",
        "--json",
    )
    pagerank = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--rank",
        "pagerank",
        "--json",
    )
    hybrid = workspace_json(
        bin_path,
        root,
        "impact",
        "--diff",
        "--by",
        "cochange",
        "--rank",
        "hybrid",
        "--json",
    )
    weighted_impact = {
        hybrid_weight_method("workspace_impact_hybrid", weight): workspace_json(
            bin_path,
            root,
            "impact",
            "--diff",
            "--by",
            "cochange",
            "--rank",
            "hybrid",
            "--hybrid-direct-weight",
            cli_weight(weight),
            "--json",
        )
        for weight in hybrid_weights
    }
    git_diff = run(["git", "diff", "--name-only"], root).stdout.splitlines()
    methods = {
        "baseline_git_diff_only": ranking_metrics(git_diff, expected, k),
        "baseline_path_locality": ranking_metrics(
            path_locality,
            expected,
            k,
        ),
        "baseline_lexical_similarity": ranking_metrics(
            lexical_similarity,
            expected,
            k,
        ),
        "baseline_content_similarity": ranking_metrics(
            content_similarity,
            expected,
            k,
        ),
        "baseline_recent_activity": ranking_metrics(
            recent_activity,
            expected,
            k,
        ),
        "baseline_global_pagerank": ranking_metrics(
            global_pagerank,
            expected,
            k,
        ),
        "workspace_impact_direct": ranking_metrics(
            paths(direct, "data", "impacted"), expected, k
        ),
        "workspace_impact_pagerank": ranking_metrics(
            paths(pagerank, "data", "impacted"), expected, k
        ),
        "workspace_impact_hybrid": ranking_metrics(
            paths(hybrid, "data", "impacted"), expected, k
        ),
    }
    methods.update(
        {
            method: ranking_metrics(paths(result, "data", "impacted"), expected, k)
            for method, result in weighted_impact.items()
        }
    )

    return {
        "name": name,
        "task": "multi_seed_impact",
        "seed_files": seed_files,
        "expected": sorted(expected),
        "index": {
            "commits_indexed": index["data"]["commits_indexed"],
            "ignored_large_commits": index["data"]["ignored_large_commits"],
            "edge_count": index["data"]["edge_count"],
        },
        "methods": methods,
    }


def measure_retrieval_suite(
    bin_path: Path,
    k: int,
    hybrid_weights: list[float],
) -> dict[str, Any]:
    scenarios: list[dict[str, Any]] = []
    pairs = RELATED_COMPARISON_PAIRS + IMPACT_COMPARISON_PAIRS

    with make_history_fixture() as name:
        scenarios.append(
            evaluate_related_case(
                bin_path,
                name="transitive_auth_chain",
                root=Path(name),
                target="src/auth.rs",
                expected={"src/session.rs", "src/cookie.rs", "tests/cookie_test.rs"},
                k=k,
                hybrid_weights=hybrid_weights,
            )
        )

    with make_broad_noise_fixture() as name:
        scenarios.append(
            evaluate_related_case(
                bin_path,
                name="broad_generated_commit_filtered",
                root=Path(name),
                target="src/core.rs",
                expected={"tests/core_test.rs"},
                max_files_per_commit=8,
                k=k,
                hybrid_weights=hybrid_weights,
            )
        )

    with make_multi_seed_fixture() as name:
        scenarios.append(
            evaluate_impact_case(
                bin_path,
                name="multi_seed_shared_test_discovery",
                root=Path(name),
                seed_files=["src/api.rs", "src/worker.rs"],
                expected={"src/shared.rs", "tests/shared_test.rs"},
                k=k,
                hybrid_weights=hybrid_weights,
            )
        )

    with make_doc_noise_fixture() as name:
        scenarios.append(
            evaluate_related_case(
                bin_path,
                name="direct_doc_noise_with_indirect_test",
                root=Path(name),
                target="src/core.rs",
                expected={"src/adapter.rs", "tests/adapter_test.rs"},
                k=k,
                hybrid_weights=hybrid_weights,
            )
        )

    base_scenarios = selected_metric_scenarios(scenarios, RETRIEVAL_BASE_METHODS)
    return {
        "metric": "retrieval_suite",
        "k": k,
        "scenario_count": len(scenarios),
        "scenarios": scenarios,
        "aggregate": aggregate_metric_sets(base_scenarios, k),
        "cutoff_sweep": cutoff_sweep_metric_sets(
            base_scenarios,
            default_cutoffs(k),
            pairs,
        ),
        "hybrid_weight_sweep": hybrid_weight_sweep_metric_sets(
            scenarios,
            k,
            hybrid_weights,
        ),
        "paired_deltas": paired_delta_metric_sets(
            base_scenarios,
            k,
            pairs,
        ),
    }


def measure_transaction(bin_path: Path) -> dict[str, Any]:
    with tempfile.TemporaryDirectory() as name:
        root = Path(name)
        git(root, "init", "-q")
        git(root, "config", "user.email", "measure@example.com")
        git(root, "config", "user.name", "Measure")
        write(root / "note.txt", "hello\n")
        commit_all(root, "initial")
        write(
            root / "change.patch",
            """diff --git a/note.txt b/note.txt
--- a/note.txt
+++ b/note.txt
@@ -1 +1 @@
-hello
+hello workspace
""",
        )

        patch = workspace_json(
            bin_path,
            root,
            "patch",
            "--description",
            "measure transaction",
            "change.patch",
            "--json",
        )
        transaction_id = patch["data"]["transaction_id"]
        diff_after_patch = workspace_json(bin_path, root, "diff", "--summary", "--json")
        verify = workspace_json(
            bin_path,
            root,
            "run",
            'test "$(cat note.txt)" = "hello workspace"',
            "--json",
        )
        log = workspace_json(bin_path, root, "log", "--json")
        rollback = workspace_json(bin_path, root, "rollback", transaction_id, "--json")
        diff_after_rollback = workspace_json(bin_path, root, "diff", "--summary", "--json")

        signals = {
            "transaction_id": bool(transaction_id),
            "files_changed": patch["data"]["files_changed"] == ["note.txt"],
            "diff_after_patch": diff_after_patch["data"]["files"] == ["note.txt"],
            "verification_exit_zero": verify["data"]["exit_code"] == 0,
            "log_has_patch_and_run": {"patch", "run"}.issubset(
                {entry["op"] for entry in log["data"]["entries"]}
            ),
            "rollback_restored_file": (root / "note.txt").read_text() == "hello\n",
            "diff_clean_after_rollback": diff_after_rollback["data"]["files"] == [],
        }
        passed = [name for name, ok in signals.items() if ok]
        return {
            "metric": "transaction_audit_signal_recall",
            "passed": passed,
            "failed": [name for name, ok in signals.items() if not ok],
            "recall": round(len(passed) / len(signals), 3),
            "expected_count": len(signals),
        }


def measure_repo_holdout(
    bin_path: Path,
    repo: Path,
    *,
    end_ref: str,
    max_heldout_commits: int,
    max_candidate_commits: int,
    max_files_per_commit: int,
    k: int,
    hybrid_weights: list[float],
) -> dict[str, Any]:
    repo = repo.resolve()
    end_commit = git_text(
        repo,
        "rev-parse",
        "--verify",
        f"{end_ref}^{{commit}}",
    ).strip()
    log_output = git_text(
        repo,
        "log",
        "--format=commit:%H",
        "--name-only",
        f"--max-count={max_candidate_commits}",
        end_commit,
        "--",
    )
    commits = parse_git_name_only_commits(log_output)
    cases: list[dict[str, Any]] = []
    heldout_commit_records: list[dict[str, Any]] = []
    examined_commit_count = 0
    skipped = {
        "root_commit": 0,
        "too_few_files": 0,
        "too_many_files": 0,
        "new_seed_file": 0,
    }

    with tempfile.TemporaryDirectory() as clone_name:
        clone = Path(clone_name) / "repo"
        run(["git", "clone", "--quiet", str(repo), str(clone)], repo)
        git(clone, "config", "advice.detachedHead", "false")

        heldout_commits = 0
        for commit in commits:
            examined_commit_count += 1
            files = commit["files"]
            if len(files) < 2:
                skipped["too_few_files"] += 1
                continue
            if len(files) > max_files_per_commit:
                skipped["too_many_files"] += 1
                continue

            parent_result = run(
                ["git", "rev-parse", f"{commit['hash']}^"],
                repo,
                check=False,
            )
            if parent_result.returncode != 0:
                skipped["root_commit"] += 1
                continue
            parent = parent_result.stdout.strip()

            git(clone, "checkout", "--quiet", parent)
            index = workspace_json(
                bin_path,
                clone,
                "index",
                "cochange",
                "--max-files-per-commit",
                str(max_files_per_commit),
                "--json",
            )
            index_head = index["data"].get("head")
            parent_paths = tracked_repo_paths(clone)

            heldout_commits += 1
            commit_cases: list[dict[str, Any]] = []
            new_seed_count = 0
            for seed in files:
                exists = run(
                    ["git", "cat-file", "-e", f"{parent}:{seed}"],
                    repo,
                    check=False,
                ).returncode == 0
                if not exists:
                    skipped["new_seed_file"] += 1
                    new_seed_count += 1
                    continue

                expected = set(files) - {seed}
                predictable_expected = expected & parent_paths
                path_locality = path_locality_paths(clone, {seed})
                lexical_similarity = lexical_similarity_paths(clone, {seed})
                content_similarity = content_similarity_paths(clone, {seed})
                recent_activity = recent_activity_paths(clone, {seed})
                global_pagerank = global_pagerank_paths(clone, {seed})
                direct = workspace_json(
                    bin_path,
                    clone,
                    "related",
                    seed,
                    "--by",
                    "cochange",
                    "--use-index",
                    "--json",
                )
                pagerank = workspace_json(
                    bin_path,
                    clone,
                    "related",
                    seed,
                    "--by",
                    "cochange",
                    "--rank",
                    "pagerank",
                    "--json",
                )
                hybrid = workspace_json(
                    bin_path,
                    clone,
                    "related",
                    seed,
                    "--by",
                    "cochange",
                    "--rank",
                    "hybrid",
                    "--json",
                )
                diagnostic_hybrid = workspace_json(
                    bin_path,
                    clone,
                    "related",
                    seed,
                    "--by",
                    "cochange",
                    "--rank",
                    "hybrid",
                    "--max-results",
                    str(max(k, RELATED_DIAGNOSTIC_MAX_RESULTS)),
                    "--json",
                )
                weighted_hybrid = {
                    hybrid_weight_method(
                        "workspace_related_hybrid",
                        weight,
                    ): workspace_json(
                        bin_path,
                        clone,
                        "related",
                        seed,
                        "--by",
                        "cochange",
                        "--rank",
                        "hybrid",
                        "--hybrid-direct-weight",
                        cli_weight(weight),
                        "--json",
                    )
                    for weight in hybrid_weights
                }
                methods = {
                    HISTORY_ORACLE_METHOD: ranking_metrics(
                        sorted(predictable_expected), expected, k
                    ),
                    "baseline_path_locality": ranking_metrics(
                        path_locality, expected, k
                    ),
                    "baseline_lexical_similarity": ranking_metrics(
                        lexical_similarity, expected, k
                    ),
                    "baseline_content_similarity": ranking_metrics(
                        content_similarity, expected, k
                    ),
                    "baseline_recent_activity": ranking_metrics(
                        recent_activity, expected, k
                    ),
                    "baseline_global_pagerank": ranking_metrics(
                        global_pagerank, expected, k
                    ),
                    "workspace_related_direct": ranking_metrics(
                        paths(direct, "data", "related"), expected, k
                    ),
                    "workspace_related_pagerank": ranking_metrics(
                        paths(pagerank, "data", "related"), expected, k
                    ),
                    "workspace_related_hybrid": ranking_metrics(
                        paths(hybrid, "data", "related"), expected, k
                    ),
                }
                methods.update(
                    {
                        method: ranking_metrics(
                            paths(result, "data", "related"),
                            expected,
                            k,
                        )
                        for method, result in weighted_hybrid.items()
                    }
                )
                commit_cases.append(
                    {
                        "repo": str(repo),
                        "heldout_commit": commit["hash"][:12],
                        "parent": parent[:12],
                        "seed": seed,
                        "expected": sorted(expected),
                        "predictable_expected": sorted(predictable_expected),
                        "unpredictable_expected": sorted(
                            expected - predictable_expected
                        ),
                        "index": {
                            "head": index_head[:12] if index_head else None,
                            "head_matches_parent": index_head == parent,
                            "commits_indexed": index["data"]["commits_indexed"],
                            "ignored_large_commits": index["data"][
                                "ignored_large_commits"
                            ],
                            "edge_count": index["data"]["edge_count"],
                        },
                        "methods": methods,
                        "diagnostics": {
                            "workspace_related_hybrid": ranking_diagnostics(
                                paths(diagnostic_hybrid, "data", "related"),
                                expected,
                                k,
                            ),
                        },
                    }
                )

            cases.extend(commit_cases)
            heldout_commit_records.append(
                {
                    "commit": commit["hash"][:12],
                    "parent": parent[:12],
                    "file_count": len(files),
                    "case_count": len(commit_cases),
                    "target_count": sum(
                        len(case["expected"]) for case in commit_cases
                    ),
                    "predictable_target_count": sum(
                        len(case["predictable_expected"]) for case in commit_cases
                    ),
                    "unpredictable_target_count": sum(
                        len(case["unpredictable_expected"]) for case in commit_cases
                    ),
                    "new_seed_count": new_seed_count,
                }
            )

            if heldout_commits >= max_heldout_commits:
                break

    limits = {
        "max_candidate_commits": max_candidate_commits,
        "max_heldout_commits": max_heldout_commits,
        "max_files_per_commit": max_files_per_commit,
    }
    all_target_summary = repo_holdout_metric_summary(cases, k, hybrid_weights)
    predictable_summary = repo_holdout_metric_summary(
        cases,
        k,
        hybrid_weights,
        expected_key="predictable_expected",
    )
    return {
        "metric": "repo_temporal_holdout",
        "repo": str(repo),
        "end_ref": end_ref,
        "end_commit": end_commit[:12],
        "k": k,
        "candidate_commit_count": len(commits),
        "examined_commit_count": examined_commit_count,
        "heldout_commit_count": heldout_commits,
        "case_count": len(cases),
        "skipped": skipped,
        "limits": limits,
        "heldout_commits": heldout_commit_records,
        "cases": cases,
        "aggregate": all_target_summary["aggregate"],
        "cutoff_sweep": all_target_summary["cutoff_sweep"],
        "hybrid_weight_sweep": all_target_summary["hybrid_weight_sweep"],
        "paired_deltas": all_target_summary["paired_deltas"],
        "target_count": all_target_summary["target_count"],
        "temporal_leakage_audit": temporal_leakage_audit(cases),
        "predictable_only": predictable_summary,
        "dataset": holdout_dataset_summary(
            candidate_commit_count=len(commits),
            examined_commit_count=examined_commit_count,
            heldout_commit_count=heldout_commits,
            cases=cases,
            skipped=skipped,
            limits=limits,
        ),
    }


def aggregate_repo_holdouts(
    holdouts: list[dict[str, Any]],
    k: int,
    hybrid_weights: list[float],
) -> dict[str, Any]:
    cases = [
        case
        for holdout in holdouts
        for case in holdout["cases"]
    ]
    skipped: dict[str, int] = {}
    for holdout in holdouts:
        for key, value in holdout["skipped"].items():
            skipped[key] = skipped.get(key, 0) + value

    all_target_summary = repo_holdout_metric_summary(cases, k, hybrid_weights)
    predictable_summary = repo_holdout_metric_summary(
        cases,
        k,
        hybrid_weights,
        expected_key="predictable_expected",
    )
    predictable_summary["repo_macro_average"] = macro_average_repo_holdouts(
        holdouts,
        k,
        RELATED_COMPARISON_PAIRS,
        summary_key="predictable_only",
    )
    predictable_summary["leave_one_repo_out_weight_selection"] = (
        repo_holdout_leave_one_repo_out_weight_selection(
            holdouts,
            k,
            hybrid_weights,
            expected_key="predictable_expected",
        )
    )
    candidate_commit_count = sum(
        holdout["candidate_commit_count"] for holdout in holdouts
    )
    examined_commit_count = sum(
        holdout["examined_commit_count"] for holdout in holdouts
    )
    heldout_commit_count = sum(
        holdout["heldout_commit_count"] for holdout in holdouts
    )
    limits = holdouts[0].get("limits", {}) if holdouts else {}
    return {
        "metric": "repo_temporal_holdout_aggregate",
        "repo_count": len(holdouts),
        "repos": [holdout["repo"] for holdout in holdouts],
        "end_refs": [holdout["end_ref"] for holdout in holdouts],
        "end_commits": [holdout["end_commit"] for holdout in holdouts],
        "k": k,
        "candidate_commit_count": candidate_commit_count,
        "examined_commit_count": examined_commit_count,
        "heldout_commit_count": heldout_commit_count,
        "case_count": len(cases),
        "target_count": all_target_summary["target_count"],
        "skipped": skipped,
        "limits": limits,
        "aggregate": all_target_summary["aggregate"],
        "cutoff_sweep": all_target_summary["cutoff_sweep"],
        "hybrid_weight_sweep": all_target_summary["hybrid_weight_sweep"],
        "repo_macro_average": macro_average_repo_holdouts(
            holdouts,
            k,
            RELATED_COMPARISON_PAIRS,
        ),
        "paired_deltas": all_target_summary["paired_deltas"],
        "temporal_leakage_audit": temporal_leakage_audit(cases),
        "predictable_only": predictable_summary,
        "leave_one_repo_out_weight_selection": (
            repo_holdout_leave_one_repo_out_weight_selection(
                holdouts,
                k,
                hybrid_weights,
            )
        ),
        "dataset": holdout_dataset_summary(
            candidate_commit_count=candidate_commit_count,
            examined_commit_count=examined_commit_count,
            heldout_commit_count=heldout_commit_count,
            cases=cases,
            skipped=skipped,
            limits=limits,
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-holdout-manifest",
        type=Path,
        help="JSON file with repo/ref holdout entries and optional default limits",
    )
    parser.add_argument(
        "--repo-holdout",
        action="append",
        default=[],
        type=Path,
        help="optionally add temporal holdout metrics for a real git repository; repeat for multiple repos",
    )
    parser.add_argument(
        "--repo-holdout-ref",
        action="append",
        default=[],
        help="git revision that ends the corresponding --repo-holdout history; repeat once per repo",
    )
    parser.add_argument("--max-heldout-commits", type=int)
    parser.add_argument("--max-candidate-commits", type=int)
    parser.add_argument("--max-files-per-commit", type=int)
    parser.add_argument("--k", type=int)
    parser.add_argument(
        "--hybrid-direct-weight-sweep",
        default="",
        help="comma-separated hybrid direct weights to evaluate in addition to defaults",
    )
    args = parser.parse_args()
    apply_repo_holdout_manifest(args, parser)
    if args.max_heldout_commits is None:
        args.max_heldout_commits = 5
    if args.max_candidate_commits is None:
        args.max_candidate_commits = 40
    if args.max_files_per_commit is None:
        args.max_files_per_commit = 40
    if args.k is None:
        args.k = 5
    if args.k < 1:
        parser.error("--k must be at least 1")
    if args.repo_holdout_ref and len(args.repo_holdout_ref) != len(args.repo_holdout):
        parser.error("--repo-holdout-ref must be repeated once per --repo-holdout")
    args.hybrid_direct_weight_sweep = parse_hybrid_weight_sweep(
        args.hybrid_direct_weight_sweep,
        parser,
    )
    return args


def apply_repo_holdout_manifest(
    args: argparse.Namespace,
    parser: argparse.ArgumentParser,
) -> None:
    if args.repo_holdout_manifest is None:
        return
    if args.repo_holdout or args.repo_holdout_ref:
        parser.error(
            "--repo-holdout-manifest cannot be combined with "
            "--repo-holdout or --repo-holdout-ref"
        )
    try:
        manifest = json.loads(args.repo_holdout_manifest.read_text())
    except OSError as error:
        parser.error(f"cannot read --repo-holdout-manifest: {error}")
    except json.JSONDecodeError as error:
        parser.error(f"invalid --repo-holdout-manifest JSON: {error}")

    entries = manifest.get("repo_holdouts")
    if not isinstance(entries, list) or not entries:
        parser.error(
            "--repo-holdout-manifest must contain a non-empty repo_holdouts array"
        )

    repos = []
    refs = []
    manifest_records = []
    for index, entry in enumerate(entries, start=1):
        if not isinstance(entry, dict):
            parser.error(f"repo_holdouts[{index}] must be an object")
        repo = entry.get("repo")
        ref = entry.get("ref")
        remote_url = entry.get("remote_url", entry.get("url"))
        if not isinstance(repo, str) or not repo:
            parser.error(f"repo_holdouts[{index}].repo must be a non-empty string")
        if not isinstance(ref, str) or not ref:
            parser.error(f"repo_holdouts[{index}].ref must be a non-empty string")
        if remote_url is not None and not isinstance(remote_url, str):
            parser.error(f"repo_holdouts[{index}].remote_url must be a string")
        repos.append(Path(repo))
        refs.append(ref)
        manifest_record = {"repo": repo, "ref": ref}
        if remote_url:
            manifest_record["remote_url"] = remote_url
        manifest_records.append(manifest_record)
    args.repo_holdout = repos
    args.repo_holdout_ref = refs
    args.repo_holdout_manifest_records = manifest_records
    prepared_from = manifest.get("prepared_from")
    if prepared_from is not None:
        if not isinstance(prepared_from, dict):
            parser.error("--repo-holdout-manifest prepared_from must be an object")
        source_manifest = prepared_from.get("manifest")
        source_manifest_sha256 = prepared_from.get("manifest_sha256")
        if not isinstance(source_manifest, str) or not source_manifest:
            parser.error(
                "--repo-holdout-manifest prepared_from.manifest "
                "must be a non-empty string"
            )
        if (
            not isinstance(source_manifest_sha256, str)
            or not source_manifest_sha256
        ):
            parser.error(
                "--repo-holdout-manifest prepared_from.manifest_sha256 "
                "must be a non-empty string"
            )
        args.repo_holdout_manifest_prepared_from = {
            "manifest": source_manifest,
            "manifest_sha256": source_manifest_sha256,
        }

    for field in [
        "max_heldout_commits",
        "max_candidate_commits",
        "max_files_per_commit",
        "k",
    ]:
        if getattr(args, field, None) is None and field in manifest:
            value = manifest[field]
            if not isinstance(value, int):
                parser.error(f"--repo-holdout-manifest {field} must be an integer")
            setattr(args, field, value)

    if (
        not getattr(args, "hybrid_direct_weight_sweep", "")
        and "hybrid_direct_weight_sweep" in manifest
    ):
        value = manifest["hybrid_direct_weight_sweep"]
        if not isinstance(value, list):
            parser.error(
                "--repo-holdout-manifest hybrid_direct_weight_sweep must be an array"
            )
        args.hybrid_direct_weight_sweep = ",".join(str(weight) for weight in value)


def parse_hybrid_weight_sweep(
    value: str,
    parser: argparse.ArgumentParser,
) -> list[float]:
    if not value:
        return []
    weights = []
    for raw_weight in value.split(","):
        raw_weight = raw_weight.strip()
        if not raw_weight:
            parser.error("--hybrid-direct-weight-sweep contains an empty weight")
        try:
            weight = float(raw_weight)
        except ValueError:
            parser.error(f"invalid hybrid direct weight: {raw_weight!r}")
        if not math.isfinite(weight) or not 0.0 <= weight <= 1.0:
            parser.error("--hybrid-direct-weight-sweep values must be between 0.0 and 1.0")
        if weight not in weights:
            weights.append(weight)
    return weights


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def repo_remote_url(repo: Path) -> str | None:
    return git_text_or_none(repo.resolve(), "remote", "get-url", "origin")


def repo_holdout_record(
    repo: Path,
    ref: str,
    manifest_record: dict[str, Any] | None = None,
) -> dict[str, str]:
    record = {"repo": str(repo), "ref": ref}
    remote_url = None
    if manifest_record is not None:
        remote_url = manifest_record.get("remote_url")
    remote_url = remote_url or repo_remote_url(repo)
    if remote_url:
        record["remote_url"] = remote_url
    return record


def measurement_metadata(
    args: argparse.Namespace,
    bin_path: Path,
    hybrid_weights: list[float],
) -> dict[str, Any]:
    workspace_commit = git_text_or_none(ROOT, "rev-parse", "HEAD")
    workspace_status = git_text_or_none(ROOT, "status", "--porcelain")
    repo_holdout_refs = args.repo_holdout_ref or ["HEAD"] * len(args.repo_holdout)
    manifest_records = getattr(args, "repo_holdout_manifest_records", [])
    metadata: dict[str, Any] = {
        "schema_version": 1,
        "workspace_bin": str(bin_path),
        "workspace_repo": str(ROOT),
        "workspace_commit": workspace_commit,
        "workspace_dirty": bool(workspace_status),
        "workspace_status_line_count": (
            len(workspace_status.splitlines()) if workspace_status else 0
        ),
        "measurement_script": "tools/measure_effect.py",
        "primary_k": args.k,
        "bootstrap_samples": BOOTSTRAP_SAMPLES,
        "sign_flip_samples": SIGN_FLIP_SAMPLES,
        "sign_flip_method": "exact_grid_dp_with_sampled_fallback",
        "sign_flip_exact_scale": SIGN_FLIP_EXACT_SCALE,
        "sign_flip_exact_max_states": SIGN_FLIP_EXACT_MAX_STATES,
        "default_cutoff_sweep": DEFAULT_CUTOFF_SWEEP,
        "max_heldout_commits": args.max_heldout_commits,
        "max_candidate_commits": args.max_candidate_commits,
        "max_files_per_commit": args.max_files_per_commit,
        "hybrid_direct_weight_sweep": hybrid_weights,
        "repo_holdouts": [
            repo_holdout_record(
                repo,
                ref,
                manifest_records[index] if index < len(manifest_records) else None,
            )
            for index, (repo, ref) in enumerate(
                zip(args.repo_holdout, repo_holdout_refs)
            )
        ],
    }
    if args.repo_holdout_manifest is not None:
        metadata["repo_holdout_manifest"] = str(args.repo_holdout_manifest)
        metadata["repo_holdout_manifest_sha256"] = file_sha256(
            args.repo_holdout_manifest
        )
        prepared_from = getattr(args, "repo_holdout_manifest_prepared_from", None)
        if prepared_from:
            metadata["repo_holdout_source_manifest"] = prepared_from["manifest"]
            metadata["repo_holdout_source_manifest_sha256"] = prepared_from[
                "manifest_sha256"
            ]
    return metadata


def main() -> None:
    args = parse_args()
    bin_path = workspace_bin()
    hybrid_weights = args.hybrid_direct_weight_sweep
    measurements = [
        measure_observation(bin_path),
        measure_related_and_impact(bin_path),
        measure_retrieval_suite(bin_path, args.k, hybrid_weights),
        measure_transaction(bin_path),
    ]
    if args.repo_holdout:
        repo_holdout_refs = args.repo_holdout_ref or ["HEAD"] * len(args.repo_holdout)
        repo_holdouts = [
            measure_repo_holdout(
                bin_path,
                repo,
                end_ref=end_ref,
                max_heldout_commits=args.max_heldout_commits,
                max_candidate_commits=args.max_candidate_commits,
                max_files_per_commit=args.max_files_per_commit,
                k=args.k,
                hybrid_weights=hybrid_weights,
            )
            for repo, end_ref in zip(args.repo_holdout, repo_holdout_refs)
        ]
        measurements.extend(repo_holdouts)
        if len(repo_holdouts) > 1:
            measurements.append(
                aggregate_repo_holdouts(repo_holdouts, args.k, hybrid_weights)
            )

    report = {
        "workspace_bin": str(bin_path),
        "metadata": measurement_metadata(args, bin_path, hybrid_weights),
        "measurements": measurements,
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
