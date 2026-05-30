#!/usr/bin/env python3
"""Unit tests for effect measurement helpers."""

from __future__ import annotations

import importlib.util
import itertools
import unittest
from pathlib import Path


TOOLS_DIR = Path(__file__).resolve().parent


def load_tool(name: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS_DIR / f"{name}.py")
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


measure_effect = load_tool("measure_effect")
summarize_effect = load_tool("summarize_effect")


class ExactSignFlipTests(unittest.TestCase):
    def brute_force_p_values(self, values: list[float]) -> tuple[float, float]:
        observed = sum(values) / len(values)
        observed_abs = abs(observed)
        absolute_values = [abs(value) for value in values]
        greater_or_equal = 0
        two_sided_or_equal = 0
        total = 0
        for signs in itertools.product([-1, 1], repeat=len(values)):
            total += 1
            signed_mean = sum(
                sign * value for sign, value in zip(signs, absolute_values)
            ) / len(values)
            if signed_mean >= observed - 1e-12:
                greater_or_equal += 1
            if abs(signed_mean) >= observed_abs - 1e-12:
                two_sided_or_equal += 1
        return (greater_or_equal / total, two_sided_or_equal / total)

    def test_grid_sign_flip_matches_bruteforce(self) -> None:
        values = [0.125, -0.25, 0.5, 0.0]
        self.assertEqual(
            measure_effect.exact_grid_sign_flip_p_values(values),
            self.brute_force_p_values(values),
        )

    def test_grid_sign_flip_keeps_tiny_exact_p_values_nonzero(self) -> None:
        p_greater, p_two_sided = measure_effect.paired_sign_flip_p_values(
            [1.0] * 20,
            "all_positive",
            "average_precision_at_5",
        )
        self.assertGreater(p_greater, 0.0)
        self.assertLess(p_greater, 0.0001)
        self.assertLess(p_greater, p_two_sided)

    def test_grid_sign_flip_declines_unrounded_or_too_large_state_spaces(self) -> None:
        self.assertIsNone(measure_effect.exact_grid_sign_flip_p_values([1.0 / 3.0]))
        self.assertIsNone(measure_effect.exact_grid_sign_flip_p_values([100.0, 1.0]))


class PValueAdjustmentTests(unittest.TestCase):
    def test_holm_adjusted_p_values_preserve_input_order(self) -> None:
        self.assertEqual(
            measure_effect.holm_adjusted_p_values([0.01, 0.04, 0.03]),
            [0.03, 0.06, 0.06],
        )

    def test_holm_adjusted_p_values_are_monotonic_over_ties(self) -> None:
        self.assertEqual(
            measure_effect.holm_adjusted_p_values([0.5, 0.01, 0.02, 0.02]),
            [0.5, 0.04, 0.06, 0.06],
        )


class HoldoutOracleTests(unittest.TestCase):
    def test_history_oracle_ceiling_retargets_to_predictable_slice(self) -> None:
        case = {
            "expected": ["existing.rs", "new.rs"],
            "predictable_expected": ["existing.rs"],
            "methods": {
                "history_oracle_ceiling": measure_effect.ranking_metrics(
                    ["existing.rs"],
                    {"existing.rs", "new.rs"},
                    5,
                ),
                "workspace_related_hybrid": measure_effect.ranking_metrics(
                    ["existing.rs"],
                    {"existing.rs", "new.rs"},
                    5,
                ),
            },
        }

        all_targets = measure_effect.repo_holdout_metric_summary([case], 5, [])
        oracle = all_targets["aggregate"]["history_oracle_ceiling"]
        self.assertEqual(oracle["mean_recall_at_5"], 0.5)
        self.assertEqual(oracle["mean_average_precision_at_5"], 0.5)

        predictable = measure_effect.repo_holdout_metric_summary(
            [case],
            5,
            [],
            expected_key="predictable_expected",
        )
        oracle = predictable["aggregate"]["history_oracle_ceiling"]
        self.assertEqual(oracle["mean_recall_at_5"], 1.0)
        self.assertEqual(oracle["mean_average_precision_at_5"], 1.0)


class SummaryFormattingTests(unittest.TestCase):
    def test_small_p_values_render_without_rounding_to_zero(self) -> None:
        self.assertEqual(
            summarize_effect.fmt_p_value({"p": 1.0 / (2**20)}, "p"),
            "<0.0001",
        )
        self.assertEqual(summarize_effect.fmt_p_value({"p": 0.0}, "p"), "0.0000")
        self.assertEqual(summarize_effect.fmt_p_value({}, "p"), "")

    def test_sign_flip_method_metadata_is_optional(self) -> None:
        legacy = summarize_effect.render_metadata_table(
            {"metadata": {"workspace_bin": "target/debug/workspace"}}
        )
        self.assertNotIn("sign-flip method", legacy)

        current = summarize_effect.render_metadata_table(
            {
                "metadata": {
                    "workspace_bin": "target/debug/workspace",
                    "sign_flip_method": "exact_grid_dp_with_sampled_fallback",
                }
            }
        )
        self.assertIn("sign-flip method", current)

    def test_oracle_normalized_table_reports_gap(self) -> None:
        table = summarize_effect.render_oracle_normalized_table(
            {
                "k": 5,
                "aggregate": {
                    "history_oracle_ceiling": {
                        "mean_average_precision_at_5": 0.8,
                    },
                    "workspace_related_hybrid": {
                        "mean_average_precision_at_5": 0.6,
                    },
                },
            },
            "Holdout",
            ["workspace_related_hybrid"],
        )
        self.assertIn("AP/oracle", table)
        self.assertIn("| related hybrid | 0.600 | 0.800 | 0.750 | 0.200 |", table)


if __name__ == "__main__":
    unittest.main()
