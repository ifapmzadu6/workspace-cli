#!/usr/bin/env python3
"""Verify a generated effect artifact directory."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


PASS_MARKER = "effect threshold check passed"
EXPECTED_RUN_MANIFEST_SCHEMA_VERSION = 1
EXPECTED_EFFECT_METADATA_SCHEMA_VERSION = 2
EXPECTED_RESULT_SUMMARY_SCHEMA_VERSION = 7
FLOAT_TOLERANCE = 1e-9

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
    "verify_artifacts",
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
summarize_effect = load_sibling_tool("summarize_effect")


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
    schema_version = manifest.get("schema_version")
    if schema_version != EXPECTED_RUN_MANIFEST_SCHEMA_VERSION:
        failures.append(
            "run_manifest.json schema_version must be "
            f"{EXPECTED_RUN_MANIFEST_SCHEMA_VERSION}, got {schema_version!r}"
        )
    generated_at = manifest.get("generated_at")
    if not is_nonempty_string(generated_at):
        failures.append("run_manifest.json generated_at must be a non-empty string")
    else:
        try:
            parsed = datetime.fromisoformat(generated_at.replace("Z", "+00:00"))
        except ValueError:
            failures.append(
                "run_manifest.json generated_at must be an ISO-8601 timestamp"
            )
        else:
            if parsed.tzinfo is None or parsed.tzinfo.utcoffset(parsed) is None:
                failures.append(
                    "run_manifest.json generated_at must include a timezone"
                )

    verify_manifest_artifact_paths(manifest, failures)
    for key in ("workspace_repo", "output_dir"):
        if not is_nonempty_string(manifest.get(key)):
            failures.append(f"run_manifest.json {key} must be a non-empty string")

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
        verify_manifest_commands(manifest, commands, failures)

    if not isinstance(manifest.get("require_holdout_thresholds"), bool):
        failures.append("run_manifest.json require_holdout_thresholds must be boolean")
    if not isinstance(manifest.get("require_clean_workspace_verifier"), bool):
        failures.append(
            "run_manifest.json require_clean_workspace_verifier must be boolean"
        )
    paper_manifest = manifest.get("paper_manifest")
    if paper_manifest is not None and not is_nonempty_string(paper_manifest):
        failures.append("run_manifest.json paper_manifest must be a string or null")
    if manifest.get("require_holdout_thresholds") is True and not is_nonempty_string(
        paper_manifest
    ):
        failures.append(
            "run_manifest.json paper_manifest must be a non-empty string when "
            "require_holdout_thresholds is true"
        )

    for key in OPTIONAL_ARTIFACT_FILES:
        value = manifest.get(key)
        if value is not None and not isinstance(value, str):
            failures.append(f"run_manifest.json {key} must be a string or null")
        if value is not None and isinstance(sha256, dict) and key not in sha256:
            failures.append(f"run_manifest.json sha256 map missing key: {key}")


def verify_manifest_artifact_paths(
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    for key, filename in ARTIFACT_FILES.items():
        verify_manifest_artifact_path(manifest, key, filename, failures, required=True)
    for key, filename in OPTIONAL_ARTIFACT_FILES.items():
        verify_manifest_artifact_path(manifest, key, filename, failures, required=False)


def verify_manifest_artifact_path(
    manifest: dict[str, Any],
    key: str,
    filename: str,
    failures: list[str],
    *,
    required: bool,
) -> None:
    value = manifest.get(key)
    if value is None:
        if required:
            failures.append(f"run_manifest.json {key} must be a string path")
        return
    if not is_nonempty_string(value):
        expected_type = "a string path" if required else "a string path or null"
        failures.append(f"run_manifest.json {key} must be {expected_type}")
        return
    if Path(value).name != filename:
        failures.append(f"run_manifest.json {key} must point to {filename}")


def verify_manifest_commands(
    manifest: dict[str, Any],
    commands: dict[str, Any],
    failures: list[str],
) -> None:
    expected_tools = {
        "measure": "measure_effect.py",
        "check_thresholds": "check_effect_thresholds.py",
        "summarize": "summarize_effect.py",
        "extract_result_summary": "extract_effect_summary.py",
        "verify_artifacts": "verify_effect_artifacts.py",
    }
    for key in sorted(REQUIRED_COMMANDS & set(commands)):
        command = commands.get(key)
        if not is_string_list(command):
            failures.append(f"run_manifest.json commands.{key} must be a string list")
            continue
        expected_tool = expected_tools[key]
        if not command_contains_basename(command, expected_tool):
            failures.append(
                f"run_manifest.json commands.{key} must invoke {expected_tool}"
            )

    if manifest.get("require_holdout_thresholds") is True:
        require_command_arg(
            commands,
            "measure",
            "--repo-holdout-manifest",
            failures,
            "require_holdout_thresholds",
        )
        require_command_arg(
            commands,
            "check_thresholds",
            "--require-holdout",
            failures,
            "require_holdout_thresholds",
        )
    if manifest.get("require_clean_workspace_verifier") is True:
        require_command_arg(
            commands,
            "verify_artifacts",
            "--require-clean-workspace",
            failures,
            "require_clean_workspace_verifier",
        )


def require_command_arg(
    commands: dict[str, Any],
    key: str,
    arg: str,
    failures: list[str],
    flag_name: str,
) -> None:
    command = commands.get(key)
    if not is_string_list(command):
        return
    if arg not in command:
        failures.append(
            f"run_manifest.json commands.{key} must include {arg} when "
            f"{flag_name} is true"
        )


def is_string_list(value: Any) -> bool:
    return isinstance(value, list) and all(isinstance(part, str) for part in value)


def is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def command_contains_basename(command: list[str], expected_basename: str) -> bool:
    return any(Path(part).name == expected_basename for part in command)


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


def verify_holdout_manifest_contents(
    effect_report: dict[str, Any],
    artifact_dir: Path,
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    if manifest.get("require_holdout_thresholds") is not True:
        return
    metadata = effect_report.get("metadata")
    if not isinstance(metadata, dict):
        failures.append("effect.json metadata must be present for holdout verification")
        return
    if not is_nonempty_string(metadata.get("repo_holdout_manifest_sha256")):
        failures.append(
            "effect.json metadata.repo_holdout_manifest_sha256 must be a "
            "non-empty string when require_holdout_thresholds is true"
        )
    if not is_nonempty_string(manifest.get("holdout_manifest")):
        failures.append(
            "run_manifest.json holdout_manifest must be a non-empty string when "
            "require_holdout_thresholds is true"
        )

    holdout_manifest = load_json_object(
        artifact_dir / OPTIONAL_ARTIFACT_FILES["holdout_manifest"],
        "holdout_manifest.json",
        failures,
    )
    verify_holdout_entries_match(
        metadata.get("repo_holdouts"),
        holdout_manifest.get("repo_holdouts"),
        failures,
        source_label="holdout_manifest.json",
        compare_repo=True,
    )
    verify_prepared_source_manifest(
        metadata,
        holdout_manifest,
        artifact_dir,
        manifest,
        failures,
    )


def verify_prepared_source_manifest(
    metadata: dict[str, Any],
    holdout_manifest: dict[str, Any],
    artifact_dir: Path,
    run_manifest: dict[str, Any],
    failures: list[str],
) -> None:
    metadata_source = metadata.get("repo_holdout_source_manifest")
    metadata_source_hash = metadata.get("repo_holdout_source_manifest_sha256")
    if metadata_source is None and metadata_source_hash is None:
        return
    prepared_from = holdout_manifest.get("prepared_from")
    if not isinstance(prepared_from, dict):
        failures.append(
            "holdout_manifest.json prepared_from must be present when "
            "effect.json metadata records a source holdout manifest"
        )
        return
    source = prepared_from.get("manifest")
    if source != metadata_source:
        failures.append(
            "holdout_manifest.json prepared_from.manifest does not match "
            "effect.json metadata.repo_holdout_source_manifest"
        )
    source_hash = prepared_from.get("manifest_sha256")
    if source_hash != metadata_source_hash:
        failures.append(
            "holdout_manifest.json prepared_from.manifest_sha256 does not match "
            "effect.json metadata.repo_holdout_source_manifest_sha256"
        )
    if not is_nonempty_string(run_manifest.get("holdout_source_manifest")):
        failures.append(
            "run_manifest.json holdout_source_manifest must be a non-empty string "
            "when effect.json metadata records a source holdout manifest"
        )
    source_manifest = load_json_object(
        artifact_dir / OPTIONAL_ARTIFACT_FILES["holdout_source_manifest"],
        "holdout_source_manifest.json",
        failures,
    )
    verify_holdout_entries_match(
        metadata.get("repo_holdouts"),
        source_manifest.get("repo_holdouts"),
        failures,
        source_label="holdout_source_manifest.json",
        compare_repo=False,
    )


def verify_holdout_entries_match(
    metadata_entries: Any,
    manifest_entries: Any,
    failures: list[str],
    *,
    source_label: str,
    compare_repo: bool,
) -> None:
    if not isinstance(metadata_entries, list) or not metadata_entries:
        failures.append("effect.json metadata.repo_holdouts must be a non-empty list")
        return
    if not isinstance(manifest_entries, list) or not manifest_entries:
        failures.append(f"{source_label} repo_holdouts must be a non-empty list")
        return
    if len(metadata_entries) != len(manifest_entries):
        failures.append(
            f"{source_label} repo_holdouts length does not match "
            "effect.json metadata.repo_holdouts"
        )
    fields = ["ref", "remote_url"]
    if compare_repo:
        fields.insert(0, "repo")
    for index, (metadata_entry, manifest_entry) in enumerate(
        zip(metadata_entries, manifest_entries)
    ):
        entry_label = f"{source_label} repo_holdouts[{index}]"
        if not isinstance(metadata_entry, dict):
            failures.append(
                f"effect.json metadata.repo_holdouts[{index}] must be an object"
            )
            continue
        if not isinstance(manifest_entry, dict):
            failures.append(f"{entry_label} must be an object")
            continue
        for field in fields:
            metadata_value = metadata_entry.get(field)
            manifest_value = manifest_entry.get(field)
            if metadata_value != manifest_value:
                failures.append(
                    f"{entry_label}.{field} does not match effect.json "
                    f"metadata.repo_holdouts[{index}].{field}"
                )


def verify_threshold_log(
    effect_report: dict[str, Any],
    manifest: dict[str, Any],
    path: Path,
    failures: list[str],
) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        failures.append(f"thresholds.txt could not be read: {error}")
        return
    if PASS_MARKER not in text:
        failures.append(f"thresholds.txt does not contain {PASS_MARKER!r}")
    require_holdout = manifest.get("require_holdout_thresholds")
    if not isinstance(require_holdout, bool):
        return
    expected = check_effect_thresholds.render_success_output(
        effect_report,
        require_holdout=require_holdout,
    )
    if text != expected:
        failures.append(
            "thresholds.txt does not match check_effect_thresholds.py output"
        )


def verify_clean_workspace_metadata(
    effect_report: dict[str, Any],
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    for label, report in (
        ("effect.json", effect_report),
        ("result_summary.json", result_summary),
    ):
        metadata = report.get("metadata")
        if not isinstance(metadata, dict):
            failures.append(f"{label} metadata must be present for clean verification")
            continue
        workspace_commit = metadata.get("workspace_commit")
        if not isinstance(workspace_commit, str) or not workspace_commit.strip():
            failures.append(
                f"{label} metadata.workspace_commit must be a non-empty string"
            )
        if metadata.get("workspace_dirty") is not False:
            failures.append(f"{label} metadata.workspace_dirty must be false")
        status_line_count = metadata.get("workspace_status_line_count")
        if (
            not isinstance(status_line_count, int)
            or isinstance(status_line_count, bool)
            or status_line_count != 0
        ):
            failures.append(
                f"{label} metadata.workspace_status_line_count must be integer 0"
            )


def verify_clean_workspace_manifest_command(
    manifest: dict[str, Any],
    failures: list[str],
) -> None:
    commands = manifest.get("commands")
    if not isinstance(commands, dict):
        return
    verify_command = commands.get("verify_artifacts")
    if not isinstance(verify_command, list) or not all(
        isinstance(part, str) for part in verify_command
    ):
        failures.append(
            "run_manifest.json commands.verify_artifacts must be a string list"
        )
        return
    if "--require-clean-workspace" not in verify_command:
        failures.append(
            "run_manifest.json commands.verify_artifacts must include "
            "--require-clean-workspace"
        )


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
    verify_threshold_margin_schema(result_summary, failures)


def verify_effect_report_schema(
    effect_report: dict[str, Any],
    failures: list[str],
) -> None:
    metadata = effect_report.get("metadata")
    if not isinstance(metadata, dict):
        failures.append("effect.json metadata must be an object")
        return
    schema_version = metadata.get("schema_version")
    if schema_version != EXPECTED_EFFECT_METADATA_SCHEMA_VERSION:
        failures.append(
            "effect.json metadata.schema_version must be "
            f"{EXPECTED_EFFECT_METADATA_SCHEMA_VERSION}, got {schema_version!r}"
        )


def verify_threshold_margin_schema(
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    margins = result_summary.get("threshold_margins")
    if not isinstance(margins, list):
        failures.append("result_summary.json threshold_margins must be a list")
        return
    seen_labels: set[str] = set()
    for index, entry in enumerate(margins):
        label = f"result_summary.json threshold_margins[{index}]"
        if not isinstance(entry, dict):
            failures.append(f"{label} must be an object")
            continue
        margin_label = entry.get("label")
        if not isinstance(margin_label, str):
            failures.append(f"{label}.label must be a string")
        elif margin_label in seen_labels:
            failures.append(f"{label}.label must be unique: {margin_label}")
        else:
            seen_labels.add(margin_label)
        status = entry.get("status")
        if status not in {"pass", "fail", "missing"}:
            failures.append(f"{label}.status must be pass, fail, or missing")
        if entry.get("missing"):
            if status != "missing":
                failures.append(f"{label}.status must be missing when missing is true")
            continue
        if status == "missing":
            failures.append(f"{label}.status cannot be missing without missing=true")
        gate = entry.get("gate")
        if gate not in {"minimum", "maximum"}:
            failures.append(f"{label}.gate must be minimum or maximum")
            continue
        value = entry.get("value")
        if not is_json_number(value):
            failures.append(f"{label}.value must be numeric")
        if gate == "minimum":
            for field in ("minimum", "margin"):
                if not is_json_number(entry.get(field)):
                    failures.append(f"{label}.{field} must be numeric")
            if all(
                is_json_number(entry.get(field))
                for field in ("value", "minimum", "margin")
            ):
                verify_threshold_floor_margin(entry, label, failures)
        else:
            for field in ("maximum", "headroom"):
                if not is_json_number(entry.get(field)):
                    failures.append(f"{label}.{field} must be numeric")
            if all(
                is_json_number(entry.get(field))
                for field in ("value", "maximum", "headroom")
            ):
                verify_threshold_ceiling_margin(entry, label, failures)


def is_json_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def verify_threshold_floor_margin(
    entry: dict[str, Any],
    label: str,
    failures: list[str],
) -> None:
    value = float(entry["value"])
    minimum = float(entry["minimum"])
    margin = float(entry["margin"])
    expected_margin = value - minimum
    if abs(margin - expected_margin) > FLOAT_TOLERANCE:
        failures.append(
            f"{label}.margin must equal value - minimum: "
            f"{margin} != {expected_margin}"
        )
    verify_threshold_status(entry, label, margin, failures, field="margin")


def verify_threshold_ceiling_margin(
    entry: dict[str, Any],
    label: str,
    failures: list[str],
) -> None:
    value = float(entry["value"])
    maximum = float(entry["maximum"])
    headroom = float(entry["headroom"])
    expected_headroom = maximum - value
    if abs(headroom - expected_headroom) > FLOAT_TOLERANCE:
        failures.append(
            f"{label}.headroom must equal maximum - value: "
            f"{headroom} != {expected_headroom}"
        )
    verify_threshold_status(entry, label, headroom, failures, field="headroom")


def verify_threshold_status(
    entry: dict[str, Any],
    label: str,
    margin: float,
    failures: list[str],
    *,
    field: str,
) -> None:
    expected_status = "pass" if margin + FLOAT_TOLERANCE >= 0.0 else "fail"
    if entry.get("status") != expected_status:
        failures.append(
            f"{label}.status must be {expected_status} for {field}={margin}"
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


def verify_residual_pair_conflicts(
    result_summary: dict[str, Any],
    failures: list[str],
) -> None:
    holdout = result_summary.get("repo_temporal_holdout")
    if not isinstance(holdout, dict):
        return
    verify_holdout_residual_pair_conflicts(
        holdout,
        "repo_temporal_holdout",
        failures,
    )
    predictable = holdout.get("predictable_only")
    if isinstance(predictable, dict):
        verify_holdout_residual_pair_conflicts(
            predictable,
            "repo_temporal_holdout.predictable_only",
            failures,
        )


def verify_holdout_residual_pair_conflicts(
    holdout: dict[str, Any],
    label: str,
    failures: list[str],
) -> None:
    conflicts = holdout.get("residual_pair_conflicts")
    if conflicts is None:
        return
    if not isinstance(conflicts, list):
        failures.append(
            f"result_summary.json {label}.residual_pair_conflicts must be a list"
        )
        return
    for conflict_index, conflict in enumerate(conflicts):
        conflict_label = f"{label}.residual_pair_conflicts[{conflict_index}]"
        verify_residual_pair_conflict_schema(conflict, conflict_label, failures)


def verify_residual_pair_conflict_schema(
    conflict: Any,
    label: str,
    failures: list[str],
) -> None:
    if not isinstance(conflict, dict):
        failures.append(f"result_summary.json {label} must be an object")
        return

    for field in ("seed", "candidate", "expected_key", "method"):
        value = conflict.get(field)
        if not isinstance(value, str) or not value:
            failures.append(
                f"result_summary.json {label}.{field} must be a non-empty string"
            )
    for field in ("repo", "repo_name"):
        value = conflict.get(field)
        if value is not None and not isinstance(value, str):
            failures.append(
                f"result_summary.json {label}.{field} must be a string or null"
            )

    true_count = verify_positive_integer_field(
        conflict,
        "true_target_count",
        label,
        failures,
    )
    false_count = verify_positive_integer_field(
        conflict,
        "residual_false_positive_count",
        label,
        failures,
    )
    true_commits = verify_non_empty_string_list_field(
        conflict,
        "true_target_commits",
        label,
        failures,
    )
    false_commits = verify_non_empty_string_list_field(
        conflict,
        "residual_false_positive_commits",
        label,
        failures,
    )
    if true_count is not None and true_commits is not None and len(true_commits) > true_count:
        failures.append(
            f"result_summary.json {label}.true_target_commits cannot exceed "
            "true_target_count"
        )
    if false_count is not None and false_commits is not None and len(false_commits) > false_count:
        failures.append(
            f"result_summary.json {label}.residual_false_positive_commits cannot exceed "
            "residual_false_positive_count"
        )


def verify_positive_integer_field(
    value: dict[str, Any],
    field: str,
    label: str,
    failures: list[str],
) -> int | None:
    field_value = value.get(field)
    if not isinstance(field_value, int) or isinstance(field_value, bool) or field_value <= 0:
        failures.append(
            f"result_summary.json {label}.{field} must be a positive integer"
        )
        return None
    return field_value


def verify_non_empty_string_list_field(
    value: dict[str, Any],
    field: str,
    label: str,
    failures: list[str],
) -> list[str] | None:
    values = verify_string_list_field(value, field, label, failures)
    if values is not None and not values:
        failures.append(f"result_summary.json {label}.{field} must be non-empty")
    return values


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
        return
    verify_residual_gap_cluster_schema(clusters, label, failures)


def verify_residual_gap_cluster_schema(
    clusters: list[Any],
    label: str,
    failures: list[str],
) -> None:
    for cluster_index, cluster in enumerate(clusters):
        if not isinstance(cluster, dict):
            failures.append(
                f"result_summary.json {label}.residual_gap_clusters[{cluster_index}] "
                "must be an object"
            )
            continue
        cases = cluster.get("top_residual_cases")
        if not isinstance(cases, list) or not cases:
            failures.append(
                f"result_summary.json {label}.residual_gap_clusters[{cluster_index}] "
                "missing top_residual_cases"
            )
            continue
        for field in (
            "missing_expected_counts",
            "missing_predictable_expected_counts",
            "missing_unpredictable_expected_counts",
            "method_false_positive_counts",
        ):
            verify_path_count_rows(
                cluster.get(field),
                f"{label}.residual_gap_clusters[{cluster_index}].{field}",
                failures,
            )
        for case_index, case in enumerate(cases):
            case_label = (
                f"{label}.residual_gap_clusters[{cluster_index}]"
                f".top_residual_cases[{case_index}]"
            )
            verify_residual_gap_case_schema(
                case,
                case_label,
                failures,
            )


def verify_path_count_rows(value: Any, label: str, failures: list[str]) -> None:
    if not isinstance(value, list):
        failures.append(f"result_summary.json {label} must be a list")
        return
    for index, entry in enumerate(value):
        entry_label = f"{label}[{index}]"
        if not isinstance(entry, dict):
            failures.append(f"result_summary.json {entry_label} must be an object")
            continue
        path = entry.get("path")
        if not isinstance(path, str):
            failures.append(f"result_summary.json {entry_label}.path must be a string")
        count = entry.get("count")
        if not isinstance(count, int) or isinstance(count, bool) or count <= 0:
            failures.append(
                f"result_summary.json {entry_label}.count must be a positive integer"
            )


def verify_residual_gap_case_schema(
    case: Any,
    label: str,
    failures: list[str],
) -> None:
    if not isinstance(case, dict):
        failures.append(f"result_summary.json {label} must be an object")
        return
    string_lists: dict[str, list[str]] = {}
    for field in (
        "missing_expected",
        "method_hits",
        "missing_predictable_expected",
        "missing_unpredictable_expected",
        "method_false_positives",
        "method_top",
    ):
        values = verify_string_list_field(case, field, label, failures)
        if values is not None:
            string_lists[field] = values
    ranks = case.get("missing_expected_ranks")
    if not isinstance(ranks, list):
        failures.append(
            f"result_summary.json {label}.missing_expected_ranks must be a list"
        )
    else:
        rank_paths = []
        for rank_index, entry in enumerate(ranks):
            entry_label = f"{label}.missing_expected_ranks[{rank_index}]"
            if not isinstance(entry, dict):
                failures.append(f"result_summary.json {entry_label} must be an object")
                continue
            path = entry.get("path")
            if not isinstance(path, str):
                failures.append(f"result_summary.json {entry_label}.path must be a string")
            else:
                rank_paths.append(path)
            rank = entry.get("rank")
            if rank is not None and (
                not isinstance(rank, int) or isinstance(rank, bool)
            ):
                failures.append(
                    f"result_summary.json {entry_label}.rank must be an integer or null"
                )
            score = entry.get("score")
            if score is not None and not is_json_number(score):
                failures.append(
                    f"result_summary.json {entry_label}.score must be a number"
                )
            if isinstance(rank, int) and not isinstance(rank, bool) and score is None:
                failures.append(
                    f"result_summary.json {entry_label}.score is required "
                    "when rank is present"
                )
        missing_expected = string_lists.get("missing_expected")
        if missing_expected is not None and rank_paths != missing_expected:
            failures.append(
                f"result_summary.json {label}.missing_expected_ranks paths "
                "must match missing_expected"
            )
    top_ranked = case.get("method_top_ranked")
    if not isinstance(top_ranked, list):
        failures.append(
            f"result_summary.json {label}.method_top_ranked must be a list"
        )
    else:
        top_paths = []
        for rank_index, entry in enumerate(top_ranked):
            entry_label = f"{label}.method_top_ranked[{rank_index}]"
            path = verify_ranked_candidate_schema(
                entry,
                entry_label,
                failures,
                require_score=True,
            )
            if path is not None:
                top_paths.append(path)
            if isinstance(entry, dict) and entry.get("rank") != rank_index + 1:
                failures.append(
                    f"result_summary.json {entry_label}.rank must be {rank_index + 1}"
                )
        method_top = string_lists.get("method_top")
        if method_top is not None and top_paths != method_top:
            failures.append(
                f"result_summary.json {label}.method_top_ranked paths "
                "must match method_top"
            )


def verify_string_list_field(
    case: dict[str, Any],
    field: str,
    label: str,
    failures: list[str],
) -> list[str] | None:
    values = case.get(field)
    if not isinstance(values, list):
        failures.append(f"result_summary.json {label}.{field} must be a list")
        return None
    result = []
    for index, value in enumerate(values):
        if not isinstance(value, str):
            failures.append(
                f"result_summary.json {label}.{field}[{index}] must be a string"
            )
            continue
        result.append(value)
    return result


def verify_ranked_candidate_schema(
    entry: Any,
    label: str,
    failures: list[str],
    *,
    require_score: bool = False,
) -> str | None:
    if not isinstance(entry, dict):
        failures.append(f"result_summary.json {label} must be an object")
        return None
    path = entry.get("path")
    if not isinstance(path, str):
        failures.append(f"result_summary.json {label}.path must be a string")
        path = None
    rank = entry.get("rank")
    if not isinstance(rank, int) or isinstance(rank, bool):
        failures.append(f"result_summary.json {label}.rank must be an integer")
    score = entry.get("score")
    if score is None and require_score:
        failures.append(f"result_summary.json {label}.score is required")
    elif score is not None and not is_json_number(score):
        failures.append(f"result_summary.json {label}.score must be a number")
    return path


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


def verify_markdown_matches_report(
    effect_report: dict[str, Any],
    markdown_path: Path,
    failures: list[str],
) -> None:
    try:
        actual = markdown_path.read_text(encoding="utf-8")
    except OSError as error:
        failures.append(f"effect.md could not be read: {error}")
        return
    expected = summarize_effect.render_report(effect_report)
    if actual != expected:
        failures.append("effect.md does not match summarize_effect.py output")


def verify_markdown_residual_count_tables(
    result_summary: dict[str, Any],
    markdown_path: Path,
    failures: list[str],
) -> None:
    try:
        markdown = markdown_path.read_text(encoding="utf-8")
    except OSError as error:
        failures.append(f"effect.md could not be read for residual count check: {error}")
        return

    holdout = result_summary.get("repo_temporal_holdout")
    if not isinstance(holdout, dict):
        return
    verify_holdout_markdown_residual_count_table(
        holdout,
        "repo_temporal_holdout",
        markdown,
        failures,
    )
    verify_holdout_markdown_residual_pair_conflict_table(
        holdout,
        "repo_temporal_holdout",
        markdown,
        failures,
    )
    predictable = holdout.get("predictable_only")
    if isinstance(predictable, dict):
        verify_holdout_markdown_residual_count_table(
            predictable,
            "repo_temporal_holdout.predictable_only",
            markdown,
            failures,
        )
        verify_holdout_markdown_residual_pair_conflict_table(
            predictable,
            "repo_temporal_holdout.predictable_only",
            markdown,
            failures,
        )


def verify_holdout_markdown_residual_count_table(
    holdout: dict[str, Any],
    label: str,
    markdown: str,
    failures: list[str],
) -> None:
    clusters = holdout.get("residual_gap_clusters")
    if not isinstance(clusters, list) or not clusters:
        return
    if "missing counts" not in markdown:
        failures.append(f"effect.md missing missing counts column for {label}")
    if "predictable miss counts" not in markdown:
        failures.append(
            f"effect.md missing predictable miss counts column for {label}"
        )
    if "new miss counts" not in markdown:
        failures.append(f"effect.md missing new miss counts column for {label}")
    if "false-positive counts" not in markdown:
        failures.append(f"effect.md missing false-positive counts column for {label}")
    for cluster_index, cluster in enumerate(clusters):
        if not isinstance(cluster, dict):
            continue
        cluster_label = f"{label}.residual_gap_clusters[{cluster_index}]"
        for field in (
            "missing_expected_counts",
            "missing_predictable_expected_counts",
            "missing_unpredictable_expected_counts",
            "method_false_positive_counts",
        ):
            text = format_residual_path_counts(cluster.get(field))
            if text and text not in markdown:
                failures.append(f"effect.md missing {cluster_label}.{field}: {text}")


def verify_holdout_markdown_residual_pair_conflict_table(
    holdout: dict[str, Any],
    label: str,
    markdown: str,
    failures: list[str],
) -> None:
    conflicts = holdout.get("residual_pair_conflicts")
    if not isinstance(conflicts, list) or not conflicts:
        return
    if "Residual Pair Conflicts" not in markdown:
        failures.append(f"effect.md missing residual pair conflict table for {label}")
    if "residual false positives" not in markdown:
        failures.append(
            f"effect.md missing residual false positives column for {label}"
        )
    for conflict_index, conflict in enumerate(conflicts):
        if not isinstance(conflict, dict):
            continue
        conflict_label = f"{label}.residual_pair_conflicts[{conflict_index}]"
        seed = conflict.get("seed")
        candidate = conflict.get("candidate")
        if isinstance(seed, str) and seed and seed not in markdown:
            failures.append(f"effect.md missing {conflict_label}.seed: {seed}")
        if isinstance(candidate, str) and candidate and candidate not in markdown:
            failures.append(
                f"effect.md missing {conflict_label}.candidate: {candidate}"
            )
        for field in (
            "true_target_commits",
            "residual_false_positive_commits",
        ):
            text = format_residual_commit_list(conflict.get(field))
            if text and text not in markdown:
                failures.append(f"effect.md missing {conflict_label}.{field}: {text}")


def format_residual_commit_list(value: Any, *, limit: int = 3) -> str:
    if not isinstance(value, list) or not value:
        return ""
    commits = [
        short_residual_commit(commit)
        for commit in value[:limit]
        if isinstance(commit, str)
    ]
    if len(value) > limit:
        commits.append(f"+{len(value) - limit} more")
    return ", ".join(commits)


def short_residual_commit(commit: str) -> str:
    return commit[:10]


def format_residual_path_counts(value: Any, *, limit: int = 4) -> str:
    if not isinstance(value, list) or not value:
        return ""
    rendered = []
    for entry in value[:limit]:
        if not isinstance(entry, dict):
            continue
        path = entry.get("path")
        count = entry.get("count")
        if isinstance(path, str) and isinstance(count, int) and not isinstance(count, bool):
            rendered.append(f"{path} x{count}")
    if len(value) > limit:
        rendered.append(f"+{len(value) - limit} more")
    return ", ".join(rendered)


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


def verify_artifact_directory(
    artifact_dir: Path,
    *,
    require_clean_workspace: bool = False,
) -> list[str]:
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
    verify_effect_report_schema(effect_report, failures)
    verify_result_summary_schema(result_summary, failures)
    if require_clean_workspace:
        verify_clean_workspace_metadata(effect_report, result_summary, failures)
        verify_clean_workspace_manifest_command(manifest, failures)
    verify_residual_gap_clusters(result_summary, failures)
    verify_residual_pair_conflicts(result_summary, failures)
    verify_result_summary_matches_report(effect_report, result_summary, failures)
    verify_markdown_matches_report(effect_report, artifact_dir / "effect.md", failures)
    verify_markdown_residual_count_tables(
        result_summary,
        artifact_dir / "effect.md",
        failures,
    )
    verify_manifest_shape(manifest, failures)
    verify_checksums(artifact_dir, manifest, failures)
    verify_holdout_manifest_hashes(effect_report, manifest, failures)
    verify_holdout_manifest_contents(effect_report, artifact_dir, manifest, failures)
    verify_threshold_recheck(effect_report, manifest, failures)
    verify_threshold_log(
        effect_report,
        manifest,
        artifact_dir / "thresholds.txt",
        failures,
    )
    return failures


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--require-clean-workspace",
        action="store_true",
        help="fail unless artifact metadata records a clean workspace commit",
    )
    parser.add_argument("artifact_dir", type=Path, help="effect artifact directory")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    failures = verify_artifact_directory(
        args.artifact_dir,
        require_clean_workspace=args.require_clean_workspace,
    )
    if failures:
        print("effect artifact verification failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("effect artifact verification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
