#!/usr/bin/env python3
"""Fail CI when effect measurement drops below expected effect thresholds."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_SIGN_FLIP_METHOD = "exact_grid_dp_with_sampled_fallback"
EXPECTED_HYBRID_WEIGHT_SWEEP = [
    0.0,
    0.05,
    0.1,
    0.25,
    0.5,
    0.6,
    0.7,
    0.75,
    0.8,
    0.82,
    0.85,
    0.88,
    0.9,
    0.92,
    0.95,
    1.0,
]
EXPECTED_RELATED_HYBRID_DEFAULT_WEIGHT = 0.9
FLOAT_TOLERANCE = 1e-12
MAX_HOLDOUT_HOLM_P = 0.005
MIN_HOLDOUT_ORACLE_NORMALIZED_AP = 0.90
HOLDOUT_HOLM_COMPARISONS = [
    "workspace_related_hybrid_minus_workspace_related_direct",
    "workspace_related_hybrid_minus_workspace_related_pagerank",
    "workspace_related_hybrid_minus_baseline_lexical_similarity",
    "workspace_related_hybrid_minus_baseline_content_similarity",
    "workspace_related_hybrid_minus_baseline_recent_activity",
    "workspace_related_hybrid_minus_baseline_global_pagerank",
]
HOLDOUT_DELTA_THRESHOLDS = [
    ("workspace_related_direct", "direct_delta"),
    ("workspace_related_pagerank", "pagerank_delta"),
    ("baseline_lexical_similarity", "lexical_delta"),
    ("baseline_content_similarity", "content_delta"),
    ("baseline_recent_activity", "recent_delta"),
    ("baseline_global_pagerank", "global_delta"),
]
LORO_HOLM_COMPARISONS = [
    "workspace_related_hybrid_loro_minus_workspace_related_direct",
    "workspace_related_hybrid_loro_minus_workspace_related_pagerank",
    "workspace_related_hybrid_loro_minus_baseline_lexical_similarity",
    "workspace_related_hybrid_loro_minus_baseline_content_similarity",
    "workspace_related_hybrid_loro_minus_baseline_recent_activity",
    "workspace_related_hybrid_loro_minus_baseline_global_pagerank",
]


def repo_holdout_thresholds(predictable: bool) -> dict[str, float]:
    if predictable:
        return {
            "hybrid_ap": 0.82,
            "direct_delta": 0.15,
            "lexical_delta": 0.50,
            "content_delta": 0.35,
            "recent_delta": 0.32,
            "global_delta": 0.24,
            "pagerank_delta": 0.18,
            "oracle_normalized": MIN_HOLDOUT_ORACLE_NORMALIZED_AP,
            "loro_ap": 0.82,
            "macro_hybrid_ap": 0.84,
            "macro_direct_delta": 0.16,
            "macro_pagerank_delta": 0.19,
            "default_weight_ap": 0.82,
        }
    return {
        "hybrid_ap": 0.78,
        "direct_delta": 0.13,
        "lexical_delta": 0.48,
        "content_delta": 0.33,
        "recent_delta": 0.30,
        "global_delta": 0.23,
        "pagerank_delta": 0.17,
        "oracle_normalized": MIN_HOLDOUT_ORACLE_NORMALIZED_AP,
        "loro_ap": 0.78,
        "macro_hybrid_ap": 0.81,
        "macro_direct_delta": 0.14,
        "macro_pagerank_delta": 0.18,
        "default_weight_ap": 0.78,
    }


def load_report(path: str) -> dict[str, Any]:
    if path == "-":
        return json.load(sys.stdin)
    return json.loads(Path(path).read_text())


def measurement_by_name(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    for measurement in report.get("measurements", []):
        if measurement.get("metric") == name:
            return measurement
    return None


def check_report(report: dict[str, Any], *, require_holdout: bool = False) -> list[str]:
    failures: list[str] = []

    metadata = report.get("metadata", {})
    sign_flip_method = metadata.get("sign_flip_method")
    if sign_flip_method != EXPECTED_SIGN_FLIP_METHOD:
        failures.append(
            "metadata.sign_flip_method "
            f"expected {EXPECTED_SIGN_FLIP_METHOD!r}, got {sign_flip_method!r}"
        )

    map_recall = measurement_by_name(report, "map_fact_recall")
    if not map_recall:
        failures.append("missing map_fact_recall measurement")
    elif float(map_recall.get("recall", 0.0)) < 1.0:
        failures.append(f"map_fact_recall recall < 1.0: {map_recall.get('recall')}")

    transaction = measurement_by_name(report, "transaction_audit_signal_recall")
    if not transaction:
        failures.append("missing transaction_audit_signal_recall measurement")
    elif float(transaction.get("recall", 0.0)) < 1.0:
        failures.append(
            "transaction_audit_signal_recall recall < 1.0: "
            f"{transaction.get('recall')}"
        )

    retrieval = measurement_by_name(report, "retrieval_suite")
    if not retrieval:
        failures.append("missing retrieval_suite measurement")
        return failures

    if int(retrieval.get("scenario_count", 0)) < 4:
        failures.append(
            f"retrieval_suite scenario_count < 4: {retrieval.get('scenario_count')}"
        )

    aggregate = retrieval.get("aggregate", {})
    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid",
        "mean_recall_at_5",
        0.99,
    )
    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid",
        "mean_average_precision_at_5",
        0.85,
    )
    require_mean(
        failures,
        aggregate,
        "workspace_impact_hybrid",
        "mean_recall_at_5",
        0.99,
    )
    require_mean(
        failures,
        aggregate,
        "workspace_impact_hybrid",
        "mean_average_precision_at_5",
        0.95,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_direct",
        metric="mean_average_precision_at_5",
        minimum=0.30,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_lexical_similarity",
        metric="mean_average_precision_at_5",
        minimum=0.25,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_content_similarity",
        metric="mean_average_precision_at_5",
        minimum=0.25,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_impact_hybrid",
        right="workspace_impact_direct",
        metric="mean_average_precision_at_5",
        minimum=0.40,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_impact_hybrid",
        right="baseline_lexical_similarity",
        metric="mean_average_precision_at_5",
        minimum=0.35,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_impact_hybrid",
        right="baseline_content_similarity",
        metric="mean_average_precision_at_5",
        minimum=0.30,
    )

    repo_holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if not repo_holdout:
        if require_holdout:
            failures.append("missing repo_temporal_holdout_aggregate measurement")
        return failures

    check_repo_holdout_thresholds(failures, repo_holdout, predictable=False)
    predictable = repo_holdout.get("predictable_only")
    if not isinstance(predictable, dict):
        failures.append("missing repo_temporal_holdout_aggregate.predictable_only")
    else:
        check_repo_holdout_thresholds(failures, predictable, predictable=True)
    return failures


def check_repo_holdout_thresholds(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    predictable: bool,
) -> None:
    label = "predictable repo_temporal_holdout_aggregate" if predictable else (
        "repo_temporal_holdout_aggregate"
    )
    if not predictable:
        require_count(failures, holdout, "repo_count", 3, label)
        require_temporal_leakage_audit(failures, holdout, label)
    require_count(failures, holdout, "case_count", 45, label)
    require_count(failures, holdout, "target_count", 180, label)

    aggregate = holdout.get("aggregate", {})
    thresholds = repo_holdout_thresholds(predictable)
    ap_metric = "mean_average_precision_at_5"
    delta_metric = "average_precision_at_5"

    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid",
        "mean_average_precision_at_5",
        thresholds["hybrid_ap"],
    )
    require_mean(
        failures,
        aggregate,
        "history_oracle_ceiling",
        "mean_average_precision_at_5",
        0.75,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_direct",
        metric=ap_metric,
        minimum=thresholds["direct_delta"],
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_lexical_similarity",
        metric=ap_metric,
        minimum=thresholds["lexical_delta"],
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_content_similarity",
        metric=ap_metric,
        minimum=thresholds["content_delta"],
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_recent_activity",
        metric=ap_metric,
        minimum=thresholds["recent_delta"],
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_global_pagerank",
        metric=ap_metric,
        minimum=thresholds["global_delta"],
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_pagerank",
        metric=ap_metric,
        minimum=thresholds["pagerank_delta"],
    )
    require_oracle_normalized(
        failures,
        aggregate,
        method="workspace_related_hybrid",
        oracle="history_oracle_ceiling",
        metric=ap_metric,
        minimum=thresholds["oracle_normalized"],
        label=label,
    )
    require_holm_evidence(
        failures,
        holdout.get("paired_deltas", {}),
        HOLDOUT_HOLM_COMPARISONS,
        metric=delta_metric,
        maximum=MAX_HOLDOUT_HOLM_P,
        label=label,
    )
    repo_macro = holdout.get("repo_macro_average")
    if not isinstance(repo_macro, dict):
        failures.append(f"{label} missing repo_macro_average")
    else:
        check_repo_macro_thresholds(
            failures,
            repo_macro,
            min_ap=thresholds["macro_hybrid_ap"],
            min_direct_delta=thresholds["macro_direct_delta"],
            min_pagerank_delta=thresholds["macro_pagerank_delta"],
            min_lexical_delta=thresholds["lexical_delta"],
            min_content_delta=thresholds["content_delta"],
            min_recent_delta=thresholds["recent_delta"],
            min_global_delta=thresholds["global_delta"],
            label=f"{label}.repo_macro_average",
        )

    require_weight_sweep(failures, holdout, EXPECTED_HYBRID_WEIGHT_SWEEP, label)
    require_default_weight_alignment(
        failures,
        holdout,
        weight=EXPECTED_RELATED_HYBRID_DEFAULT_WEIGHT,
        metric="mean_average_precision_at_5",
        label=label,
    )
    require_weight_is_sweep_best(
        failures,
        holdout,
        weight=EXPECTED_RELATED_HYBRID_DEFAULT_WEIGHT,
        metric="mean_average_precision_at_5",
        label=label,
    )
    require_weight_ap(
        failures,
        holdout,
        weight=EXPECTED_RELATED_HYBRID_DEFAULT_WEIGHT,
        metric="mean_average_precision_at_5",
        minimum=thresholds["default_weight_ap"],
        label=label,
    )
    require_loro_thresholds(
        failures,
        holdout,
        min_ap=thresholds["loro_ap"],
        min_direct_delta=thresholds["direct_delta"],
        min_lexical_delta=thresholds["lexical_delta"],
        min_content_delta=thresholds["content_delta"],
        min_recent_delta=thresholds["recent_delta"],
        min_global_delta=thresholds["global_delta"],
        min_pagerank_delta=thresholds["pagerank_delta"],
        max_holm_p=MAX_HOLDOUT_HOLM_P,
        label=label,
    )


def require_count(
    failures: list[str],
    measurement: dict[str, Any],
    key: str,
    minimum: int,
    label: str,
) -> None:
    value = int(measurement.get(key, 0))
    if value < minimum:
        failures.append(f"{label}.{key} < {minimum}: {value}")


def require_temporal_leakage_audit(
    failures: list[str],
    holdout: dict[str, Any],
    label: str,
) -> None:
    audit = holdout.get("temporal_leakage_audit")
    if not isinstance(audit, dict):
        failures.append(f"{label} missing temporal_leakage_audit")
        return
    case_count = int(holdout.get("case_count", 0))
    audit_case_count = int(audit.get("case_count", 0))
    checked = int(audit.get("checked_case_count", 0))
    matched = int(audit.get("head_matches_parent_count", 0))
    failures_count = int(audit.get("failure_count", 0))
    if audit_case_count != case_count:
        failures.append(
            f"{label}.temporal_leakage_audit case_count != holdout case_count: "
            f"{audit_case_count} != {case_count}"
        )
    if checked != case_count:
        failures.append(
            f"{label}.temporal_leakage_audit checked_case_count != case_count: "
            f"{checked} != {case_count}"
        )
    if matched != checked:
        failures.append(
            f"{label}.temporal_leakage_audit head_matches_parent_count != "
            f"checked_case_count: {matched} != {checked}"
        )
    if failures_count != 0:
        failures.append(
            f"{label}.temporal_leakage_audit failure_count != 0: {failures_count}"
        )


def check_repo_macro_thresholds(
    failures: list[str],
    macro: dict[str, Any],
    *,
    min_ap: float,
    min_direct_delta: float,
    min_pagerank_delta: float,
    min_lexical_delta: float,
    min_content_delta: float,
    min_recent_delta: float,
    min_global_delta: float,
    label: str,
) -> None:
    require_count(failures, macro, "repo_count", 3, label)
    aggregate = macro.get("aggregate", {})
    metric = "mean_average_precision_at_5"
    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid",
        metric,
        min_ap,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_direct",
        metric=metric,
        minimum=min_direct_delta,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_pagerank",
        metric=metric,
        minimum=min_pagerank_delta,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_lexical_similarity",
        metric=metric,
        minimum=min_lexical_delta,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_content_similarity",
        metric=metric,
        minimum=min_content_delta,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_recent_activity",
        metric=metric,
        minimum=min_recent_delta,
        label=label,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_global_pagerank",
        metric=metric,
        minimum=min_global_delta,
        label=label,
    )


def threshold_margin_entries(
    report: dict[str, Any],
    *,
    require_holdout: bool = False,
) -> list[dict[str, Any]]:
    lines: list[dict[str, Any]] = []
    retrieval = measurement_by_name(report, "retrieval_suite")
    if retrieval:
        aggregate = retrieval.get("aggregate", {})
        append_mean_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_related_hybrid.mean_recall_at_5",
            "workspace_related_hybrid",
            "mean_recall_at_5",
            0.99,
        )
        append_mean_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_related_hybrid.mean_average_precision_at_5",
            "workspace_related_hybrid",
            "mean_average_precision_at_5",
            0.85,
        )
        append_mean_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_impact_hybrid.mean_recall_at_5",
            "workspace_impact_hybrid",
            "mean_recall_at_5",
            0.99,
        )
        append_mean_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_impact_hybrid.mean_average_precision_at_5",
            "workspace_impact_hybrid",
            "mean_average_precision_at_5",
            0.95,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_related_hybrid_minus_workspace_related_direct"
            ".mean_average_precision_at_5",
            "workspace_related_hybrid",
            "workspace_related_direct",
            "mean_average_precision_at_5",
            0.30,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_related_hybrid_minus_baseline_lexical_similarity"
            ".mean_average_precision_at_5",
            "workspace_related_hybrid",
            "baseline_lexical_similarity",
            "mean_average_precision_at_5",
            0.25,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_related_hybrid_minus_baseline_content_similarity"
            ".mean_average_precision_at_5",
            "workspace_related_hybrid",
            "baseline_content_similarity",
            "mean_average_precision_at_5",
            0.25,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_impact_hybrid_minus_workspace_impact_direct"
            ".mean_average_precision_at_5",
            "workspace_impact_hybrid",
            "workspace_impact_direct",
            "mean_average_precision_at_5",
            0.40,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_impact_hybrid_minus_baseline_lexical_similarity"
            ".mean_average_precision_at_5",
            "workspace_impact_hybrid",
            "baseline_lexical_similarity",
            "mean_average_precision_at_5",
            0.35,
        )
        append_delta_margin(
            lines,
            aggregate,
            "retrieval_suite.workspace_impact_hybrid_minus_baseline_content_similarity"
            ".mean_average_precision_at_5",
            "workspace_impact_hybrid",
            "baseline_content_similarity",
            "mean_average_precision_at_5",
            0.30,
        )

    holdout = measurement_by_name(report, "repo_temporal_holdout_aggregate")
    if holdout:
        lines.extend(repo_holdout_margin_report(holdout, predictable=False))
        predictable = holdout.get("predictable_only")
        if isinstance(predictable, dict):
            lines.extend(repo_holdout_margin_report(predictable, predictable=True))
    elif require_holdout:
        lines.append(
            {
                "label": "repo_temporal_holdout_aggregate",
                "status": "missing",
                "missing": True,
            }
        )
    return lines


def threshold_margin_report(
    report: dict[str, Any],
    *,
    require_holdout: bool = False,
) -> list[str]:
    return [
        format_threshold_margin_entry(entry)
        for entry in threshold_margin_entries(
            report,
            require_holdout=require_holdout,
        )
    ]


def repo_holdout_margin_report(
    holdout: dict[str, Any],
    *,
    predictable: bool,
) -> list[dict[str, Any]]:
    label = "predictable repo_temporal_holdout_aggregate" if predictable else (
        "repo_temporal_holdout_aggregate"
    )
    thresholds = repo_holdout_thresholds(predictable)
    aggregate = holdout.get("aggregate", {})
    lines: list[dict[str, Any]] = []
    if not predictable:
        append_count_margin(lines, f"{label}.repo_count", holdout.get("repo_count"), 3)
    append_count_margin(lines, f"{label}.case_count", holdout.get("case_count"), 45)
    append_count_margin(
        lines,
        f"{label}.target_count",
        holdout.get("target_count"),
        180,
    )
    append_mean_margin(
        lines,
        aggregate,
        f"{label}.workspace_related_hybrid.mean_average_precision_at_5",
        "workspace_related_hybrid",
        "mean_average_precision_at_5",
        thresholds["hybrid_ap"],
    )
    append_mean_margin(
        lines,
        aggregate,
        f"{label}.history_oracle_ceiling.mean_average_precision_at_5",
        "history_oracle_ceiling",
        "mean_average_precision_at_5",
        0.75,
    )
    append_holdout_delta_margins(
        lines,
        aggregate,
        f"{label}.workspace_related_hybrid",
        "workspace_related_hybrid",
        "mean_average_precision_at_5",
        thresholds,
        HOLDOUT_DELTA_THRESHOLDS,
    )
    append_oracle_margin(
        lines,
        aggregate,
        f"{label}.workspace_related_hybrid.oracle_normalized_average_precision_at_5",
        "workspace_related_hybrid",
        "history_oracle_ceiling",
        "mean_average_precision_at_5",
        thresholds["oracle_normalized"],
    )
    append_ceiling_margin(
        lines,
        f"{label}.paired_deltas.max_holm_p_greater_average_precision_at_5",
        max_holm_p_value(
            holdout.get("paired_deltas", {}),
            HOLDOUT_HOLM_COMPARISONS,
            "average_precision_at_5",
        ),
        MAX_HOLDOUT_HOLM_P,
    )
    repo_macro = holdout.get("repo_macro_average")
    if isinstance(repo_macro, dict):
        append_count_margin(
            lines,
            f"{label}.repo_macro_average.repo_count",
            repo_macro.get("repo_count"),
            3,
        )
        append_mean_margin(
            lines,
            repo_macro.get("aggregate", {}),
            f"{label}.repo_macro_average.workspace_related_hybrid"
            ".mean_average_precision_at_5",
            "workspace_related_hybrid",
            "mean_average_precision_at_5",
            thresholds["macro_hybrid_ap"],
        )
        append_holdout_delta_margins(
            lines,
            repo_macro.get("aggregate", {}),
            f"{label}.repo_macro_average.workspace_related_hybrid",
            "workspace_related_hybrid",
            "mean_average_precision_at_5",
            thresholds,
            [
                ("workspace_related_direct", "macro_direct_delta"),
                ("workspace_related_pagerank", "macro_pagerank_delta"),
                ("baseline_lexical_similarity", "lexical_delta"),
                ("baseline_content_similarity", "content_delta"),
                ("baseline_recent_activity", "recent_delta"),
                ("baseline_global_pagerank", "global_delta"),
            ],
        )
    append_weight_margin(
        lines,
        holdout,
        label,
        EXPECTED_RELATED_HYBRID_DEFAULT_WEIGHT,
        "mean_average_precision_at_5",
        thresholds["default_weight_ap"],
    )
    loro = holdout.get("leave_one_repo_out_weight_selection")
    if isinstance(loro, dict):
        append_count_margin(
            lines,
            f"{label}.leave_one_repo_out.selection_count",
            len(loro.get("selections", [])),
            3,
        )
        append_mean_margin(
            lines,
            loro.get("aggregate", {}),
            f"{label}.leave_one_repo_out.workspace_related_hybrid_loro"
            ".mean_average_precision_at_5",
            "workspace_related_hybrid_loro",
            "mean_average_precision_at_5",
            thresholds["loro_ap"],
        )
        append_holdout_delta_margins(
            lines,
            loro.get("aggregate", {}),
            f"{label}.leave_one_repo_out.workspace_related_hybrid_loro",
            "workspace_related_hybrid_loro",
            "mean_average_precision_at_5",
            thresholds,
            HOLDOUT_DELTA_THRESHOLDS,
        )
        append_ceiling_margin(
            lines,
            f"{label}.leave_one_repo_out.max_holm_p_greater_average_precision_at_5",
            max_holm_p_value(
                loro.get("paired_deltas", {}),
                LORO_HOLM_COMPARISONS,
                "average_precision_at_5",
            ),
            MAX_HOLDOUT_HOLM_P,
        )
    return lines


def append_holdout_delta_margins(
    lines: list[dict[str, Any]],
    aggregate: dict[str, Any],
    label_prefix: str,
    left: str,
    metric: str,
    thresholds: dict[str, float],
    comparisons: list[tuple[str, str]],
) -> None:
    for right, threshold_key in comparisons:
        append_delta_margin(
            lines,
            aggregate,
            f"{label_prefix}_minus_{right}.{metric}",
            left,
            right,
            metric,
            thresholds[threshold_key],
        )


def append_mean_margin(
    lines: list[dict[str, Any]],
    aggregate: dict[str, Any],
    label: str,
    method: str,
    metric: str,
    minimum: float,
) -> None:
    summary = aggregate.get(method)
    if not isinstance(summary, dict) or metric not in summary:
        return
    append_floor_margin(lines, label, float(summary[metric]), minimum)


def append_delta_margin(
    lines: list[dict[str, Any]],
    aggregate: dict[str, Any],
    label: str,
    left: str,
    right: str,
    metric: str,
    minimum: float,
) -> None:
    left_summary = aggregate.get(left)
    right_summary = aggregate.get(right)
    if not isinstance(left_summary, dict) or not isinstance(right_summary, dict):
        return
    if metric not in left_summary or metric not in right_summary:
        return
    value = float(left_summary[metric]) - float(right_summary[metric])
    append_floor_margin(lines, label, value, minimum)


def append_weight_margin(
    lines: list[dict[str, Any]],
    holdout: dict[str, Any],
    label: str,
    weight: float,
    metric: str,
    minimum: float,
) -> None:
    value = weight_sweep_value(holdout, weight=weight, metric=metric)
    if value is None:
        return
    append_floor_margin(
        lines,
        f"{label}.hybrid_weight_sweep[{weight:g}].{metric}",
        value,
        minimum,
    )
    default_summary = holdout.get("aggregate", {}).get("workspace_related_hybrid", {})
    if metric in default_summary:
        append_ceiling_margin(
            lines,
            f"{label}.hybrid_weight_sweep[{weight:g}].default_alignment_abs_delta_"
            f"{metric}",
            abs(float(default_summary[metric]) - value),
            0.001,
        )
    append_ceiling_margin(
        lines,
        f"{label}.hybrid_weight_sweep[{weight:g}].best_weight_advantage_{metric}",
        max_weight_sweep_advantage(holdout, weight=weight, metric=metric),
        0.001,
    )


def append_oracle_margin(
    lines: list[dict[str, Any]],
    aggregate: dict[str, Any],
    label: str,
    method: str,
    oracle: str,
    metric: str,
    minimum: float,
) -> None:
    method_summary = aggregate.get(method)
    oracle_summary = aggregate.get(oracle)
    if not isinstance(method_summary, dict) or not isinstance(oracle_summary, dict):
        return
    if metric not in method_summary or metric not in oracle_summary:
        return
    oracle_value = float(oracle_summary[metric])
    if oracle_value <= 0.0:
        return
    append_floor_margin(
        lines,
        label,
        float(method_summary[metric]) / oracle_value,
        minimum,
    )


def append_count_margin(
    lines: list[dict[str, Any]],
    label: str,
    value: Any,
    minimum: int,
) -> None:
    if value is None:
        return
    count = int(value)
    margin = count - minimum
    lines.append(
        {
            "label": label,
            "value": count,
            "minimum": minimum,
            "margin": margin,
            "gate": "minimum",
            "kind": "count",
            "status": "pass" if margin >= 0 else "fail",
        }
    )


def append_floor_margin(
    lines: list[dict[str, Any]],
    label: str,
    value: float,
    minimum: float,
) -> None:
    margin = value - minimum
    lines.append(
        {
            "label": label,
            "value": value,
            "minimum": minimum,
            "margin": margin,
            "gate": "minimum",
            "kind": "floor",
            "status": "pass" if margin + FLOAT_TOLERANCE >= 0.0 else "fail",
        }
    )


def append_ceiling_margin(
    lines: list[dict[str, Any]],
    label: str,
    value: float | None,
    maximum: float,
) -> None:
    if value is None:
        return
    headroom = maximum - value
    lines.append(
        {
            "label": label,
            "value": value,
            "maximum": maximum,
            "headroom": headroom,
            "gate": "maximum",
            "kind": "ceiling",
            "status": "pass" if headroom + FLOAT_TOLERANCE >= 0.0 else "fail",
        }
    )


def format_threshold_float(value: float, *, decimals: int, signed: bool = False) -> str:
    number = float(value)
    if number == 0.0:
        number = 0.0
    sign = "+" if signed else ""
    fixed = f"{number:{sign}.{decimals}f}"
    if number != 0.0 and float(fixed) == 0.0:
        return f"{number:{sign}.6g}"
    return fixed


def format_threshold_margin_entry(entry: dict[str, Any]) -> str:
    label = str(entry.get("label", ""))
    if entry.get("missing"):
        return f"{label}: missing"
    gate = entry.get("gate")
    if gate == "minimum":
        if entry.get("kind") == "count":
            return (
                f"{label}: value={int(entry['value'])}, "
                f"minimum={int(entry['minimum'])}, "
                f"margin={int(entry['margin']):+d}"
            )
        return (
            f"{label}: value={format_threshold_float(entry['value'], decimals=3)}, "
            f"minimum={format_threshold_float(entry['minimum'], decimals=3)}, "
            f"margin={format_threshold_float(entry['margin'], decimals=3, signed=True)}"
        )
    if gate == "maximum":
        return (
            f"{label}: value={format_threshold_float(entry['value'], decimals=4)}, "
            f"maximum={format_threshold_float(entry['maximum'], decimals=4)}, "
            f"headroom={format_threshold_float(entry['headroom'], decimals=4, signed=True)}"
        )
    return f"{label}: {entry.get('status', 'unknown')}"


def max_holm_p_value(
    deltas: dict[str, Any],
    comparisons: list[str],
    metric: str,
) -> float | None:
    key = f"p_greater_holm_delta_{metric}"
    values = []
    for comparison in comparisons:
        summary = deltas.get(comparison)
        if isinstance(summary, dict) and key in summary:
            values.append(float(summary[key]))
    return max(values) if values else None


def max_weight_sweep_advantage(
    holdout: dict[str, Any],
    *,
    weight: float,
    metric: str,
) -> float | None:
    selected_value = weight_sweep_value(holdout, weight=weight, metric=metric)
    if selected_value is None:
        return None
    advantages = []
    for entry in holdout.get("hybrid_weight_sweep", []):
        candidate_weight = float(entry.get("hybrid_direct_weight", -1.0))
        candidate_value = weight_sweep_value(
            holdout,
            weight=candidate_weight,
            metric=metric,
        )
        if candidate_value is not None:
            advantages.append(candidate_value - selected_value)
    return max(advantages) if advantages else None


def require_mean(
    failures: list[str],
    aggregate: dict[str, Any],
    method: str,
    metric: str,
    minimum: float,
    *,
    label: str | None = None,
) -> None:
    prefix = f"{label}." if label else ""
    summary = aggregate.get(method)
    if summary is None:
        failures.append(f"missing aggregate method: {prefix}{method}")
        return
    value = float(summary.get(metric, 0.0))
    if value < minimum:
        failures.append(f"{prefix}{method}.{metric} < {minimum}: {value}")


def require_delta(
    failures: list[str],
    aggregate: dict[str, Any],
    *,
    left: str,
    right: str,
    metric: str,
    minimum: float,
    label: str | None = None,
) -> None:
    prefix = f"{label}." if label else ""
    left_summary = aggregate.get(left)
    right_summary = aggregate.get(right)
    if left_summary is None or right_summary is None:
        failures.append(f"missing aggregate delta inputs: {prefix}{left} - {right}")
        return
    delta = float(left_summary.get(metric, 0.0)) - float(right_summary.get(metric, 0.0))
    if delta + FLOAT_TOLERANCE < minimum:
        failures.append(
            f"{prefix}{left}.{metric} - {right}.{metric} < {minimum}: {delta}"
        )


def require_oracle_normalized(
    failures: list[str],
    aggregate: dict[str, Any],
    *,
    method: str,
    oracle: str,
    metric: str,
    minimum: float,
    label: str | None = None,
) -> None:
    prefix = f"{label}." if label else ""
    method_summary = aggregate.get(method)
    oracle_summary = aggregate.get(oracle)
    if method_summary is None or oracle_summary is None:
        failures.append(
            f"missing oracle-normalized inputs: {prefix}{method} / {oracle}"
        )
        return
    method_value = float(method_summary.get(metric, 0.0))
    oracle_value = float(oracle_summary.get(metric, 0.0))
    if oracle_value <= 0.0:
        failures.append(
            f"{prefix}{oracle}.{metric} must be positive for oracle normalization"
        )
        return
    normalized = method_value / oracle_value
    if normalized < minimum:
        failures.append(
            f"{prefix}{method}.{metric} / {oracle}.{metric} < {minimum}: "
            f"{normalized}"
        )


def require_holm_evidence(
    failures: list[str],
    deltas: dict[str, Any],
    comparisons: list[str],
    *,
    metric: str,
    maximum: float,
    label: str,
) -> None:
    for comparison in comparisons:
        summary = deltas.get(comparison)
        if summary is None:
            failures.append(f"{label}.{comparison} missing paired delta evidence")
            continue
        key = f"p_greater_holm_delta_{metric}"
        if key not in summary:
            failures.append(f"{label}.{comparison} missing {key}")
            continue
        value = float(summary[key])
        if value > maximum:
            failures.append(f"{label}.{comparison}.{key} > {maximum}: {value}")


def require_weight_sweep(
    failures: list[str],
    holdout: dict[str, Any],
    expected_weights: list[float],
    label: str,
) -> None:
    require_weight_list(
        failures,
        [
            float(entry.get("hybrid_direct_weight"))
            for entry in holdout.get("hybrid_weight_sweep", [])
            if "hybrid_direct_weight" in entry
        ],
        expected_weights,
        f"{label}.hybrid_weight_sweep",
    )


def require_weight_list(
    failures: list[str],
    actual_weights: list[float],
    expected_weights: list[float],
    label: str,
) -> None:
    missing = [
        weight
        for weight in expected_weights
        if not any(abs(actual - weight) < 1e-9 for actual in actual_weights)
    ]
    if missing:
        failures.append(f"{label} missing weights: {missing}")


def require_weight_ap(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    weight: float,
    metric: str,
    minimum: float,
    label: str,
) -> None:
    entry = next(
        (
            item
            for item in holdout.get("hybrid_weight_sweep", [])
            if abs(float(item.get("hybrid_direct_weight", -1.0)) - weight) < 1e-9
        ),
        None,
    )
    if entry is None:
        failures.append(f"{label}.hybrid_weight_sweep missing weight {weight}")
        return
    related = entry.get("related", {})
    method = related.get(
        "method",
        hybrid_weight_method("workspace_related_hybrid", weight),
    )
    summary = related.get("aggregate", {}).get(method)
    if summary is None:
        failures.append(f"{label}.hybrid_weight_sweep[{weight}] missing method {method}")
        return
    value = float(summary.get(metric, 0.0))
    if value < minimum:
        failures.append(
            f"{label}.hybrid_weight_sweep[{weight}].{metric} < {minimum}: {value}"
        )


def weight_sweep_value(
    holdout: dict[str, Any],
    *,
    weight: float,
    metric: str,
) -> float | None:
    entry = next(
        (
            item
            for item in holdout.get("hybrid_weight_sweep", [])
            if abs(float(item.get("hybrid_direct_weight", -1.0)) - weight) < 1e-9
        ),
        None,
    )
    if entry is None:
        return None
    related = entry.get("related", {})
    method = related.get(
        "method",
        hybrid_weight_method("workspace_related_hybrid", weight),
    )
    summary = related.get("aggregate", {}).get(method)
    if not isinstance(summary, dict) or metric not in summary:
        return None
    return float(summary[metric])


def require_default_weight_alignment(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    weight: float,
    metric: str,
    label: str,
) -> None:
    default_summary = holdout.get("aggregate", {}).get("workspace_related_hybrid", {})
    if metric not in default_summary:
        failures.append(f"{label}.workspace_related_hybrid missing {metric}")
        return
    default_value = float(default_summary[metric])
    sweep_value = weight_sweep_value(holdout, weight=weight, metric=metric)
    if sweep_value is None:
        failures.append(f"{label}.hybrid_weight_sweep[{weight}] missing {metric}")
        return
    if abs(default_value - sweep_value) > 0.001:
        failures.append(
            f"{label}.workspace_related_hybrid.{metric} must match "
            f"hybrid_weight_sweep[{weight}]: {default_value} != {sweep_value}"
        )


def require_weight_is_sweep_best(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    weight: float,
    metric: str,
    label: str,
) -> None:
    selected_value = weight_sweep_value(holdout, weight=weight, metric=metric)
    if selected_value is None:
        failures.append(f"{label}.hybrid_weight_sweep[{weight}] missing {metric}")
        return
    for entry in holdout.get("hybrid_weight_sweep", []):
        candidate_weight = float(entry.get("hybrid_direct_weight", -1.0))
        candidate_value = weight_sweep_value(
            holdout,
            weight=candidate_weight,
            metric=metric,
        )
        if candidate_value is None:
            continue
        if candidate_value > selected_value + 0.001:
            failures.append(
                f"{label}.hybrid_weight_sweep[{weight}].{metric} "
                f"is below weight {candidate_weight}: "
                f"{selected_value} < {candidate_value}"
            )


def require_loro_thresholds(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    min_ap: float,
    min_direct_delta: float,
    min_lexical_delta: float,
    min_content_delta: float,
    min_recent_delta: float,
    min_global_delta: float,
    min_pagerank_delta: float,
    max_holm_p: float,
    label: str,
) -> None:
    loro = holdout.get("leave_one_repo_out_weight_selection")
    if not isinstance(loro, dict):
        failures.append(f"{label} missing leave_one_repo_out_weight_selection")
        return
    selections = loro.get("selections", [])
    if len(selections) < 3:
        failures.append(f"{label}.leave_one_repo_out_weight_selection selections < 3")
    require_weight_list(
        failures,
        [float(weight) for weight in loro.get("candidate_weights", [])],
        EXPECTED_HYBRID_WEIGHT_SWEEP,
        f"{label}.leave_one_repo_out_weight_selection.candidate_weights",
    )
    aggregate = loro.get("aggregate", {})
    ap_metric = "mean_average_precision_at_5"
    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid_loro",
        ap_metric,
        min_ap,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="workspace_related_direct",
        metric=ap_metric,
        minimum=min_direct_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="workspace_related_pagerank",
        metric=ap_metric,
        minimum=min_pagerank_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="baseline_lexical_similarity",
        metric=ap_metric,
        minimum=min_lexical_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="baseline_content_similarity",
        metric=ap_metric,
        minimum=min_content_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="baseline_recent_activity",
        metric=ap_metric,
        minimum=min_recent_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="baseline_global_pagerank",
        metric=ap_metric,
        minimum=min_global_delta,
    )
    require_holm_evidence(
        failures,
        loro.get("paired_deltas", {}),
        LORO_HOLM_COMPARISONS,
        metric="average_precision_at_5",
        maximum=max_holm_p,
        label=f"{label}.leave_one_repo_out_weight_selection",
    )


def hybrid_weight_method(prefix: str, weight: float) -> str:
    return f"{prefix}_w_{weight:g}".replace(".", "_")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--require-holdout",
        action="store_true",
        help="fail when the report does not include cross-repo temporal holdouts",
    )
    parser.add_argument(
        "report",
        nargs="?",
        default="-",
        help="effect measurement JSON path; reads stdin when omitted or '-'",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    report = load_report(args.report)
    failures = check_report(
        report,
        require_holdout=args.require_holdout,
    )
    if failures:
        print("effect threshold check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        raise SystemExit(1)
    print(
        render_success_output(
            report,
            require_holdout=args.require_holdout,
        ),
        end="",
    )


def render_success_output(
    report: dict[str, Any],
    *,
    require_holdout: bool = False,
) -> str:
    lines = ["effect threshold check passed"]
    margin_lines = threshold_margin_report(
        report,
        require_holdout=require_holdout,
    )
    if margin_lines:
        lines.append("effect threshold margins:")
        for line in margin_lines:
            lines.append(f"- {line}")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    main()
