#!/usr/bin/env python3
"""Measure workspace-cli effects against small reproducible fixtures.

This is not a correctness smoke test. It measures whether the CLI improves
workspace observability compared with narrow baseline signals such as the
current git diff or direct co-change only.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


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


def paths(value: dict[str, Any], *segments: str) -> list[str]:
    cursor: Any = value
    for segment in segments:
        cursor = cursor[segment]
    return [item["path"] for item in cursor]


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

        append(root / "src/auth.rs", "local auth change\n")
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
            "workspace_impact_pagerank": precision_recall(
                paths(impact, "data", "impacted"), expected, 3
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


def main() -> None:
    bin_path = workspace_bin()
    report = {
        "workspace_bin": str(bin_path),
        "measurements": [
            measure_observation(bin_path),
            measure_related_and_impact(bin_path),
            measure_transaction(bin_path),
        ],
    }
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
