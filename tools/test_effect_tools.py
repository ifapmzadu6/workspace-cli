#!/usr/bin/env python3
"""Unit tests for effect measurement helpers."""

from __future__ import annotations

import importlib.util
import itertools
import json
import subprocess
import tempfile
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
check_effect_thresholds = load_tool("check_effect_thresholds")
run_effect_artifacts = load_tool("run_effect_artifacts")


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


class StaticBaselineTests(unittest.TestCase):
    def test_path_tokens_split_names_and_ignore_structural_tokens(self) -> None:
        self.assertEqual(
            measure_effect.path_tokens("src/authCookie_test.rs"),
            {"auth", "cookie"},
        )

    def test_lexical_similarity_ranks_name_overlap_without_seed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            measure_effect.git(root, "init", "-q")
            for path in [
                "src/auth.rs",
                "src/session.rs",
                "tests/auth_test.rs",
                "docs/auth.md",
            ]:
                measure_effect.write(root / path, f"{path}\n")
            measure_effect.git(root, "add", ".")

            ranked = measure_effect.lexical_similarity_paths(root, {"src/auth.rs"})

        self.assertNotIn("src/auth.rs", ranked)
        self.assertEqual(ranked[:2], ["docs/auth.md", "tests/auth_test.rs"])


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


class EffectThresholdTests(unittest.TestCase):
    def weight_sweep(self, ap: float) -> list[dict]:
        entries = []
        for weight in check_effect_thresholds.EXPECTED_HYBRID_WEIGHT_SWEEP:
            method = check_effect_thresholds.hybrid_weight_method(
                "workspace_related_hybrid",
                weight,
            )
            entries.append(
                {
                    "hybrid_direct_weight": weight,
                    "related": {
                        "method": method,
                        "aggregate": {
                            method: {
                                "mean_average_precision_at_5": ap,
                            },
                        },
                    },
                }
            )
        return entries

    def loro_selection(self, ap: float, direct_ap: float, lexical_ap: float) -> dict:
        return {
            "candidate_weights": check_effect_thresholds.EXPECTED_HYBRID_WEIGHT_SWEEP,
            "selections": [{}, {}, {}],
            "aggregate": {
                "workspace_related_direct": {
                    "mean_average_precision_at_5": direct_ap,
                },
                "baseline_lexical_similarity": {
                    "mean_average_precision_at_5": lexical_ap,
                },
                "workspace_related_hybrid_loro": {
                    "mean_average_precision_at_5": ap,
                },
            },
        }

    def repo_holdout(self, *, predictable: bool) -> dict:
        if predictable:
            return {
                "case_count": 48,
                "target_count": 190,
                "aggregate": {
                    "workspace_related_direct": {
                        "mean_average_precision_at_5": 0.62,
                    },
                    "baseline_lexical_similarity": {
                        "mean_average_precision_at_5": 0.20,
                    },
                    "workspace_related_pagerank": {
                        "mean_average_precision_at_5": 0.60,
                    },
                    "workspace_related_hybrid": {
                        "mean_average_precision_at_5": 0.72,
                    },
                    "history_oracle_ceiling": {
                        "mean_average_precision_at_5": 0.90,
                    },
                },
                "hybrid_weight_sweep": self.weight_sweep(0.72),
                "leave_one_repo_out_weight_selection": self.loro_selection(
                    0.71,
                    0.62,
                    0.20,
                ),
            }
        return {
            "metric": "repo_temporal_holdout_aggregate",
            "repo_count": 3,
            "case_count": 50,
            "target_count": 207,
            "aggregate": {
                "workspace_related_direct": {
                    "mean_average_precision_at_5": 0.56,
                },
                "baseline_lexical_similarity": {
                    "mean_average_precision_at_5": 0.20,
                },
                "workspace_related_pagerank": {
                    "mean_average_precision_at_5": 0.53,
                },
                "workspace_related_hybrid": {
                    "mean_average_precision_at_5": 0.64,
                },
                "history_oracle_ceiling": {
                    "mean_average_precision_at_5": 0.81,
                },
            },
            "hybrid_weight_sweep": self.weight_sweep(0.64),
            "leave_one_repo_out_weight_selection": self.loro_selection(
                0.63,
                0.56,
                0.20,
            ),
            "predictable_only": self.repo_holdout(predictable=True),
        }

    def passing_report(self) -> dict:
        return {
            "metadata": {
                "sign_flip_method": "exact_grid_dp_with_sampled_fallback",
            },
            "measurements": [
                {
                    "metric": "map_fact_recall",
                    "recall": 1.0,
                },
                {
                    "metric": "transaction_audit_signal_recall",
                    "recall": 1.0,
                },
                {
                    "metric": "retrieval_suite",
                    "scenario_count": 4,
                    "aggregate": {
                        "workspace_related_direct": {
                            "mean_average_precision_at_5": 0.50,
                        },
                        "baseline_lexical_similarity": {
                            "mean_average_precision_at_5": 0.40,
                        },
                        "workspace_related_hybrid": {
                            "mean_recall_at_5": 1.0,
                            "mean_average_precision_at_5": 0.90,
                        },
                        "workspace_impact_direct": {
                            "mean_average_precision_at_5": 0.50,
                        },
                        "workspace_impact_hybrid": {
                            "mean_recall_at_5": 1.0,
                            "mean_average_precision_at_5": 1.0,
                        },
                    },
                },
                self.repo_holdout(predictable=False),
            ],
        }

    def test_effect_thresholds_pass_for_expected_fixture_floor(self) -> None:
        self.assertEqual(
            check_effect_thresholds.check_report(self.passing_report()),
            [],
        )

    def test_effect_thresholds_allow_missing_holdout_by_default(self) -> None:
        report = self.passing_report()
        report["measurements"] = report["measurements"][:-1]
        self.assertEqual(check_effect_thresholds.check_report(report), [])

    def test_effect_thresholds_can_require_holdout_measurements(self) -> None:
        report = self.passing_report()
        report["measurements"] = report["measurements"][:-1]
        failures = check_effect_thresholds.check_report(report, require_holdout=True)
        self.assertIn("missing repo_temporal_holdout_aggregate measurement", failures)

    def test_effect_thresholds_fail_for_degraded_related_hybrid(self) -> None:
        report = self.passing_report()
        report["measurements"][2]["aggregate"]["workspace_related_hybrid"][
            "mean_average_precision_at_5"
        ] = 0.70
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any(
                "workspace_related_hybrid.mean_average_precision_at_5" in item
                for item in failures
            ),
            failures,
        )

    def test_effect_thresholds_fail_when_lexical_baseline_catches_hybrid(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["aggregate"]["baseline_lexical_similarity"][
            "mean_average_precision_at_5"
        ] = 0.40
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("baseline_lexical_similarity" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_when_dense_holdout_sweep_is_missing(self) -> None:
        report = self.passing_report()
        holdout = report["measurements"][-1]
        holdout["hybrid_weight_sweep"] = [
            entry
            for entry in holdout["hybrid_weight_sweep"]
            if entry["hybrid_direct_weight"] != 0.75
        ]
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("hybrid_weight_sweep missing weights" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_for_degraded_holdout_loro(self) -> None:
        report = self.passing_report()
        holdout = report["measurements"][-1]
        holdout["leave_one_repo_out_weight_selection"]["aggregate"][
            "workspace_related_hybrid_loro"
        ]["mean_average_precision_at_5"] = 0.50
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any(
                "workspace_related_hybrid_loro.mean_average_precision_at_5" in item
                for item in failures
            ),
            failures,
        )


class EffectArtifactRunnerTests(unittest.TestCase):
    def test_paper_plan_uses_manifest_and_requires_holdout_thresholds(self) -> None:
        plan = run_effect_artifacts.build_plan(
            Path("target/test-effect-artifacts"),
            run_effect_artifacts.DEFAULT_PAPER_MANIFEST,
        )
        self.assertIn("--repo-holdout-manifest", plan["measurement_command"])
        self.assertIn("--require-holdout", plan["threshold_command"])
        self.assertEqual(plan["json_path"].name, "effect.json")
        self.assertEqual(plan["markdown_path"].name, "effect.md")
        self.assertEqual(plan["threshold_path"].name, "thresholds.txt")

    def test_fixture_plan_keeps_holdout_thresholds_optional(self) -> None:
        plan = run_effect_artifacts.build_plan(
            Path("target/test-effect-artifacts"),
            None,
        )
        self.assertNotIn("--repo-holdout-manifest", plan["measurement_command"])
        self.assertNotIn("--require-holdout", plan["threshold_command"])

    def test_artifact_runner_writes_all_outputs_and_run_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            plan = run_effect_artifacts.build_plan(output_dir, None)
            calls = []

            def fake_runner(command, **kwargs):
                calls.append(command)
                stdout = kwargs.get("stdout")
                if stdout is not None:
                    stdout.write("{}\n")
                return subprocess.CompletedProcess(command, 0)

            run_effect_artifacts.run_plan(plan, runner=fake_runner)

            self.assertEqual(calls, [
                plan["measurement_command"],
                plan["threshold_command"],
                plan["summary_command"],
            ])
            self.assertTrue(plan["json_path"].exists())
            self.assertTrue(plan["markdown_path"].exists())
            self.assertTrue(plan["threshold_path"].exists())
            run_manifest = json.loads(plan["run_manifest_path"].read_text())
            self.assertEqual(
                run_manifest["commands"]["measure"],
                plan["measurement_command"],
            )
            self.assertFalse(run_manifest["require_holdout_thresholds"])


if __name__ == "__main__":
    unittest.main()
