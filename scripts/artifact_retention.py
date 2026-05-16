#!/usr/bin/env python3
"""Retention helpers for ACOS QEMU test artifacts.

The policy is intentionally simple and deterministic:
- delete files older than ``max_age_days``;
- then, if the remaining tree is larger than ``max_total_bytes``, delete the
  oldest files until it fits;
- remove empty directories left behind.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

DEFAULT_MAX_AGE_DAYS = 7
DEFAULT_MAX_TOTAL_BYTES = 5 * 1024 * 1024 * 1024


@dataclass(frozen=True)
class RetentionResult:
    deleted_files: tuple[Path, ...]
    deleted_bytes: int
    remaining_bytes: int


def iter_artifact_files(root: Path) -> Iterable[Path]:
    """Yield regular files below ``root`` without following symlinks."""
    if not root.exists():
        return
    for path in root.rglob("*"):
        if path.is_file() and not path.is_symlink():
            yield path


def _tree_size(files: Iterable[Path]) -> int:
    total = 0
    for path in files:
        try:
            total += path.stat().st_size
        except FileNotFoundError:
            continue
    return total


def _remove_empty_dirs(root: Path) -> None:
    if not root.exists():
        return
    for path in sorted((p for p in root.rglob("*") if p.is_dir()), reverse=True):
        try:
            path.rmdir()
        except OSError:
            pass


def prune_artifacts(
    root: str | Path,
    *,
    max_age_days: int = DEFAULT_MAX_AGE_DAYS,
    max_total_bytes: int = DEFAULT_MAX_TOTAL_BYTES,
    now: float | None = None,
) -> RetentionResult:
    """Apply age and size retention to an artifact tree.

    Args:
        root: Artifact root directory.
        max_age_days: Files older than this many days are removed first.
        max_total_bytes: Remaining total byte cap; oldest files are removed
            until the cap is satisfied.
        now: Optional UNIX timestamp for deterministic tests.
    """
    root_path = Path(root)
    root_path.mkdir(parents=True, exist_ok=True)
    clock = time.time() if now is None else now
    cutoff = clock - (max_age_days * 24 * 60 * 60)

    deleted: list[Path] = []
    deleted_bytes = 0

    def delete(path: Path) -> None:
        nonlocal deleted_bytes
        try:
            size = path.stat().st_size
            path.unlink()
        except FileNotFoundError:
            return
        deleted.append(path)
        deleted_bytes += size

    for path in list(iter_artifact_files(root_path)):
        try:
            if path.stat().st_mtime < cutoff:
                delete(path)
        except FileNotFoundError:
            continue

    remaining = list(iter_artifact_files(root_path))
    total = _tree_size(remaining)
    if total > max_total_bytes:
        oldest_first = sorted(
            remaining,
            key=lambda p: (p.stat().st_mtime, str(p)),
        )
        for path in oldest_first:
            if total <= max_total_bytes:
                break
            try:
                size = path.stat().st_size
            except FileNotFoundError:
                continue
            delete(path)
            total -= size

    _remove_empty_dirs(root_path)
    remaining_bytes = _tree_size(iter_artifact_files(root_path))
    return RetentionResult(
        deleted_files=tuple(deleted),
        deleted_bytes=deleted_bytes,
        remaining_bytes=remaining_bytes,
    )
