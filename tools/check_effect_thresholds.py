#!/usr/bin/env python3
"""Fail CI when effect measurement drops below expected fixture thresholds."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_SIGN_FLIP_METHOD = "exact_grid_dp_with_sampled_fallback"
EXPECTED_HYBRID_WEIGHT_SWEEP = [0.0, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0]


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
    require_count(failures, holdout, "case_count", 45, label)
    require_count(failures, holdout, "target_count", 180, label)

    aggregate = holdout.get("aggregate", {})
    min_hybrid_ap = 0.70 if predictable else 0.63
    min_direct_delta = 0.08 if predictable else 0.05
    min_lexical_delta = 0.35 if predictable else 0.30
    min_pagerank_delta = 0.09 if predictable else 0.08
    min_oracle_normalized = 0.75
    min_loro_ap = 0.70 if predictable else 0.62

    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid",
        "mean_average_precision_at_5",
        min_hybrid_ap,
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
        metric="mean_average_precision_at_5",
        minimum=min_direct_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="baseline_lexical_similarity",
        metric="mean_average_precision_at_5",
        minimum=min_lexical_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid",
        right="workspace_related_pagerank",
        metric="mean_average_precision_at_5",
        minimum=min_pagerank_delta,
    )
    require_oracle_normalized(
        failures,
        aggregate,
        method="workspace_related_hybrid",
        oracle="history_oracle_ceiling",
        metric="mean_average_precision_at_5",
        minimum=min_oracle_normalized,
    )

    require_weight_sweep(failures, holdout, EXPECTED_HYBRID_WEIGHT_SWEEP, label)
    require_weight_ap(
        failures,
        holdout,
        weight=0.75,
        metric="mean_average_precision_at_5",
        minimum=0.70 if predictable else 0.63,
        label=label,
    )
    require_loro_thresholds(
        failures,
        holdout,
        min_ap=min_loro_ap,
        min_direct_delta=min_direct_delta,
        min_lexical_delta=min_lexical_delta,
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


def require_mean(
    failures: list[str],
    aggregate: dict[str, Any],
    method: str,
    metric: str,
    minimum: float,
) -> None:
    summary = aggregate.get(method)
    if summary is None:
        failures.append(f"missing aggregate method: {method}")
        return
    value = float(summary.get(metric, 0.0))
    if value < minimum:
        failures.append(f"{method}.{metric} < {minimum}: {value}")


def require_delta(
    failures: list[str],
    aggregate: dict[str, Any],
    *,
    left: str,
    right: str,
    metric: str,
    minimum: float,
) -> None:
    left_summary = aggregate.get(left)
    right_summary = aggregate.get(right)
    if left_summary is None or right_summary is None:
        failures.append(f"missing aggregate delta inputs: {left} - {right}")
        return
    delta = float(left_summary.get(metric, 0.0)) - float(right_summary.get(metric, 0.0))
    if delta < minimum:
        failures.append(f"{left}.{metric} - {right}.{metric} < {minimum}: {delta}")


def require_oracle_normalized(
    failures: list[str],
    aggregate: dict[str, Any],
    *,
    method: str,
    oracle: str,
    metric: str,
    minimum: float,
) -> None:
    method_summary = aggregate.get(method)
    oracle_summary = aggregate.get(oracle)
    if method_summary is None or oracle_summary is None:
        failures.append(f"missing oracle-normalized inputs: {method} / {oracle}")
        return
    method_value = float(method_summary.get(metric, 0.0))
    oracle_value = float(oracle_summary.get(metric, 0.0))
    if oracle_value <= 0.0:
        failures.append(f"{oracle}.{metric} must be positive for oracle normalization")
        return
    normalized = method_value / oracle_value
    if normalized < minimum:
        failures.append(
            f"{method}.{metric} / {oracle}.{metric} < {minimum}: {normalized}"
        )


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


def require_loro_thresholds(
    failures: list[str],
    holdout: dict[str, Any],
    *,
    min_ap: float,
    min_direct_delta: float,
    min_lexical_delta: float,
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
    require_mean(
        failures,
        aggregate,
        "workspace_related_hybrid_loro",
        "mean_average_precision_at_5",
        min_ap,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="workspace_related_direct",
        metric="mean_average_precision_at_5",
        minimum=min_direct_delta,
    )
    require_delta(
        failures,
        aggregate,
        left="workspace_related_hybrid_loro",
        right="baseline_lexical_similarity",
        metric="mean_average_precision_at_5",
        minimum=min_lexical_delta,
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
    failures = check_report(
        load_report(args.report),
        require_holdout=args.require_holdout,
    )
    if failures:
        print("effect threshold check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        raise SystemExit(1)
    print("effect threshold check passed")


if __name__ == "__main__":
    main()
