#!/usr/bin/env python3
"""Fail CI when effect measurement drops below expected fixture thresholds."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


EXPECTED_SIGN_FLIP_METHOD = "exact_grid_dp_with_sampled_fallback"


def load_report(path: str) -> dict[str, Any]:
    if path == "-":
        return json.load(sys.stdin)
    return json.loads(Path(path).read_text())


def measurement_by_name(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    for measurement in report.get("measurements", []):
        if measurement.get("metric") == name:
            return measurement
    return None


def check_report(report: dict[str, Any]) -> list[str]:
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
        left="workspace_impact_hybrid",
        right="workspace_impact_direct",
        metric="mean_average_precision_at_5",
        minimum=0.40,
    )
    return failures


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
    failures = check_report(load_report(parse_args().report))
    if failures:
        print("effect threshold check failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        raise SystemExit(1)
    print("effect threshold check passed")


if __name__ == "__main__":
    main()
