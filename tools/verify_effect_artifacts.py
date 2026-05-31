#!/usr/bin/env python3
"""Verify a generated effect artifact directory."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any


PASS_MARKER = "effect threshold check passed"
EXPECTED_RESULT_SUMMARY_SCHEMA_VERSION = 1

ARTIFACT_FILES = {
    "json": "effect.json",
    "markdown": "effect.md",
    "result_summary": "result_summary.json",
    "thresholds": "thresholds.txt",
}
OPTIONAL_ARTIFACT_FILES = {
    "holdout_manifest": "holdout_manifest.json",
    "holdout_source_manifest": "holdout_source_manifest.json",
}
KNOWN_ARTIFACT_FILES = {**ARTIFACT_FILES, **OPTIONAL_ARTIFACT_FILES}
RUN_MANIFEST = "run_manifest.json"
REQUIRED_COMMANDS = {
    "measure",
    "check_thresholds",
    "summarize",
    "extract_result_summary",
}


def load_sibling_tool(name: str) -> Any:
    path = Path(__file__).resolve().parent / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


check_effect_thresholds = load_sibling_tool("check_effect_thresholds")
extract_effect_summary = load_sibling_tool("extract_effect_summary")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json_object(path: Path, label: str, failures: list[str]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        failures.append(f"{label} could not be read: {error}")
        return {}
    except json.JSONDecodeError as error:
        failures.append(f"{label} is not valid JSON: {error}")
        return {}
    if not isinstance(value, dict):
        failures.append(f"{label} must contain a JSON object")
        return {}
    return value


def verify_required_files(artifact_dir: Path, failures: list[str]) -> None:
    for filename in [*ARTIFACT_FILES.values(), RUN_MANIFEST]:
        path = artifact_dir / filename
        if not path.is_file():
            failures.append(f"missing required artifact: {filename}")


def verify_manifest_shape(manifest: dict[str, Any], failures: list[str]) -> None:
    sha256 = manifest.get("sha256")
    if not isinstance(sha256, dict):
        failures.append("run_manifest.json missing sha256 map")
    else:
        missing_hashes = sorted(set(ARTIFACT_FILES) - set(sha256))
        if missing_hashes:
            failures.append(
                "run_manifest.json sha256 map missing keys: "
                + ", ".join(missing_hashes)
            )
        for key in sorted(set(KNOWN_ARTIFACT_FILES) & set(sha256)):
            if not isinstance(sha256[key], str):
                failures.append(f"run_manifest.json sha256.{key} must be a string")

    commands = manifest.get("commands")
    if not isinstance(commands, dict):
        failures.append("run_manifest.json missing commands map")
    else:
        missing_commands = sorted(REQUIRED_COMMANDS - set(commands))
        if missing_commands:
            failures.append(
                "run_manifest.json commands map missing keys: "
                + ", ".join(missing_commands)
            )

    if not isinstance(manifest.get("require_holdout_thresholds"), bool):
        failures.append("run_manifest.json require_holdout_thresholds must be boolean")

    for key in OPTIONAL_ARTIFACT_FILES:
        value = manifest.get(key)
        if value is not None and not isinstance(value, str):
            failures.append(f"run_manifest.json {key} must be a string or null")
        if value is not None and isinstance(sha256, dict) and key not in sha256:
            failures.append(f"run_manifest.json sha256 map missing key: {key}")


def verify_checksums(
    artifact_dir: Path,
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    sha256 = manifest.get("sha256")
    if not isinstance(sha256, dict):
        return
    for key, filename in KNOWN_ARTIFACT_FILES.items():
        expected = sha256.get(key)
        if not isinstance(expected, str):
            continue
        path = artifact_dir / filename
        if not path.is_file():
            failures.append(f"missing checksummed artifact: {filename}")
            continue
        actual = file_sha256(path)
        if actual != expected:
            failures.append(
                f"{filename} sha256 mismatch: expected {expected}, got {actual}"
            )


def verify_holdout_manifest_hashes(
    effect_report: dict[str, Any],
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    metadata = effect_report.get("metadata")
    if not isinstance(metadata, dict):
        return
    sha256 = manifest.get("sha256")
    if not isinstance(sha256, dict):
        return

    holdout_hash = metadata.get("repo_holdout_manifest_sha256")
    if holdout_hash is not None:
        if not isinstance(holdout_hash, str):
            failures.append(
                "effect.json metadata.repo_holdout_manifest_sha256 must be a string"
            )
        elif sha256.get("holdout_manifest") != holdout_hash:
            failures.append(
                "holdout_manifest.json sha256 does not match "
                "effect.json metadata.repo_holdout_manifest_sha256"
            )

    source_hash = metadata.get("repo_holdout_source_manifest_sha256")
    if source_hash is not None:
        if not isinstance(source_hash, str):
            failures.append(
                "effect.json metadata.repo_holdout_source_manifest_sha256 "
                "must be a string"
            )
        elif sha256.get("holdout_source_manifest") != source_hash:
            failures.append(
                "holdout_source_manifest.json sha256 does not match "
                "effect.json metadata.repo_holdout_source_manifest_sha256"
            )


def verify_threshold_log(path: Path, failures: list[str]) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        failures.append(f"thresholds.txt could not be read: {error}")
        return
    if PASS_MARKER not in text:
        failures.append(f"thresholds.txt does not contain {PASS_MARKER!r}")


def verify_result_summary_schema(
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    schema_version = result_summary.get("schema_version")
    if schema_version != EXPECTED_RESULT_SUMMARY_SCHEMA_VERSION:
        failures.append(
            "result_summary.json schema_version must be "
            f"{EXPECTED_RESULT_SUMMARY_SCHEMA_VERSION}, got {schema_version!r}"
        )


def verify_residual_gap_clusters(
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    holdout = result_summary.get("repo_temporal_holdout")
    if not isinstance(holdout, dict):
        return
    verify_holdout_residual_gap_clusters(
        holdout,
        "repo_temporal_holdout",
        failures,
    )
    predictable = holdout.get("predictable_only")
    if isinstance(predictable, dict):
        verify_holdout_residual_gap_clusters(
            predictable,
            "repo_temporal_holdout.predictable_only",
            failures,
        )


def verify_holdout_residual_gap_clusters(
    holdout: dict[str, Any],
    label: str,
    failures: list[str],
) -> None:
    k = holdout.get("k", 5)
    metric = f"oracle_gap_average_precision_at_{k}"
    hybrid = (
        holdout.get("oracle_normalized", {})
        .get("workspace_related_hybrid", {})
        if isinstance(holdout.get("oracle_normalized"), dict)
        else {}
    )
    if not isinstance(hybrid, dict):
        return
    gap = hybrid.get(metric)
    if gap is None or float(gap) <= 0.0:
        return
    clusters = holdout.get("residual_gap_clusters")
    if not isinstance(clusters, list) or not clusters:
        failures.append(
            f"result_summary.json {label} missing residual_gap_clusters "
            f"despite positive {metric}"
        )


def verify_result_summary_matches_report(
    effect_report: dict[str, Any],
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    expected = extract_effect_summary.extract_summary(effect_report)
    if result_summary != expected:
        failures.append(
            "result_summary.json does not match extract_effect_summary.py output"
        )


def verify_threshold_recheck(
    effect_report: dict[str, Any],
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    require_holdout = manifest.get("require_holdout_thresholds")
    if not isinstance(require_holdout, bool):
        return
    threshold_failures = check_effect_thresholds.check_report(
        effect_report,
        require_holdout=require_holdout,
    )
    for failure in threshold_failures:
        failures.append(f"threshold recheck failed: {failure}")


def verify_artifact_directory(artifact_dir: Path) -> list[str]:
    artifact_dir = artifact_dir.resolve()
    failures: list[str] = []
    if not artifact_dir.is_dir():
        return [f"artifact directory does not exist: {artifact_dir}"]

    verify_required_files(artifact_dir, failures)
    if failures:
        return failures

    effect_report = load_json_object(
        artifact_dir / "effect.json",
        "effect.json",
        failures,
    )
    result_summary = load_json_object(
        artifact_dir / "result_summary.json",
        "result_summary.json",
        failures,
    )
    manifest = load_json_object(
        artifact_dir / RUN_MANIFEST,
        RUN_MANIFEST,
        failures,
    )

    if "measurements" not in effect_report:
        failures.append("effect.json missing measurements")
    verify_result_summary_schema(result_summary, failures)
    verify_residual_gap_clusters(result_summary, failures)
    verify_result_summary_matches_report(effect_report, result_summary, failures)
    verify_manifest_shape(manifest, failures)
    verify_checksums(artifact_dir, manifest, failures)
    verify_holdout_manifest_hashes(effect_report, manifest, failures)
    verify_threshold_recheck(effect_report, manifest, failures)
    verify_threshold_log(artifact_dir / "thresholds.txt", failures)
    return failures


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path, help="effect artifact directory")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    failures = verify_artifact_directory(args.artifact_dir)
    if failures:
        print("effect artifact verification failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("effect artifact verification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
