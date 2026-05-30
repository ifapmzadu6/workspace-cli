#!/usr/bin/env python3
"""Verify a generated effect artifact directory."""

from __future__ import annotations

import argparse
import hashlib
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
RUN_MANIFEST = "run_manifest.json"
REQUIRED_COMMANDS = {
    "measure",
    "check_thresholds",
    "summarize",
    "extract_result_summary",
}


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
        for key in sorted(set(ARTIFACT_FILES) & set(sha256)):
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


def verify_checksums(
    artifact_dir: Path,
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    sha256 = manifest.get("sha256")
    if not isinstance(sha256, dict):
        return
    for key, filename in ARTIFACT_FILES.items():
        expected = sha256.get(key)
        if not isinstance(expected, str):
            continue
        actual = file_sha256(artifact_dir / filename)
        if actual != expected:
            failures.append(
                f"{filename} sha256 mismatch: expected {expected}, got {actual}"
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
    verify_manifest_shape(manifest, failures)
    verify_checksums(artifact_dir, manifest, failures)
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
