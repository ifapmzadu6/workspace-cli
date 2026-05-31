#!/usr/bin/env python3
"""Generate effect measurement artifacts in one reproducible directory."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, TextIO


ROOT = Path(__file__).resolve().parent.parent
TOOLS_DIR = ROOT / "tools"
DEFAULT_OUTPUT_DIR = ROOT / "target" / "effect-artifacts"
DEFAULT_PAPER_MANIFEST = TOOLS_DIR / "effect_paper_holdouts.json"


Runner = Callable[..., subprocess.CompletedProcess[str]]


def repo_relative(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def resolve_user_path(path: Path) -> Path:
    if path.is_absolute():
        return path
    return (Path.cwd() / path).resolve()


def build_plan(output_dir: Path, manifest: Path | None) -> dict[str, Any]:
    output_dir = resolve_user_path(output_dir)
    manifest = resolve_user_path(manifest) if manifest is not None else None
    json_path = output_dir / "effect.json"
    markdown_path = output_dir / "effect.md"
    result_summary_path = output_dir / "result_summary.json"
    threshold_path = output_dir / "thresholds.txt"
    run_manifest_path = output_dir / "run_manifest.json"
    holdout_manifest_path = output_dir / "holdout_manifest.json"
    holdout_source_manifest_path = output_dir / "holdout_source_manifest.json"

    measurement_command = [
        sys.executable,
        str(TOOLS_DIR / "measure_effect.py"),
    ]
    if manifest is not None:
        measurement_command.extend(["--repo-holdout-manifest", str(manifest)])

    threshold_command = [
        sys.executable,
        str(TOOLS_DIR / "check_effect_thresholds.py"),
    ]
    if manifest is not None:
        threshold_command.append("--require-holdout")
    threshold_command.append(str(json_path))

    summary_command = [
        sys.executable,
        str(TOOLS_DIR / "summarize_effect.py"),
        str(json_path),
    ]
    result_summary_command = [
        sys.executable,
        str(TOOLS_DIR / "extract_effect_summary.py"),
        str(json_path),
    ]
    verify_command = [
        sys.executable,
        str(TOOLS_DIR / "verify_effect_artifacts.py"),
        str(output_dir),
    ]

    return {
        "output_dir": output_dir,
        "json_path": json_path,
        "markdown_path": markdown_path,
        "result_summary_path": result_summary_path,
        "threshold_path": threshold_path,
        "run_manifest_path": run_manifest_path,
        "holdout_manifest_path": holdout_manifest_path,
        "holdout_source_manifest_path": holdout_source_manifest_path,
        "measurement_command": measurement_command,
        "threshold_command": threshold_command,
        "summary_command": summary_command,
        "result_summary_command": result_summary_command,
        "verify_command": verify_command,
        "manifest": manifest,
        "require_holdout_thresholds": manifest is not None,
    }


def run_stdout_to_file(
    command: list[str],
    path: Path,
    *,
    runner: Runner,
    merge_stderr: bool = False,
) -> None:
    with path.open("w", encoding="utf-8") as output:
        stderr: int | TextIO | None = subprocess.STDOUT if merge_stderr else None
        runner(
            command,
            cwd=ROOT,
            check=True,
            stdout=output,
            stderr=stderr,
            text=True,
        )


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact_checksums(plan: dict[str, Any]) -> dict[str, str]:
    checksums = {
        "json": file_sha256(plan["json_path"]),
        "markdown": file_sha256(plan["markdown_path"]),
        "result_summary": file_sha256(plan["result_summary_path"]),
        "thresholds": file_sha256(plan["threshold_path"]),
    }
    if plan["manifest"] is not None:
        checksums["holdout_manifest"] = file_sha256(plan["holdout_manifest_path"])
        if plan["holdout_source_manifest_path"].is_file():
            checksums["holdout_source_manifest"] = file_sha256(
                plan["holdout_source_manifest_path"]
            )
    return checksums


def load_json_file(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def prepared_source_manifest_path(manifest_path: Path) -> Path | None:
    manifest = load_json_file(manifest_path)
    prepared_from = manifest.get("prepared_from")
    if not isinstance(prepared_from, dict):
        return None
    source = prepared_from.get("manifest")
    if not isinstance(source, str) or not source:
        return None
    source_path = Path(source)
    if source_path.is_absolute():
        return source_path if source_path.is_file() else None
    for candidate in [ROOT / source_path, manifest_path.parent / source_path]:
        if candidate.is_file():
            return candidate.resolve()
    return None


def copy_manifest_artifacts(plan: dict[str, Any]) -> None:
    manifest = plan["manifest"]
    if manifest is None:
        return
    shutil.copyfile(manifest, plan["holdout_manifest_path"])
    source_manifest = prepared_source_manifest_path(manifest)
    if source_manifest is not None:
        shutil.copyfile(source_manifest, plan["holdout_source_manifest_path"])


def write_run_manifest(plan: dict[str, Any]) -> None:
    payload = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "workspace_repo": str(ROOT),
        "output_dir": str(plan["output_dir"]),
        "json": repo_relative(plan["json_path"]),
        "markdown": repo_relative(plan["markdown_path"]),
        "result_summary": repo_relative(plan["result_summary_path"]),
        "thresholds": repo_relative(plan["threshold_path"]),
        "paper_manifest": (
            repo_relative(plan["manifest"]) if plan["manifest"] is not None else None
        ),
        "holdout_manifest": (
            repo_relative(plan["holdout_manifest_path"])
            if plan["manifest"] is not None
            else None
        ),
        "holdout_source_manifest": (
            repo_relative(plan["holdout_source_manifest_path"])
            if plan["holdout_source_manifest_path"].is_file()
            else None
        ),
        "require_holdout_thresholds": plan["require_holdout_thresholds"],
        "sha256": artifact_checksums(plan),
        "commands": {
            "measure": plan["measurement_command"],
            "check_thresholds": plan["threshold_command"],
            "summarize": plan["summary_command"],
            "extract_result_summary": plan["result_summary_command"],
            "verify_artifacts": plan["verify_command"],
        },
    }
    plan["run_manifest_path"].write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def run_plan(
    plan: dict[str, Any],
    *,
    runner: Runner = subprocess.run,
) -> None:
    plan["output_dir"].mkdir(parents=True, exist_ok=True)
    copy_manifest_artifacts(plan)
    run_stdout_to_file(plan["measurement_command"], plan["json_path"], runner=runner)
    run_stdout_to_file(
        plan["threshold_command"],
        plan["threshold_path"],
        runner=runner,
        merge_stderr=True,
    )
    run_stdout_to_file(plan["summary_command"], plan["markdown_path"], runner=runner)
    run_stdout_to_file(
        plan["result_summary_command"],
        plan["result_summary_path"],
        runner=runner,
    )
    write_run_manifest(plan)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--paper",
        action="store_true",
        help="use tools/effect_paper_holdouts.json and require holdout thresholds",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="holdout manifest to pass to tools/measure_effect.py",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help=(
            "directory for effect.json, effect.md, result_summary.json, "
            "thresholds.txt, and run_manifest.json"
        ),
    )
    args = parser.parse_args(argv)
    if args.paper and args.manifest is not None:
        parser.error("--paper cannot be combined with --manifest")
    if args.paper:
        args.manifest = DEFAULT_PAPER_MANIFEST
    return args


def main() -> None:
    args = parse_args()
    plan = build_plan(args.output_dir, args.manifest)
    run_plan(plan)
    print(f"wrote effect artifacts to {plan['output_dir']}")


if __name__ == "__main__":
    main()
