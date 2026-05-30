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
import subprocess
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
BOOTSTRAP_SAMPLES = 1000
SIGN_FLIP_SAMPLES = 10000
DEFAULT_CUTOFF_SWEEP = [1, 3, 5]
RELATED_COMPARISON_PAIRS = [
    ("workspace_related_hybrid", "workspace_related_direct"),
    ("workspace_related_hybrid", "workspace_related_pagerank"),
    ("workspace_related_pagerank", "workspace_related_direct"),
]
IMPACT_COMPARISON_PAIRS = [
    ("workspace_impact_hybrid", "workspace_impact_direct"),
    ("workspace_impact_hybrid", "workspace_impact_pagerank"),
    ("workspace_impact_pagerank", "workspace_impact_direct"),
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
            comparison[f"p_greater_delta_{metric_name}"] = round(p_greater, 4)
            comparison[f"p_two_sided_delta_{metric_name}"] = round(p_two_sided, 4)
        deltas[comparison_name] = comparison
    return deltas


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
    normalized = path.strip().lstrip("./").replace("\\", "/").rstrip("/")
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
) -> dict[str, Any]:
    index = workspace_json(
        bin_path,
        root,
        "index",
        "cochange",
        "--max-files-per-commit",
        str(max_files_per_commit),
        "--json",
    )
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
    git_diff = run(["git", "diff", "--name-only"], root).stdout.splitlines()

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
        "methods": {
            "baseline_git_diff_only": ranking_metrics(git_diff, expected, k),
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
        },
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
) -> dict[str, Any]:
    index = workspace_json(
        bin_path,
        root,
        "index",
        "cochange",
        "--max-files-per-commit",
        str(max_files_per_commit),
        "--json",
    )
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
    git_diff = run(["git", "diff", "--name-only"], root).stdout.splitlines()

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
        "methods": {
            "baseline_git_diff_only": ranking_metrics(git_diff, expected, k),
            "workspace_impact_direct": ranking_metrics(
                paths(direct, "data", "impacted"), expected, k
            ),
            "workspace_impact_pagerank": ranking_metrics(
                paths(pagerank, "data", "impacted"), expected, k
            ),
            "workspace_impact_hybrid": ranking_metrics(
                paths(hybrid, "data", "impacted"), expected, k
            ),
        },
    }


def measure_retrieval_suite(bin_path: Path, k: int) -> dict[str, Any]:
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
            )
        )

    return {
        "metric": "retrieval_suite",
        "k": k,
        "scenario_count": len(scenarios),
        "scenarios": scenarios,
        "aggregate": aggregate_metric_sets(scenarios, k),
        "cutoff_sweep": cutoff_sweep_metric_sets(
            scenarios,
            default_cutoffs(k),
            pairs,
        ),
        "paired_deltas": paired_delta_metric_sets(
            scenarios,
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

            heldout_commits += 1
            for seed in files:
                exists = run(
                    ["git", "cat-file", "-e", f"{parent}:{seed}"],
                    repo,
                    check=False,
                ).returncode == 0
                if not exists:
                    skipped["new_seed_file"] += 1
                    continue

                expected = set(files) - {seed}
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
                cases.append(
                    {
                        "repo": str(repo),
                        "heldout_commit": commit["hash"][:12],
                        "parent": parent[:12],
                        "seed": seed,
                        "expected": sorted(expected),
                        "index": {
                            "commits_indexed": index["data"]["commits_indexed"],
                            "ignored_large_commits": index["data"][
                                "ignored_large_commits"
                            ],
                            "edge_count": index["data"]["edge_count"],
                        },
                        "methods": {
                            "workspace_related_direct": ranking_metrics(
                                paths(direct, "data", "related"), expected, k
                            ),
                            "workspace_related_pagerank": ranking_metrics(
                                paths(pagerank, "data", "related"), expected, k
                            ),
                            "workspace_related_hybrid": ranking_metrics(
                                paths(hybrid, "data", "related"), expected, k
                            ),
                        },
                    }
                )

            if heldout_commits >= max_heldout_commits:
                break

    return {
        "metric": "repo_temporal_holdout",
        "repo": str(repo),
        "end_ref": end_ref,
        "end_commit": end_commit[:12],
        "k": k,
        "candidate_commit_count": len(commits),
        "heldout_commit_count": heldout_commits,
        "case_count": len(cases),
        "skipped": skipped,
        "cases": cases,
        "aggregate": aggregate_metric_sets(cases, k) if cases else {},
        "cutoff_sweep": cutoff_sweep_metric_sets(
            cases,
            default_cutoffs(k),
            RELATED_COMPARISON_PAIRS,
        )
        if cases
        else [],
        "paired_deltas": paired_delta_metric_sets(
            cases,
            k,
            RELATED_COMPARISON_PAIRS,
        )
        if cases
        else {},
    }


def aggregate_repo_holdouts(holdouts: list[dict[str, Any]], k: int) -> dict[str, Any]:
    cases = [
        case
        for holdout in holdouts
        for case in holdout["cases"]
    ]
    skipped: dict[str, int] = {}
    for holdout in holdouts:
        for key, value in holdout["skipped"].items():
            skipped[key] = skipped.get(key, 0) + value

    return {
        "metric": "repo_temporal_holdout_aggregate",
        "repo_count": len(holdouts),
        "repos": [holdout["repo"] for holdout in holdouts],
        "end_refs": [holdout["end_ref"] for holdout in holdouts],
        "end_commits": [holdout["end_commit"] for holdout in holdouts],
        "k": k,
        "candidate_commit_count": sum(
            holdout["candidate_commit_count"] for holdout in holdouts
        ),
        "heldout_commit_count": sum(
            holdout["heldout_commit_count"] for holdout in holdouts
        ),
        "case_count": len(cases),
        "skipped": skipped,
        "aggregate": aggregate_metric_sets(cases, k) if cases else {},
        "cutoff_sweep": cutoff_sweep_metric_sets(
            cases,
            default_cutoffs(k),
            RELATED_COMPARISON_PAIRS,
        )
        if cases
        else [],
        "paired_deltas": paired_delta_metric_sets(
            cases,
            k,
            RELATED_COMPARISON_PAIRS,
        )
        if cases
        else {},
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
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
    parser.add_argument("--max-heldout-commits", type=int, default=5)
    parser.add_argument("--max-candidate-commits", type=int, default=40)
    parser.add_argument("--max-files-per-commit", type=int, default=40)
    parser.add_argument("--k", type=int, default=5)
    args = parser.parse_args()
    if args.k < 1:
        parser.error("--k must be at least 1")
    if args.repo_holdout_ref and len(args.repo_holdout_ref) != len(args.repo_holdout):
        parser.error("--repo-holdout-ref must be repeated once per --repo-holdout")
    return args


def main() -> None:
    args = parse_args()
    bin_path = workspace_bin()
    measurements = [
        measure_observation(bin_path),
        measure_related_and_impact(bin_path),
        measure_retrieval_suite(bin_path, args.k),
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
            )
            for repo, end_ref in zip(args.repo_holdout, repo_holdout_refs)
        ]
        measurements.extend(repo_holdouts)
        if len(repo_holdouts) > 1:
            measurements.append(aggregate_repo_holdouts(repo_holdouts, args.k))

    report = {
        "workspace_bin": str(bin_path),
        "measurements": measurements,
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
