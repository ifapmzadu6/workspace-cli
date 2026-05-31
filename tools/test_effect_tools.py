#!/usr/bin/env python3
"""Unit tests for effect measurement helpers."""

from __future__ import annotations

import argparse
import importlib.util
import itertools
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock


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
extract_effect_summary = load_tool("extract_effect_summary")
verify_effect_artifacts = load_tool("verify_effect_artifacts")
prepare_effect_holdouts = load_tool("prepare_effect_holdouts")


class WorkflowConfigurationTests(unittest.TestCase):
    def test_paper_workflow_uses_node24_artifact_upload(self) -> None:
        workflow = (
            TOOLS_DIR.parent / ".github" / "workflows" / "paper-effect.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("actions/upload-artifact@v7", workflow)
        self.assertNotIn("actions/upload-artifact@v5", workflow)
        self.assertNotIn("FORCE_JAVASCRIPT_ACTIONS_TO_NODE24", workflow)


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
    def test_observable_repo_path_preserves_dot_prefixed_paths(self) -> None:
        self.assertEqual(
            measure_effect.observable_repo_path(".github/workflows/release.yml"),
            ".github/workflows/release.yml",
        )
        self.assertEqual(
            measure_effect.observable_repo_path("./.github/workflows/release.yml"),
            ".github/workflows/release.yml",
        )
        self.assertEqual(
            measure_effect.observable_repo_path("./src/main.rs"),
            "src/main.rs",
        )
        self.assertIsNone(measure_effect.observable_repo_path("./../outside.rs"))

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

    def test_loro_weight_selection_prefers_default_within_noise_band(self) -> None:
        summaries = [
            {
                "hybrid_direct_weight": 0.9,
                "train_average_precision_at_5": 0.653,
                "train_ndcg_at_5": 0.788,
            },
            {
                "hybrid_direct_weight": 0.95,
                "train_average_precision_at_5": 0.654,
                "train_ndcg_at_5": 0.790,
            },
        ]

        selected = measure_effect.select_loro_weight_summary(summaries, 5)

        self.assertEqual(selected["hybrid_direct_weight"], 0.9)

    def test_loro_weight_selection_keeps_clear_train_winner(self) -> None:
        summaries = [
            {
                "hybrid_direct_weight": 0.9,
                "train_average_precision_at_5": 0.653,
                "train_ndcg_at_5": 0.788,
            },
            {
                "hybrid_direct_weight": 0.95,
                "train_average_precision_at_5": 0.657,
                "train_ndcg_at_5": 0.790,
            },
        ]

        selected = measure_effect.select_loro_weight_summary(summaries, 5)

        self.assertEqual(selected["hybrid_direct_weight"], 0.95)

    def test_temporal_leakage_audit_requires_index_head_to_match_parent(self) -> None:
        audit = measure_effect.temporal_leakage_audit(
            [
                {
                    "repo": "repo",
                    "heldout_commit": "bbbb",
                    "parent": "aaaa",
                    "seed": "src/a.rs",
                    "index": {
                        "head": "aaaa",
                        "head_matches_parent": True,
                    },
                },
                {
                    "repo": "repo",
                    "heldout_commit": "dddd",
                    "parent": "cccc",
                    "seed": "src/b.rs",
                    "index": {
                        "head": "eeee",
                        "head_matches_parent": False,
                    },
                },
            ]
        )

        self.assertEqual(audit["case_count"], 2)
        self.assertEqual(audit["checked_case_count"], 2)
        self.assertEqual(audit["head_matches_parent_count"], 1)
        self.assertEqual(audit["failure_count"], 1)
        self.assertEqual(audit["failures"][0]["seed"], "src/b.rs")

    def test_repo_holdout_record_includes_origin_remote(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            repo = Path(tmp_dir)
            measure_effect.git(repo, "init", "-q")
            measure_effect.git(
                repo,
                "remote",
                "add",
                "origin",
                "https://example.test/project.git",
            )

            record = measure_effect.repo_holdout_record(repo, "HEAD")

        self.assertEqual(record["remote_url"], "https://example.test/project.git")

    def test_repo_holdout_manifest_preserves_optional_remote_url(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            manifest = Path(tmp_dir) / "holdouts.json"
            manifest.write_text(
                json.dumps(
                    {
                        "repo_holdouts": [
                            {
                                "repo": ".",
                                "ref": "abcdef",
                                "remote_url": "https://example.test/repo.git",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            args = argparse.Namespace(
                repo_holdout_manifest=manifest,
                repo_holdout=[],
                repo_holdout_ref=[],
            )

            measure_effect.apply_repo_holdout_manifest(args, argparse.ArgumentParser())

        self.assertEqual(
            args.repo_holdout_manifest_records[0]["remote_url"],
            "https://example.test/repo.git",
        )

    def test_repo_holdout_manifest_preserves_prepared_source_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            manifest = Path(tmp_dir) / "holdouts.local.json"
            manifest.write_text(
                json.dumps(
                    {
                        "prepared_from": {
                            "manifest": "tools/effect_paper_holdouts.json",
                            "manifest_sha256": "a" * 64,
                        },
                        "repo_holdouts": [
                            {
                                "repo": ".",
                                "ref": "abcdef",
                                "remote_url": "https://example.test/repo.git",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            args = argparse.Namespace(
                repo_holdout_manifest=manifest,
                repo_holdout=[],
                repo_holdout_ref=[],
            )

            measure_effect.apply_repo_holdout_manifest(args, argparse.ArgumentParser())

        self.assertEqual(
            args.repo_holdout_manifest_prepared_from,
            {
                "manifest": "tools/effect_paper_holdouts.json",
                "manifest_sha256": "a" * 64,
            },
        )


class HoldoutPreparationTests(unittest.TestCase):
    def test_safe_repo_dir_name_uses_remote_basename(self) -> None:
        self.assertEqual(
            prepare_effect_holdouts.safe_repo_dir_name(
                "git@github.com:example/workspace-cli.git"
            ),
            "workspace-cli",
        )
        self.assertEqual(
            prepare_effect_holdouts.safe_repo_dir_name("https://example.test/a b.git"),
            "a-b",
        )

    def test_prepare_holdouts_clones_and_writes_local_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            manifest = root / "holdouts.json"
            manifest.write_text(
                json.dumps(
                    {
                        "repo_holdouts": [
                            {
                                "repo": ".",
                                "remote_url": (
                                    "https://example.test/workspace-cli.git"
                                ),
                                "ref": "aaaa",
                            },
                            {
                                "repo": "../other",
                                "url": "git@example.test:org/workspace-cli.git",
                                "ref": "bbbb",
                            },
                        ],
                        "k": 5,
                    }
                ),
                encoding="utf-8",
            )
            calls = []

            def fake_runner(command, **kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(command, 0, stdout="commit\n")

            output_manifest = root / "holdouts.local.json"
            result = prepare_effect_holdouts.prepare_holdouts(
                manifest,
                root / "repos",
                output_manifest,
                runner=fake_runner,
            )

            local_manifest = json.loads(output_manifest.read_text(encoding="utf-8"))
            self.assertEqual(
                local_manifest["prepared_from"],
                {
                    "manifest": str(manifest),
                    "manifest_sha256": prepare_effect_holdouts.file_sha256(
                        manifest
                    ),
                },
            )
            repos = local_manifest["repo_holdouts"]
            self.assertEqual(repos[0]["repo"], str(root / "repos" / "workspace-cli"))
            self.assertEqual(repos[1]["repo"], str(root / "repos" / "workspace-cli-2"))
            self.assertEqual(
                repos[0]["remote_url"],
                "https://example.test/workspace-cli.git",
            )
            self.assertEqual(
                repos[1]["remote_url"],
                "git@example.test:org/workspace-cli.git",
            )
            self.assertEqual(result["local_repos"][1].name, "workspace-cli-2")
            self.assertEqual(
                calls[0],
                [
                    "git",
                    "clone",
                    "--quiet",
                    "https://example.test/workspace-cli.git",
                    str(root / "repos" / "workspace-cli"),
                ],
            )
            self.assertEqual(
                calls[1],
                [
                    "git",
                    "-C",
                    str(root / "repos" / "workspace-cli"),
                    "rev-parse",
                    "--verify",
                    "aaaa^{commit}",
                ],
            )

    def test_prepare_holdouts_fetches_existing_clone_after_origin_check(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            local_repo = root / "repos" / "existing"
            local_repo.mkdir(parents=True)
            manifest = root / "holdouts.json"
            manifest.write_text(
                json.dumps(
                    {
                        "repo_holdouts": [
                            {
                                "repo": "../existing",
                                "remote_url": "https://example.test/existing.git",
                                "ref": "cccc",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            calls = []

            def fake_runner(command, **kwargs):
                calls.append(command)
                if command[-1] == "--is-inside-work-tree":
                    return subprocess.CompletedProcess(command, 0, stdout="true\n")
                if command[-2:] == ["get-url", "origin"]:
                    return subprocess.CompletedProcess(
                        command,
                        0,
                        stdout="https://example.test/existing.git\n",
                    )
                return subprocess.CompletedProcess(command, 0, stdout="commit\n")

            prepare_effect_holdouts.prepare_holdouts(
                manifest,
                root / "repos",
                root / "holdouts.local.json",
                runner=fake_runner,
            )

            self.assertIn(
                [
                    "git",
                    "-C",
                    str(local_repo),
                    "fetch",
                    "--quiet",
                    "--tags",
                    "origin",
                    "+refs/heads/*:refs/remotes/origin/*",
                ],
                calls,
            )

    def test_prepare_holdouts_requires_remote_url(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            manifest = root / "holdouts.json"
            manifest.write_text(
                json.dumps({"repo_holdouts": [{"repo": ".", "ref": "dddd"}]}),
                encoding="utf-8",
            )

            with self.assertRaises(prepare_effect_holdouts.ManifestError):
                prepare_effect_holdouts.prepare_holdouts(
                    manifest,
                    root / "repos",
                    root / "holdouts.local.json",
                    runner=subprocess.run,
                )


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

    def test_content_similarity_ranks_body_overlap_without_seed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            measure_effect.git(root, "init", "-q")
            files = {
                "src/auth.rs": "fn login() { validate_password(); issue_token(); }\n",
                "src/session.rs": "fn validate_password() { issue_session(); }\n",
                "docs/auth.md": "deployment guide\n",
                "README.md": "project overview\n",
            }
            for path, content in files.items():
                measure_effect.write(root / path, content)
            measure_effect.git(root, "add", ".")

            ranked = measure_effect.content_similarity_paths(root, {"src/auth.rs"})

        self.assertNotIn("src/auth.rs", ranked)
        self.assertEqual(ranked[0], "src/session.rs")


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

    def test_metadata_table_includes_holdout_remote_urls(self) -> None:
        table = summarize_effect.render_metadata_table(
            {
                "metadata": {
                    "workspace_bin": "target/debug/workspace",
                    "repo_holdouts": [
                        {
                            "repo": "../example",
                            "ref": "abcdef123456",
                            "remote_url": "https://example.test/repo.git",
                        }
                    ],
                }
            }
        )

        self.assertIn("../example@abcdef1234", table)
        self.assertIn("https://example.test/repo.git", table)

    def test_metadata_table_includes_holdout_source_manifest(self) -> None:
        table = summarize_effect.render_metadata_table(
            {
                "metadata": {
                    "workspace_bin": "target/debug/workspace",
                    "repo_holdout_manifest": "target/effect-repos/holdouts.local.json",
                    "repo_holdout_manifest_sha256": "b" * 64,
                    "repo_holdout_source_manifest": "tools/effect_paper_holdouts.json",
                    "repo_holdout_source_manifest_sha256": "c" * 64,
                }
            }
        )

        self.assertIn("target/effect-repos/holdouts.local.json", table)
        self.assertIn("tools/effect_paper_holdouts.json", table)
        self.assertIn("cccccccccccccccc", table)

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

    def test_residual_gap_cluster_table_groups_by_commit(self) -> None:
        report = {
            "measurements": [
                {
                    "metric": "repo_temporal_holdout_aggregate",
                    "k": 5,
                },
                {
                    "metric": "repo_temporal_holdout",
                    "repo": "/tmp/example",
                    "cases": [
                        {
                            "repo": "/tmp/example",
                            "heldout_commit": "abcdef123456",
                            "seed": "package.json",
                            "expected": ["README.md", ".github/workflows/ci.yml"],
                            "predictable_expected": [".github/workflows/ci.yml"],
                            "methods": {
                                "workspace_related_hybrid": {
                                    "average_precision_at_5": 0.25,
                                    "top": ["README.md", "Cargo.toml"],
                                },
                                "history_oracle_ceiling": {
                                    "average_precision_at_5": 1.0,
                                    "top": [
                                        ".github/workflows/ci.yml",
                                        "README.md",
                                    ],
                                },
                            },
                        },
                        {
                            "repo": "/tmp/example",
                            "heldout_commit": "abcdef123456",
                            "seed": "Cargo.toml",
                            "expected": ["Cargo.lock"],
                            "predictable_expected": [],
                            "methods": {
                                "workspace_related_hybrid": {
                                    "average_precision_at_5": 1.0,
                                    "top": ["Cargo.lock"],
                                },
                                "history_oracle_ceiling": {
                                    "average_precision_at_5": 1.0,
                                    "top": ["Cargo.lock"],
                                },
                            },
                        },
                    ],
                },
            ],
        }

        table = summarize_effect.render_residual_gap_cluster_table(
            report,
            "Holdout",
        )
        self.assertIn("Holdout Residual Gap Clusters @5", table)
        self.assertIn("| example | abcdef1234 | 1 | 1 | 2 | 0.750 |", table)
        self.assertIn(".github/workflows/ci.yml", table)

        predictable_table = summarize_effect.render_residual_gap_cluster_table(
            report,
            "Predictable Holdout",
            expected_key="predictable_expected",
            retarget_metrics=True,
        )
        self.assertIn("Predictable Holdout Residual Gap Clusters @5", predictable_table)
        self.assertIn("| example | abcdef1234 | 1 | 1 | 1 | 1.000 |", predictable_table)

    def test_per_repo_holdout_table_includes_static_baselines(self) -> None:
        def method(ap: float) -> dict:
            return {
                "mean_average_precision_at_5": ap,
                "mean_ndcg_at_5": ap,
            }

        def delta(value: float) -> dict:
            return {"mean_delta_average_precision_at_5": value}

        table = summarize_effect.render_repo_holdout_table(
            {
                "measurements": [
                    {
                        "metric": "repo_temporal_holdout",
                        "repo": "/tmp/example",
                        "k": 5,
                        "case_count": 1,
                        "target_count": 2,
                        "aggregate": {
                            "baseline_path_locality": method(0.1),
                            "baseline_lexical_similarity": method(0.2),
                            "baseline_content_similarity": method(0.3),
                            "baseline_recent_activity": method(0.4),
                            "baseline_global_pagerank": method(0.5),
                            "history_oracle_ceiling": method(1.0),
                            "workspace_related_direct": method(0.6),
                            "workspace_related_pagerank": method(0.7),
                            "workspace_related_hybrid": method(0.8),
                        },
                        "paired_deltas": {
                            (
                                "workspace_related_hybrid_minus_"
                                "baseline_lexical_similarity"
                            ): delta(0.6),
                            (
                                "workspace_related_hybrid_minus_"
                                "baseline_content_similarity"
                            ): delta(0.5),
                        },
                    }
                ]
            },
            "Per-Repo Temporal Holdout",
        )

        self.assertIn("lexical AP", table)
        self.assertIn("content AP", table)
        self.assertIn("hybrid-content delta AP", table)
        self.assertIn("| example | 1 | 2 | 1.000 | 0.100 | 0.200 | 0.300 |", table)

    def test_temporal_leakage_audit_table_reports_matching_heads(self) -> None:
        table = summarize_effect.render_temporal_leakage_audit_table(
            {
                "measurements": [
                    {
                        "metric": "repo_temporal_holdout",
                        "repo": "/tmp/example",
                        "temporal_leakage_audit": {
                            "case_count": 3,
                            "checked_case_count": 3,
                            "head_matches_parent_count": 3,
                            "failure_count": 0,
                            "omitted_failures": 0,
                        },
                    }
                ]
            },
            {
                "repo_count": 1,
                "temporal_leakage_audit": {
                    "case_count": 3,
                    "checked_case_count": 3,
                    "head_matches_parent_count": 3,
                    "failure_count": 0,
                    "omitted_failures": 0,
                },
            },
        )

        self.assertIn("Temporal Holdout Leakage Audit", table)
        self.assertIn("| cross-repo | 1 | 3 | 3 | 3 | 0 | 0 |", table)
        self.assertIn("| example | 1 | 3 | 3 | 3 | 0 | 0 |", table)


class EffectSummaryExtractionTests(unittest.TestCase):
    def test_extract_summary_includes_headline_holdout_metrics(self) -> None:
        report = {
            "metadata": {"workspace_commit": "abcdef"},
            "measurements": [
                {"metric": "map_fact_recall", "recall": 1.0},
                {"metric": "transaction_audit_signal_recall", "recall": 1.0},
                {
                    "metric": "retrieval_suite",
                    "k": 5,
                    "scenario_count": 4,
                    "aggregate": {
                        "workspace_related_hybrid": {
                            "mean_recall_at_5": 1.0,
                            "mean_average_precision_at_5": 0.9,
                            "mean_ndcg_at_5": 0.95,
                        },
                        "workspace_impact_hybrid": {
                            "mean_recall_at_5": 1.0,
                            "mean_average_precision_at_5": 1.0,
                            "mean_ndcg_at_5": 1.0,
                        },
                    },
                    "paired_deltas": {},
                },
                {
                    "metric": "repo_temporal_holdout_aggregate",
                    "k": 5,
                    "repo_count": 3,
                    "case_count": 50,
                    "target_count": 207,
                    "heldout_commit_count": 15,
                    "temporal_leakage_audit": {
                        "case_count": 50,
                        "checked_case_count": 50,
                        "head_matches_parent_count": 50,
                        "failure_count": 0,
                    },
                    "predictable_only": {
                        "k": 5,
                        "case_count": 3,
                        "target_count": 3,
                        "aggregate": {
                            "workspace_related_hybrid": {
                                "mean_average_precision_at_5": 0.7,
                            },
                            "history_oracle_ceiling": {
                                "mean_average_precision_at_5": 0.9,
                            },
                        },
                        "paired_deltas": {},
                        "hybrid_weight_sweep": [],
                    },
                    "aggregate": {
                        "workspace_related_hybrid": {
                            "mean_average_precision_at_5": 0.651,
                            "ci95_low_average_precision_at_5": 0.555,
                            "ci95_high_average_precision_at_5": 0.741,
                        },
                        "workspace_related_direct": {
                            "mean_average_precision_at_5": 0.564,
                        },
                        "baseline_path_locality": {
                            "mean_average_precision_at_5": 0.1,
                        },
                        "history_oracle_ceiling": {
                            "mean_average_precision_at_5": 0.7,
                        },
                    },
                    "paired_deltas": {
                        "workspace_related_hybrid_minus_workspace_related_direct": {
                            "mean_delta_average_precision_at_5": 0.087,
                            "p_greater_holm_delta_average_precision_at_5": 0.00003,
                            "win_count_delta_average_precision_at_5": 21,
                            "tie_count_delta_average_precision_at_5": 24,
                            "loss_count_delta_average_precision_at_5": 5,
                        },
                    },
                    "hybrid_weight_sweep": [
                        {
                            "hybrid_direct_weight": 0.5,
                            "related": {
                                "method": "workspace_related_hybrid_w_0_5",
                                "aggregate": {
                                    "workspace_related_hybrid_w_0_5": {
                                        "mean_average_precision_at_5": 0.64,
                                    }
                                },
                            },
                        },
                        {
                            "hybrid_direct_weight": 0.9,
                            "related": {
                                "method": "workspace_related_hybrid_w_0_9",
                                "aggregate": {
                                    "workspace_related_hybrid_w_0_9": {
                                        "mean_average_precision_at_5": 0.651,
                                    }
                                },
                                "paired_deltas": {
                                    "workspace_related_hybrid_w_0_9_minus_workspace_related_direct": {
                                        "mean_delta_average_precision_at_5": 0.087,
                                        "p_greater_holm_delta_average_precision_at_5": 0.00003,
                                    },
                                },
                            },
                        },
                    ],
                },
                {
                    "metric": "repo_temporal_holdout",
                    "repo": "/tmp/workspace-cli",
                    "k": 5,
                    "case_count": 6,
                    "target_count": 8,
                    "heldout_commit_count": 2,
                    "aggregate": {
                        "workspace_related_hybrid": {
                            "mean_average_precision_at_5": 0.75,
                        },
                        "workspace_related_direct": {
                            "mean_average_precision_at_5": 0.5,
                        },
                        "baseline_path_locality": {
                            "mean_average_precision_at_5": 0.25,
                        },
                        "history_oracle_ceiling": {
                            "mean_average_precision_at_5": 1.0,
                        },
                    },
                    "paired_deltas": {},
                    "hybrid_weight_sweep": [],
                    "cases": [
                        {
                            "repo": "/tmp/workspace-cli",
                            "heldout_commit": "abc123",
                            "seed": "README.md",
                            "expected": [
                                "Cargo.toml",
                                "src/main.rs",
                            ],
                            "predictable_expected": [
                                "Cargo.toml",
                                "src/main.rs",
                            ],
                            "unpredictable_expected": [],
                            "methods": {
                                "workspace_related_hybrid": {
                                    "average_precision_at_5": 0.25,
                                    "hits": ["Cargo.toml"],
                                    "top": [
                                        "README.md",
                                        "Cargo.toml",
                                        "Cargo.lock",
                                        "tests/cli.rs",
                                        ".gitignore",
                                    ],
                                },
                                "history_oracle_ceiling": {
                                    "average_precision_at_5": 1.0,
                                    "top": ["Cargo.toml", "src/main.rs"],
                                },
                            },
                        },
                        {
                            "repo": "/tmp/workspace-cli",
                            "heldout_commit": "abc123",
                            "seed": "Cargo.toml",
                            "expected": ["Cargo.lock"],
                            "predictable_expected": ["Cargo.lock"],
                            "unpredictable_expected": [],
                            "methods": {
                                "workspace_related_hybrid": {
                                    "average_precision_at_5": 1.0,
                                    "hits": ["Cargo.lock"],
                                    "top": ["Cargo.lock"],
                                },
                                "history_oracle_ceiling": {
                                    "average_precision_at_5": 1.0,
                                    "top": ["Cargo.lock"],
                                },
                            },
                        },
                        {
                            "repo": "/tmp/workspace-cli",
                            "heldout_commit": "def456",
                            "seed": "src/main.rs",
                            "expected": ["tests/cli.rs"],
                            "predictable_expected": [],
                            "unpredictable_expected": ["tests/cli.rs"],
                            "methods": {
                                "workspace_related_hybrid": {
                                    "average_precision_at_5": 0.5,
                                    "hits": [],
                                    "top": ["README.md", "Cargo.toml"],
                                },
                                "history_oracle_ceiling": {
                                    "average_precision_at_5": 0.75,
                                    "top": ["tests/cli.rs"],
                                },
                            },
                        },
                    ],
                },
            ],
        }

        summary = extract_effect_summary.extract_summary(report)

        self.assertEqual(summary["schema_version"], 1)
        self.assertEqual(summary["observation_recall"]["map_fact_recall"], 1.0)
        holdout = summary["repo_temporal_holdout"]
        self.assertEqual(holdout["temporal_leakage_audit"]["failure_count"], 0)
        self.assertEqual(holdout["best_weight_sweep"]["direct_weight"], 0.9)
        self.assertEqual(
            [entry["direct_weight"] for entry in holdout["weight_sweep"]],
            [0.5, 0.9],
        )
        self.assertEqual(
            holdout["weight_sweep"][1]["average_precision_at_5"],
            0.651,
        )
        self.assertEqual(
            holdout["weight_sweep"][1]["delta_vs_direct"]["mean_delta"],
            0.087,
        )
        self.assertEqual(holdout["per_repo"][0]["repo_name"], "workspace-cli")
        self.assertEqual(
            holdout["per_repo"][0]["methods"]["workspace_related_hybrid"][
                "average_precision_at_5"
            ]["mean"],
            0.75,
        )
        self.assertEqual(
            holdout["methods"]["workspace_related_hybrid"][
                "average_precision_at_5"
            ]["mean"],
            0.651,
        )
        self.assertEqual(
            holdout["methods"]["baseline_path_locality"][
                "average_precision_at_5"
            ]["mean"],
            0.1,
        )
        self.assertEqual(
            holdout["oracle_normalized"]["workspace_related_hybrid"][
                "oracle_normalized_average_precision_at_5"
            ],
            0.93,
        )
        self.assertEqual(
            holdout["oracle_normalized"]["workspace_related_hybrid"][
                "oracle_gap_average_precision_at_5"
            ],
            0.049,
        )
        self.assertEqual(
            holdout["per_repo"][0]["oracle_normalized"]["workspace_related_hybrid"][
                "oracle_normalized_average_precision_at_5"
            ],
            0.75,
        )
        residual_clusters = holdout["residual_gap_clusters"]
        self.assertEqual(
            [cluster["heldout_commit"] for cluster in residual_clusters],
            ["abc123", "def456"],
        )
        self.assertEqual(residual_clusters[0]["case_count"], 1)
        self.assertEqual(residual_clusters[0]["target_count"], 2)
        self.assertEqual(
            residual_clusters[0]["oracle_gap_average_precision_at_5"],
            0.75,
        )
        self.assertEqual(
            residual_clusters[0]["top_residual_cases"][0]["missing_expected"],
            ["src/main.rs"],
        )
        predictable_clusters = holdout["predictable_only"]["residual_gap_clusters"]
        self.assertEqual(len(predictable_clusters), 1)
        self.assertEqual(predictable_clusters[0]["expected_key"], "predictable_expected")
        self.assertTrue(predictable_clusters[0]["retarget_metrics"])
        self.assertEqual(
            predictable_clusters[0]["oracle_gap_average_precision_at_5"],
            0.75,
        )
        self.assertEqual(
            holdout["per_repo"][0]["residual_gap_clusters"][0][
                "heldout_commit"
            ],
            "abc123",
        )
        self.assertEqual(
            holdout["key_deltas"][
                "workspace_related_hybrid_minus_workspace_related_direct"
            ]["wins"],
            21,
        )


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

    def paired_deltas(self, left: str) -> dict:
        return {
            f"{left}_minus_workspace_related_direct": self.passing_delta(),
            f"{left}_minus_workspace_related_pagerank": self.passing_delta(),
            f"{left}_minus_baseline_lexical_similarity": self.passing_delta(),
            f"{left}_minus_baseline_content_similarity": self.passing_delta(),
            f"{left}_minus_baseline_recent_activity": self.passing_delta(),
            f"{left}_minus_baseline_global_pagerank": self.passing_delta(),
        }

    def passing_delta(self) -> dict:
        return {"p_greater_holm_delta_average_precision_at_5": 0.001}

    def loro_selection(
        self,
        *,
        ap: float,
        direct_ap: float,
        pagerank_ap: float,
        lexical_ap: float,
        content_ap: float,
        recent_ap: float,
        global_ap: float,
    ) -> dict:
        return {
            "candidate_weights": check_effect_thresholds.EXPECTED_HYBRID_WEIGHT_SWEEP,
            "selections": [{}, {}, {}],
            "aggregate": {
                "workspace_related_direct": {
                    "mean_average_precision_at_5": direct_ap,
                },
                "workspace_related_pagerank": {
                    "mean_average_precision_at_5": pagerank_ap,
                },
                "baseline_lexical_similarity": {
                    "mean_average_precision_at_5": lexical_ap,
                },
                "baseline_content_similarity": {
                    "mean_average_precision_at_5": content_ap,
                },
                "baseline_recent_activity": {
                    "mean_average_precision_at_5": recent_ap,
                },
                "baseline_global_pagerank": {
                    "mean_average_precision_at_5": global_ap,
                },
                "workspace_related_hybrid_loro": {
                    "mean_average_precision_at_5": ap,
                },
            },
            "paired_deltas": self.paired_deltas("workspace_related_hybrid_loro"),
        }

    def repo_macro_average(self, *, predictable: bool) -> dict:
        if predictable:
            hybrid_ap = 0.80
            direct_ap = 0.67
            pagerank_ap = 0.62
            content_ap = 0.42
            recent_ap = 0.52
            global_ap = 0.53
        else:
            hybrid_ap = 0.768
            direct_ap = 0.62
            pagerank_ap = 0.56
            content_ap = 0.38
            recent_ap = 0.50
            global_ap = 0.51
        return {
            "repo_count": 3,
            "aggregate": {
                "workspace_related_direct": {
                    "mean_average_precision_at_5": direct_ap,
                },
                "workspace_related_pagerank": {
                    "mean_average_precision_at_5": pagerank_ap,
                },
                "baseline_lexical_similarity": {
                    "mean_average_precision_at_5": 0.25,
                },
                "baseline_content_similarity": {
                    "mean_average_precision_at_5": content_ap,
                },
                "baseline_recent_activity": {
                    "mean_average_precision_at_5": recent_ap,
                },
                "baseline_global_pagerank": {
                    "mean_average_precision_at_5": global_ap,
                },
                "workspace_related_hybrid": {
                    "mean_average_precision_at_5": hybrid_ap,
                },
            },
        }

    def repo_holdout(self, *, predictable: bool) -> dict:
        if predictable:
            return {
                "case_count": 52,
                "target_count": 204,
                "aggregate": {
                    "workspace_related_direct": {
                        "mean_average_precision_at_5": 0.64,
                    },
                    "baseline_lexical_similarity": {
                        "mean_average_precision_at_5": 0.20,
                    },
                    "baseline_content_similarity": {
                        "mean_average_precision_at_5": 0.30,
                    },
                    "baseline_recent_activity": {
                        "mean_average_precision_at_5": 0.45,
                    },
                    "baseline_global_pagerank": {
                        "mean_average_precision_at_5": 0.50,
                    },
                    "workspace_related_pagerank": {
                        "mean_average_precision_at_5": 0.61,
                    },
                    "workspace_related_hybrid": {
                        "mean_average_precision_at_5": 0.770,
                    },
                    "history_oracle_ceiling": {
                        "mean_average_precision_at_5": 0.90,
                    },
                },
                "hybrid_weight_sweep": self.weight_sweep(0.770),
                "leave_one_repo_out_weight_selection": self.loro_selection(
                    ap=0.770,
                    direct_ap=0.64,
                    pagerank_ap=0.61,
                    lexical_ap=0.20,
                    content_ap=0.30,
                    recent_ap=0.45,
                    global_ap=0.50,
                ),
                "paired_deltas": self.paired_deltas("workspace_related_hybrid"),
                "repo_macro_average": self.repo_macro_average(predictable=True),
            }
        return {
            "metric": "repo_temporal_holdout_aggregate",
            "repo_count": 3,
            "case_count": 53,
            "target_count": 216,
            "aggregate": {
                "workspace_related_direct": {
                    "mean_average_precision_at_5": 0.626,
                },
                "baseline_lexical_similarity": {
                    "mean_average_precision_at_5": 0.20,
                },
                "baseline_content_similarity": {
                    "mean_average_precision_at_5": 0.30,
                },
                "baseline_recent_activity": {
                    "mean_average_precision_at_5": 0.403,
                },
                "baseline_global_pagerank": {
                    "mean_average_precision_at_5": 0.498,
                },
                "workspace_related_pagerank": {
                    "mean_average_precision_at_5": 0.577,
                },
                "workspace_related_hybrid": {
                    "mean_average_precision_at_5": 0.730,
                },
                "history_oracle_ceiling": {
                    "mean_average_precision_at_5": 0.853,
                },
            },
            "hybrid_weight_sweep": self.weight_sweep(0.730),
            "leave_one_repo_out_weight_selection": self.loro_selection(
                ap=0.730,
                direct_ap=0.626,
                pagerank_ap=0.577,
                lexical_ap=0.20,
                content_ap=0.30,
                recent_ap=0.40,
                global_ap=0.42,
            ),
            "paired_deltas": self.paired_deltas("workspace_related_hybrid"),
            "repo_macro_average": self.repo_macro_average(predictable=False),
            "temporal_leakage_audit": {
                "case_count": 53,
                "checked_case_count": 53,
                "head_matches_parent_count": 53,
                "failure_count": 0,
            },
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
                        "baseline_content_similarity": {
                            "mean_average_precision_at_5": 0.45,
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

    def test_effect_thresholds_fail_when_content_baseline_catches_hybrid(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["aggregate"]["baseline_content_similarity"][
            "mean_average_precision_at_5"
        ] = 0.56
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("baseline_content_similarity" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_for_degraded_oracle_normalized_ap(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["aggregate"]["history_oracle_ceiling"][
            "mean_average_precision_at_5"
        ] = 0.95
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any(
                "workspace_related_hybrid.mean_average_precision_at_5 / "
                "history_oracle_ceiling.mean_average_precision_at_5" in item
                for item in failures
            ),
            failures,
        )

    def test_effect_thresholds_fail_when_recent_baseline_catches_hybrid(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["aggregate"]["baseline_recent_activity"][
            "mean_average_precision_at_5"
        ] = 0.60
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("baseline_recent_activity" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_when_holdout_holm_p_is_too_large(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["paired_deltas"][
            "workspace_related_hybrid_minus_workspace_related_direct"
        ]["p_greater_holm_delta_average_precision_at_5"] = 0.02
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any(
                "p_greater_holm_delta_average_precision_at_5" in item
                for item in failures
            ),
            failures,
        )

    def test_effect_thresholds_fail_when_repo_macro_average_degrades(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["repo_macro_average"]["aggregate"][
            "workspace_related_hybrid"
        ]["mean_average_precision_at_5"] = 0.50
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("repo_macro_average" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_when_temporal_leakage_audit_fails(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["temporal_leakage_audit"][
            "head_matches_parent_count"
        ] = 49
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("temporal_leakage_audit" in item for item in failures),
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

    def test_effect_thresholds_fail_when_default_weight_does_not_match_sweep(self) -> None:
        report = self.passing_report()
        report["measurements"][-1]["aggregate"]["workspace_related_hybrid"][
            "mean_average_precision_at_5"
        ] = 0.66
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("must match hybrid_weight_sweep[0.9]" in item for item in failures),
            failures,
        )

    def test_effect_thresholds_fail_when_default_weight_is_not_best_sweep(self) -> None:
        report = self.passing_report()
        holdout = report["measurements"][-1]
        for entry in holdout["hybrid_weight_sweep"]:
            if entry["hybrid_direct_weight"] == 0.8:
                method = entry["related"]["method"]
                entry["related"]["aggregate"][method][
                    "mean_average_precision_at_5"
                ] = 0.74
        failures = check_effect_thresholds.check_report(report)
        self.assertTrue(
            any("is below weight 0.8" in item for item in failures),
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
        self.assertEqual(plan["result_summary_path"].name, "result_summary.json")
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
                plan["result_summary_command"],
            ])
            self.assertTrue(plan["json_path"].exists())
            self.assertTrue(plan["markdown_path"].exists())
            self.assertTrue(plan["result_summary_path"].exists())
            self.assertTrue(plan["threshold_path"].exists())
            run_manifest = json.loads(plan["run_manifest_path"].read_text())
            self.assertEqual(
                run_manifest["commands"]["measure"],
                plan["measurement_command"],
            )
            self.assertEqual(
                run_manifest["commands"]["extract_result_summary"],
                plan["result_summary_command"],
            )
            self.assertEqual(
                run_manifest["commands"]["verify_artifacts"],
                plan["verify_command"],
            )
            self.assertEqual(
                run_manifest["result_summary"],
                str(plan["result_summary_path"]),
            )
            self.assertEqual(
                run_manifest["sha256"]["json"],
                run_effect_artifacts.file_sha256(plan["json_path"]),
            )
            self.assertEqual(
                run_manifest["sha256"]["markdown"],
                run_effect_artifacts.file_sha256(plan["markdown_path"]),
            )
            self.assertEqual(
                run_manifest["sha256"]["result_summary"],
                run_effect_artifacts.file_sha256(plan["result_summary_path"]),
            )
            self.assertEqual(
                run_manifest["sha256"]["thresholds"],
                run_effect_artifacts.file_sha256(plan["threshold_path"]),
            )
            self.assertFalse(run_manifest["require_holdout_thresholds"])

    def test_artifact_runner_copies_holdout_manifests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            root = Path(tmp_dir)
            output_dir = root / "artifacts"
            source_manifest = root / "source_holdouts.json"
            source_manifest.write_text(
                json.dumps({"repo_holdouts": [{"repo": ".", "ref": "abcdef"}]}),
                encoding="utf-8",
            )
            local_manifest = root / "holdouts.local.json"
            local_manifest.write_text(
                json.dumps(
                    {
                        "prepared_from": {
                            "manifest": str(source_manifest),
                            "manifest_sha256": run_effect_artifacts.file_sha256(
                                source_manifest
                            ),
                        },
                        "repo_holdouts": [
                            {
                                "repo": str(root / "repo"),
                                "ref": "abcdef",
                                "remote_url": "https://example.test/repo.git",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            plan = run_effect_artifacts.build_plan(output_dir, local_manifest)

            def fake_runner(command, **kwargs):
                stdout = kwargs.get("stdout")
                if stdout is not None:
                    stdout.write("{}\n")
                return subprocess.CompletedProcess(command, 0)

            run_effect_artifacts.run_plan(plan, runner=fake_runner)

            self.assertEqual(
                json.loads(plan["holdout_manifest_path"].read_text()),
                json.loads(local_manifest.read_text()),
            )
            self.assertEqual(
                json.loads(plan["holdout_source_manifest_path"].read_text()),
                json.loads(source_manifest.read_text()),
            )
            run_manifest = json.loads(plan["run_manifest_path"].read_text())
            self.assertEqual(
                run_manifest["sha256"]["holdout_manifest"],
                run_effect_artifacts.file_sha256(plan["holdout_manifest_path"]),
            )
            self.assertEqual(
                run_manifest["sha256"]["holdout_source_manifest"],
                run_effect_artifacts.file_sha256(
                    plan["holdout_source_manifest_path"]
                ),
            )


class EffectArtifactVerifierTests(unittest.TestCase):
    SUMMARY_FIXTURE = {"schema_version": 1, "repo_temporal_holdout": {}}
    MARKDOWN_FIXTURE = "# Effect Report\n"

    def write_artifact_set(self, output_dir: Path) -> None:
        output_dir.mkdir()
        artifacts = {
            "effect.json": json.dumps({"measurements": []}) + "\n",
            "effect.md": self.MARKDOWN_FIXTURE,
            "result_summary.json": json.dumps(self.SUMMARY_FIXTURE) + "\n",
            "thresholds.txt": "effect threshold check passed\n",
        }
        for filename, content in artifacts.items():
            (output_dir / filename).write_text(content, encoding="utf-8")

        run_manifest = {
            "commands": {
                "measure": ["python3", "tools/measure_effect.py"],
                "check_thresholds": ["python3", "tools/check_effect_thresholds.py"],
                "summarize": ["python3", "tools/summarize_effect.py"],
                "extract_result_summary": [
                    "python3",
                    "tools/extract_effect_summary.py",
                ],
            },
            "require_holdout_thresholds": True,
            "sha256": {
                key: verify_effect_artifacts.file_sha256(output_dir / filename)
                for key, filename in verify_effect_artifacts.ARTIFACT_FILES.items()
            },
        }
        (output_dir / "run_manifest.json").write_text(
            json.dumps(run_manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    def verify_with_patched_semantics(
        self,
        output_dir: Path,
        *,
        threshold_failures: list[str] | None = None,
        extracted_summary: dict | None = None,
        rendered_markdown: str | None = None,
    ) -> list[str]:
        threshold_failures = threshold_failures or []
        extracted_summary = extracted_summary or self.SUMMARY_FIXTURE
        rendered_markdown = rendered_markdown or self.MARKDOWN_FIXTURE
        with mock.patch.object(
            verify_effect_artifacts.check_effect_thresholds,
            "check_report",
            return_value=threshold_failures,
        ), mock.patch.object(
            verify_effect_artifacts.extract_effect_summary,
            "extract_summary",
            return_value=extracted_summary,
        ), mock.patch.object(
            verify_effect_artifacts.summarize_effect,
            "render_report",
            return_value=rendered_markdown,
        ):
            return verify_effect_artifacts.verify_artifact_directory(output_dir)

    def test_artifact_verifier_accepts_complete_artifact_directory(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)

            self.assertEqual(
                self.verify_with_patched_semantics(output_dir),
                [],
            )

    def test_artifact_verifier_rejects_checksum_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)
            (output_dir / "effect.md").write_text(
                "# Modified Report\n",
                encoding="utf-8",
            )

            failures = self.verify_with_patched_semantics(output_dir)

        self.assertTrue(
            any("effect.md sha256 mismatch" in failure for failure in failures),
            failures,
        )

    def test_artifact_verifier_rejects_missing_threshold_pass_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)
            (output_dir / "thresholds.txt").write_text(
                "effect threshold check failed\n",
                encoding="utf-8",
            )

            failures = self.verify_with_patched_semantics(output_dir)

        self.assertTrue(
            any("thresholds.txt does not contain" in failure for failure in failures),
            failures,
        )

    def test_artifact_verifier_rejects_result_summary_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)

            failures = self.verify_with_patched_semantics(
                output_dir,
                extracted_summary={"schema_version": 1, "changed": True},
            )

        self.assertTrue(
            any("result_summary.json does not match" in failure for failure in failures),
            failures,
        )

    def test_artifact_verifier_rejects_markdown_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)
            (output_dir / "effect.md").write_text(
                "# Stale Report\n",
                encoding="utf-8",
            )
            run_manifest = json.loads((output_dir / "run_manifest.json").read_text())
            run_manifest["sha256"]["markdown"] = verify_effect_artifacts.file_sha256(
                output_dir / "effect.md"
            )
            (output_dir / "run_manifest.json").write_text(
                json.dumps(run_manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

            failures = self.verify_with_patched_semantics(output_dir)

        self.assertTrue(
            any("effect.md does not match" in failure for failure in failures),
            failures,
        )

    def test_artifact_verifier_rejects_missing_residual_gap_clusters(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)
            summary = {
                "schema_version": 1,
                "repo_temporal_holdout": {
                    "k": 5,
                    "oracle_normalized": {
                        "workspace_related_hybrid": {
                            "oracle_gap_average_precision_at_5": 0.123,
                        },
                    },
                    "predictable_only": {
                        "k": 5,
                        "oracle_normalized": {
                            "workspace_related_hybrid": {
                                "oracle_gap_average_precision_at_5": 0.456,
                            },
                        },
                    },
                },
            }
            (output_dir / "result_summary.json").write_text(
                json.dumps(summary) + "\n",
                encoding="utf-8",
            )
            run_manifest = json.loads((output_dir / "run_manifest.json").read_text())
            run_manifest["sha256"]["result_summary"] = (
                verify_effect_artifacts.file_sha256(
                    output_dir / "result_summary.json"
                )
            )
            (output_dir / "run_manifest.json").write_text(
                json.dumps(run_manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

            failures = self.verify_with_patched_semantics(
                output_dir,
                extracted_summary=summary,
            )

        self.assertTrue(
            any(
                "repo_temporal_holdout missing residual_gap_clusters" in failure
                for failure in failures
            ),
            failures,
        )
        self.assertTrue(
            any(
                "repo_temporal_holdout.predictable_only missing residual_gap_clusters"
                in failure
                for failure in failures
            ),
            failures,
        )

    def test_artifact_verifier_rejects_recomputed_threshold_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)

            failures = self.verify_with_patched_semantics(
                output_dir,
                threshold_failures=["hybrid AP degraded"],
            )

        self.assertTrue(
            any(
                "threshold recheck failed: hybrid AP degraded" in failure
                for failure in failures
            ),
            failures,
        )

    def test_artifact_verifier_rejects_holdout_manifest_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp_dir:
            output_dir = Path(tmp_dir) / "artifacts"
            self.write_artifact_set(output_dir)
            holdout_manifest = output_dir / "holdout_manifest.json"
            holdout_manifest.write_text(
                json.dumps({"repo_holdouts": []}) + "\n",
                encoding="utf-8",
            )
            effect_json = output_dir / "effect.json"
            effect_json.write_text(
                json.dumps(
                    {
                        "metadata": {
                            "repo_holdout_manifest_sha256": "0" * 64,
                        },
                        "measurements": [],
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            run_manifest = json.loads((output_dir / "run_manifest.json").read_text())
            run_manifest["holdout_manifest"] = "holdout_manifest.json"
            run_manifest["sha256"]["json"] = verify_effect_artifacts.file_sha256(
                effect_json
            )
            run_manifest["sha256"]["holdout_manifest"] = (
                verify_effect_artifacts.file_sha256(holdout_manifest)
            )
            (output_dir / "run_manifest.json").write_text(
                json.dumps(run_manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )

            failures = self.verify_with_patched_semantics(output_dir)

        self.assertTrue(
            any(
                "holdout_manifest.json sha256 does not match" in failure
                for failure in failures
            ),
            failures,
        )


if __name__ == "__main__":
    unittest.main()
