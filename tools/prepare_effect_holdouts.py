#!/usr/bin/env python3
"""Prepare local repositories for fixed-ref effect holdout manifests."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Callable


Runner = Callable[..., subprocess.CompletedProcess[str]]


class ManifestError(ValueError):
    """Raised when a holdout manifest cannot be materialized safely."""


def resolve_user_path(path: Path) -> Path:
    if path.is_absolute():
        return path
    return (Path.cwd() / path).resolve()


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ManifestError(f"cannot read manifest {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise ManifestError(f"invalid manifest JSON in {path}: {error}") from error
    if not isinstance(manifest, dict):
        raise ManifestError("holdout manifest must be a JSON object")
    return manifest


def manifest_entries(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    entries = manifest.get("repo_holdouts")
    if not isinstance(entries, list) or not entries:
        raise ManifestError("manifest must contain a non-empty repo_holdouts array")
    result = []
    for index, entry in enumerate(entries, start=1):
        if not isinstance(entry, dict):
            raise ManifestError(f"repo_holdouts[{index}] must be an object")
        result.append(entry)
    return result


def entry_remote_url(entry: dict[str, Any], index: int) -> str:
    remote_url = entry.get("remote_url", entry.get("url"))
    if not isinstance(remote_url, str) or not remote_url.strip():
        raise ManifestError(
            f"repo_holdouts[{index}] must include a non-empty remote_url"
        )
    return remote_url.strip()


def entry_ref(entry: dict[str, Any], index: int) -> str:
    ref = entry.get("ref")
    if not isinstance(ref, str) or not ref.strip():
        raise ManifestError(f"repo_holdouts[{index}].ref must be a non-empty string")
    return ref.strip()


def safe_repo_dir_name(remote_url: str) -> str:
    name = remote_url.rstrip("/").rsplit("/", 1)[-1]
    if ":" in name and "://" not in remote_url:
        name = name.rsplit(":", 1)[-1]
    if name.endswith(".git"):
        name = name[:-4]
    name = re.sub(r"[^A-Za-z0-9._-]+", "-", name).strip("-._")
    if not name or name in {".", ".."}:
        return "repo"
    return name


def unique_repo_dirs(entries: list[dict[str, Any]], repo_root: Path) -> list[Path]:
    used: set[str] = set()
    paths = []
    for index, entry in enumerate(entries, start=1):
        base = safe_repo_dir_name(entry_remote_url(entry, index))
        name = base
        suffix = 2
        while name in used:
            name = f"{base}-{suffix}"
            suffix += 1
        used.add(name)
        paths.append(repo_root / name)
    return paths


def ensure_existing_repo_origin(
    local_repo: Path,
    remote_url: str,
    *,
    runner: Runner,
) -> None:
    if not local_repo.is_dir():
        raise ManifestError(f"{local_repo} exists but is not a directory")
    inside = runner(
        ["git", "-C", str(local_repo), "rev-parse", "--is-inside-work-tree"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if inside.returncode != 0 or (inside.stdout or "").strip() != "true":
        raise ManifestError(f"{local_repo} exists but is not a git work tree")
    origin = runner(
        ["git", "-C", str(local_repo), "remote", "get-url", "origin"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    origin_url = (origin.stdout or "").strip()
    if origin_url != remote_url:
        raise ManifestError(
            f"{local_repo} origin is {origin_url!r}, expected {remote_url!r}"
        )


def materialize_repo(
    local_repo: Path,
    remote_url: str,
    ref: str,
    *,
    runner: Runner = subprocess.run,
) -> None:
    if local_repo.exists():
        ensure_existing_repo_origin(local_repo, remote_url, runner=runner)
        runner(
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
            check=True,
            text=True,
        )
    else:
        local_repo.parent.mkdir(parents=True, exist_ok=True)
        runner(
            ["git", "clone", "--quiet", remote_url, str(local_repo)],
            check=True,
            text=True,
        )

    runner(
        ["git", "-C", str(local_repo), "rev-parse", "--verify", f"{ref}^{{commit}}"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def local_manifest_payload(
    manifest: dict[str, Any],
    entries: list[dict[str, Any]],
    local_repos: list[Path],
) -> dict[str, Any]:
    payload = dict(manifest)
    local_entries = []
    for index, (entry, local_repo) in enumerate(
        zip(entries, local_repos),
        start=1,
    ):
        local_entry = dict(entry)
        local_entry["repo"] = str(local_repo)
        if "remote_url" not in local_entry:
            local_entry["remote_url"] = entry_remote_url(entry, index)
        local_entries.append(local_entry)
    payload["repo_holdouts"] = local_entries
    return payload


def prepare_holdouts(
    manifest_path: Path,
    repo_root: Path,
    output_manifest: Path,
    *,
    runner: Runner = subprocess.run,
) -> dict[str, Any]:
    manifest_path = resolve_user_path(manifest_path)
    repo_root = resolve_user_path(repo_root)
    output_manifest = resolve_user_path(output_manifest)
    manifest = load_manifest(manifest_path)
    entries = manifest_entries(manifest)
    local_repos = unique_repo_dirs(entries, repo_root)

    for index, (entry, local_repo) in enumerate(zip(entries, local_repos), start=1):
        materialize_repo(
            local_repo,
            entry_remote_url(entry, index),
            entry_ref(entry, index),
            runner=runner,
        )

    payload = local_manifest_payload(manifest, entries, local_repos)
    output_manifest.parent.mkdir(parents=True, exist_ok=True)
    output_manifest.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return {
        "output_manifest": output_manifest,
        "repo_root": repo_root,
        "local_repos": local_repos,
        "manifest": payload,
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "manifest",
        type=Path,
        help="holdout manifest with repo_holdouts entries and remote_url values",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path("target/effect-repos"),
        help="directory where holdout repositories are cloned or refreshed",
    )
    parser.add_argument(
        "--output-manifest",
        type=Path,
        default=Path("target/effect-repos/holdouts.local.json"),
        help="manifest to write with local repository paths",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    try:
        result = prepare_holdouts(
            args.manifest,
            args.repo_root,
            args.output_manifest,
        )
    except ManifestError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    except subprocess.CalledProcessError as error:
        command = " ".join(str(part) for part in error.cmd)
        print(f"error: command failed: {command}", file=sys.stderr)
        raise SystemExit(error.returncode) from error

    print(f"wrote local holdout manifest to {result['output_manifest']}")


if __name__ == "__main__":
    main()
