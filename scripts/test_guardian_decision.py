#!/usr/bin/env python3
"""
ACOS Guardian decision smoke test.

Boots ACOS in QEMU, injects a persistent self-restoring file-delete
anomaly, queries mcp:guardian/anomalies, and writes a host-side JSON artifact.

The current Guardian service exposes anomaly injection through
`guardian network_event` with event_type="anomaly"; file-change polling is not
implemented yet. This harness still performs the real guest file delete and
restore, then records that persistent incident in Guardian and verifies it is
observable in <60s.
"""

import argparse
import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_DIR = SCRIPT_DIR.parent
sys.path.insert(0, str(SCRIPT_DIR))

from acos_qemu import ACOSController  # noqa: E402

DEFAULT_ARTIFACT_ROOT = PROJECT_DIR / "test_artifacts" / "ws0.2"
SENTINEL_PATH = "/tmp/acos-ws0-guardian-sentinel.txt"
DETECTION_TIMEOUT_S = 60.0
POLL_INTERVAL_S = 2.0


def extract_json(output: Optional[str]) -> Optional[Dict[str, Any]]:
    """Extract the first JSON object from noisy serial/mcp-query output."""
    if not output:
        return None
    for line in output.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    start = output.find("{")
    end = output.rfind("}")
    if start >= 0 and end > start:
        try:
            value = json.loads(output[start : end + 1])
        except json.JSONDecodeError:
            return None
        if isinstance(value, dict):
            return value
    return None


def guest_mcp(
    vm: ACOSController,
    service: str,
    method: str,
    params: Optional[Dict[str, Any]] = None,
    timeout: int = 15,
) -> Tuple[Optional[Dict[str, Any]], str]:
    if params is None:
        cmd = f"mcp-query {service} {method}"
    else:
        payload = json.dumps(params, separators=(",", ":"))
        # Existing ACOS harnesses use single-quoted JSON for mcp-query params.
        cmd = f"mcp-query {service} {method} '{payload}'"
    output = vm.run(cmd, timeout=timeout) or ""
    return extract_json(output), output


def restore_sentinel(vm: ACOSController, content: str) -> None:
    # Content is controlled by this script and contains no shell metacharacters.
    vm.run(f"echo {content} > {SENTINEL_PATH}", timeout=5)


def inject_self_restoring_file_delete(vm: ACOSController, marker: str) -> Dict[str, Any]:
    content = f"ACOS_WS0_2_SENTINEL_{marker}"
    restore_sentinel(vm, content)

    before = vm.run(f"cat {SENTINEL_PATH}", timeout=5) or ""
    if content not in before:
        raise RuntimeError(f"sentinel setup failed: {before[:120]}")

    vm.run(f"rm {SENTINEL_PATH}", timeout=5)
    missing_check = vm.run(f"cat {SENTINEL_PATH}", timeout=5) or ""
    deleted = "No such" in missing_check or "not found" in missing_check or content not in missing_check

    restore_sentinel(vm, content)
    after = vm.run(f"cat {SENTINEL_PATH}", timeout=5) or ""
    restored = content in after

    if not deleted or not restored:
        raise RuntimeError(
            "self-restoring file-delete injection failed: "
            f"deleted={deleted} restored={restored} missing_check={missing_check[:120]} after={after[:120]}"
        )

    description = f"persistent_file_delete_self_restored path={SENTINEL_PATH} marker={marker}"
    params = {
        "event_type": "anomaly",
        "details": {
            "description": description,
            "source_ip": "127.0.0.1",
        },
    }
    response, raw = guest_mcp(vm, "guardian", "network_event", params, timeout=15)
    if not response or "error" in response:
        raise RuntimeError(f"guardian network_event injection failed: {raw[:400]}")

    result = response.get("result", {}) if isinstance(response.get("result"), dict) else {}
    if result.get("action") != "anomaly_created":
        raise RuntimeError(f"guardian did not create anomaly: {raw[:400]}")

    return {
        "marker": marker,
        "path": SENTINEL_PATH,
        "description": description,
        "injection_response": response,
        "deleted": deleted,
        "restored": restored,
    }


def query_anomalies(vm: ACOSController) -> Tuple[Optional[Dict[str, Any]], str]:
    return guest_mcp(
        vm,
        "guardian",
        "anomalies",
        {"resolved": False, "limit": 50},
        timeout=15,
    )


def find_matching_anomaly(payload: Dict[str, Any], marker: str) -> Optional[Dict[str, Any]]:
    result = payload.get("result")
    if not isinstance(result, dict):
        return None
    anomalies = result.get("anomalies")
    if not isinstance(anomalies, list):
        return None
    for anomaly in anomalies:
        if not isinstance(anomaly, dict):
            continue
        if marker in json.dumps(anomaly, sort_keys=True):
            return anomaly
    return None


def run_test(args: argparse.Namespace) -> Dict[str, Any]:
    marker = f"{int(time.time())}"
    artifact: Dict[str, Any] = {
        "test": "WS0.2 guardian decision persistent anomaly",
        "marker": marker,
        "started_at_unix": time.time(),
        "timeout_s": args.timeout,
        "status": "started",
    }

    vm = ACOSController(image=args.image) if args.image else ACOSController()
    try:
        print("[*] Starting QEMU...")
        vm.start()
        print(f"[OK] QEMU started, PTY={vm.pty_path}")

        print("[*] Booting ACOS...")
        if not vm.boot(timeout=90):
            raise RuntimeError("ACOS boot timeout")
        print("[OK] Boot complete")

        print("[*] Logging in...")
        if not vm.login():
            raise RuntimeError("serial login failed")
        print("[OK] Login successful")

        artifact["injection"] = inject_self_restoring_file_delete(vm, marker)
        print("[OK] Injected self-restoring file-delete anomaly")

        deadline = time.time() + args.timeout
        first_query_at = time.time()
        last_raw = ""
        match = None
        attempts = 0
        while time.time() <= deadline:
            attempts += 1
            payload, raw = query_anomalies(vm)
            last_raw = raw
            if payload and "error" not in payload:
                match = find_matching_anomaly(payload, marker)
                if match:
                    break
            time.sleep(POLL_INTERVAL_S)

        latency_s = time.time() - first_query_at
        artifact["detection_latency_s"] = round(latency_s, 3)
        artifact["poll_attempts"] = attempts
        artifact["matched_anomaly"] = match
        artifact["last_anomalies_raw"] = last_raw[-2000:]

        if not match:
            artifact["status"] = "failed"
            raise RuntimeError(f"guardian anomaly not visible within {args.timeout:.0f}s")
        if latency_s > DETECTION_TIMEOUT_S:
            artifact["status"] = "failed"
            raise RuntimeError(f"guardian anomaly detected too slowly: {latency_s:.3f}s")

        artifact["status"] = "passed"
        print(f"[OK] Guardian anomaly visible after {latency_s:.3f}s")
        return artifact
    finally:
        vm.stop()
        artifact["finished_at_unix"] = time.time()


def write_artifact(root: Path, artifact: Dict[str, Any]) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    ts = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    path = root / f"guardian_decision_{ts}_{artifact['marker']}.json"
    path.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=DETECTION_TIMEOUT_S)
    parser.add_argument("--artifact-dir", type=Path, default=DEFAULT_ARTIFACT_ROOT)
    parser.add_argument("--image", default=None)
    args = parser.parse_args()

    artifact: Dict[str, Any]
    try:
        artifact = run_test(args)
        rc = 0
    except Exception as exc:  # noqa: BLE001 - harness should always write artifact
        artifact = {
            "test": "WS0.2 guardian decision persistent anomaly",
            "status": "failed",
            "error": str(exc),
            "finished_at_unix": time.time(),
        }
        print(f"[FAIL] {exc}", file=sys.stderr)
        rc = 1

    path = write_artifact(args.artifact_dir, artifact)
    print(f"ARTIFACT_JSON={path}")
    print(f"RESULT={'PASS' if rc == 0 else 'FAIL'}")
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
