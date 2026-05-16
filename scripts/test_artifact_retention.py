#!/usr/bin/env python3
"""Unit tests for ACOS ArtifactManager and retention policy."""

import json
import os
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from acos_qemu import ArtifactManager
from artifact_retention import prune_artifacts


def test_artifact_manager_writes_ws_timestamp_json(tmp_path):
    manager = ArtifactManager("ws0.1", root=tmp_path, timestamp="20260516T170000Z")

    path = Path(manager.write_json({"ok": True, "tests": 3}))

    assert path == tmp_path / "ws0.1" / "20260516T170000Z.json"
    payload = json.loads(path.read_text(encoding="utf-8"))
    assert payload["ws"] == "ws0.1"
    assert payload["timestamp"] == "20260516T170000Z"
    assert payload["ok"] is True


def test_artifact_manager_rejects_path_traversal(tmp_path):
    manager = ArtifactManager("ws0.1", root=tmp_path, timestamp="run")

    try:
        manager.asset_path("../escape.ppm")
    except ValueError as exc:
        assert "path traversal" in str(exc)
    else:
        raise AssertionError("path traversal was accepted")


def test_ppm_to_png_conversion(tmp_path):
    manager = ArtifactManager("ws0.1", root=tmp_path, timestamp="run")
    ppm = manager.asset_path("screen.ppm")
    ppm.write_text("P3\n1 1\n255\n255 0 0\n", encoding="ascii")

    png = Path(manager.convert_ppm_to_png(ppm))

    assert png.exists()
    assert png.suffix == ".png"
    assert png.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")


def test_prune_artifacts_deletes_files_older_than_7_days(tmp_path):
    old = tmp_path / "ws0.1" / "old.json"
    fresh = tmp_path / "ws0.1" / "fresh.json"
    old.parent.mkdir(parents=True)
    old.write_text("old", encoding="utf-8")
    fresh.write_text("fresh", encoding="utf-8")
    now = 2_000_000.0
    os.utime(old, (now - 8 * 24 * 60 * 60, now - 8 * 24 * 60 * 60))
    os.utime(fresh, (now, now))

    result = prune_artifacts(tmp_path, max_age_days=7, max_total_bytes=1024, now=now)

    assert old in result.deleted_files
    assert not old.exists()
    assert fresh.exists()


def test_prune_artifacts_enforces_total_size_by_oldest_first(tmp_path):
    paths = []
    now = 2_000_000.0
    for idx in range(3):
        path = tmp_path / "ws0.1" / f"artifact-{idx}.bin"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(bytes([idx]) * 10)
        os.utime(path, (now + idx, now + idx))
        paths.append(path)

    result = prune_artifacts(tmp_path, max_age_days=7, max_total_bytes=20, now=now)

    assert paths[0] in result.deleted_files
    assert not paths[0].exists()
    assert paths[1].exists()
    assert paths[2].exists()
    assert result.remaining_bytes == 20
