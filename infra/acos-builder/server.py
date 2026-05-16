#!/usr/bin/env python3
"""acos-builder — HTTP build orchestrator for AH on acos-hermes-01.

Listens on 127.0.0.1:8772. Async job model: POST returns a job_id
immediately, build runs in a child process, status + tailed log
available via GET endpoints.

User: hermes. Operates on /home/hermes/acos/ (the hermes-side cookbook
clone). For canonical build semantics, see docs/BUILDING_ACOS_ON_VPS.md
in the ACOS repo.

Endpoints:
  GET  /health                      liveness + tool inventory
  POST /build_image                 make all CONFIG_NAME=acos-bare CI=1
  POST /build_recipe                make {r,cr,ucr}.<recipe> CI=1
  POST /sync                        git pull + setup-acos.sh
  GET  /jobs                        list jobs
  GET  /jobs/<id>                   job summary
  GET  /jobs/<id>/log               tail of build log (?bytes=N)
  POST /jobs/<id>/cancel            kill running job

All POST bodies are JSON. All responses are JSON.
"""
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import uuid
import shutil
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

ACOS_ROOT = Path("/home/hermes/acos")
BASE_DIR = ACOS_ROOT / "base"
LOG_DIR = Path("/var/lib/acos-builder/logs")
JOB_DIR = Path("/var/lib/acos-builder/jobs")
LISTEN_HOST = "127.0.0.1"
LISTEN_PORT = 8772

# Allowed recipe name pattern (no shell metachars, no path traversal).
RECIPE_RE = re.compile(r"^[a-zA-Z0-9_][a-zA-Z0-9_+-]{0,63}$")

# Allowed make targets for /build_recipe.
ALLOWED_MAKE_PREFIX = {"r", "cr", "ucr"}

# Limit concurrent builds — full image builds are heavy.
MAX_CONCURRENT_BUILDS = 1

# Default timeout for /build_image — full builds can take 30-60 min.
DEFAULT_IMAGE_BUILD_TIMEOUT = 5400  # 90 min
DEFAULT_RECIPE_BUILD_TIMEOUT = 1800  # 30 min
DEFAULT_SYNC_TIMEOUT = 300  # 5 min

# ---------------------------------------------------------------------------
# Job storage
# ---------------------------------------------------------------------------

_jobs_lock = threading.Lock()
_jobs = {}  # job_id -> Job


class Job:
    def __init__(self, kind, args, timeout_s):
        self.id = uuid.uuid4().hex
        self.kind = kind
        self.args = args
        self.timeout_s = timeout_s
        self.state = "pending"  # pending -> running -> {completed, failed, cancelled, timeout}
        self.pid = None
        self.exit_code = None
        self.started_at = None
        self.finished_at = None
        self.log_path = LOG_DIR / f"{self.id}.log"
        self.meta_path = JOB_DIR / f"{self.id}.json"
        self._proc = None
        self._lock = threading.Lock()

    def to_dict(self):
        with self._lock:
            duration = None
            if self.started_at is not None:
                end = self.finished_at if self.finished_at else time.time()
                duration = end - self.started_at
            return {
                "id": self.id,
                "kind": self.kind,
                "args": self.args,
                "state": self.state,
                "pid": self.pid,
                "exit_code": self.exit_code,
                "started_at": self.started_at,
                "finished_at": self.finished_at,
                "duration_s": duration,
                "log_path": str(self.log_path),
                "log_size": (self.log_path.stat().st_size
                             if self.log_path.exists() else 0),
            }

    def persist(self):
        try:
            self.meta_path.write_text(json.dumps(self.to_dict(), indent=2))
        except OSError:
            pass


def _build_argv(job):
    if job.kind == "build_image":
        return ["make", "all", "CONFIG_NAME=acos-bare", "CI=1"], str(BASE_DIR)
    if job.kind == "build_recipe":
        prefix = job.args.get("clean", "r")
        if prefix not in ALLOWED_MAKE_PREFIX:
            raise ValueError(f"clean must be one of {sorted(ALLOWED_MAKE_PREFIX)}")
        recipe = job.args["recipe"]
        return ["make", f"{prefix}.{recipe}", "CI=1"], str(BASE_DIR)
    if job.kind == "sync":
        # We use a small bash script so we can chain git pull + setup-acos.sh.
        ref = job.args.get("ref", "main")
        script = (
            f"cd {ACOS_ROOT} && "
            f"git fetch origin {ref} && "
            f"git checkout {ref} && "
            f"git pull --ff-only origin {ref} && "
            f"./scripts/setup-acos.sh"
        )
        return ["bash", "-lc", script], str(ACOS_ROOT)
    raise ValueError(f"unknown job kind: {job.kind}")


def _run_job(job):
    """Worker that runs a job to completion. Called in a thread."""
    try:
        argv, cwd = _build_argv(job)
    except (ValueError, KeyError) as e:
        with job._lock:
            job.state = "failed"
            job.exit_code = -3
            job.finished_at = time.time()
        try:
            job.log_path.write_text(f"argv-build error: {e}\n")
        except OSError:
            pass
        job.persist()
        return

    log = job.log_path.open("wb")
    log.write(f"argv: {argv}\ncwd: {cwd}\n\n".encode())
    log.flush()

    with job._lock:
        job.state = "running"
        job.started_at = time.time()
    job.persist()

    try:
        proc = subprocess.Popen(
            argv,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,  # so we can SIGTERM the whole group on cancel
        )
    except (OSError, ValueError) as e:
        with job._lock:
            job.state = "failed"
            job.exit_code = -2
            job.finished_at = time.time()
        log.write(f"\nspawn failed: {type(e).__name__}: {e}\n".encode())
        log.close()
        job.persist()
        return

    with job._lock:
        job.pid = proc.pid
        job._proc = proc

    try:
        rc = proc.wait(timeout=job.timeout_s)
        with job._lock:
            job.exit_code = rc
            job.state = "completed" if rc == 0 else "failed"
    except subprocess.TimeoutExpired:
        # Kill the whole process group.
        try:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        except (OSError, ProcessLookupError):
            pass
        with job._lock:
            job.exit_code = -4
            job.state = "timeout"
    finally:
        with job._lock:
            job.finished_at = time.time()
            job._proc = None
        try:
            log.close()
        except OSError:
            pass
        job.persist()


def _count_active_jobs():
    with _jobs_lock:
        return sum(1 for j in _jobs.values() if j.state in ("pending", "running"))


def _start_job(kind, args, timeout_s):
    """Create a job, launch a worker thread, return the Job."""
    if kind in ("build_image", "build_recipe") and _count_active_jobs() >= MAX_CONCURRENT_BUILDS:
        return None, "another build is already running"
    job = Job(kind, args, timeout_s)
    with _jobs_lock:
        _jobs[job.id] = job
    t = threading.Thread(target=_run_job, args=(job,), daemon=True, name=f"job-{job.id[:8]}")
    t.start()
    return job, None


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

def _validate_build_image_args(args):
    timeout = int(args.get("timeout_s", DEFAULT_IMAGE_BUILD_TIMEOUT))
    if not 60 <= timeout <= 7200:
        return None, "timeout_s must be 60..7200 (max 2h)"
    return {"timeout_s": timeout}, None


def _validate_build_recipe_args(args):
    recipe = (args.get("recipe") or "").strip()
    if not RECIPE_RE.match(recipe):
        return None, f"recipe must match {RECIPE_RE.pattern}"
    clean = (args.get("clean") or "r").strip()
    if clean not in ALLOWED_MAKE_PREFIX:
        return None, f"clean must be one of {sorted(ALLOWED_MAKE_PREFIX)}"
    timeout = int(args.get("timeout_s", DEFAULT_RECIPE_BUILD_TIMEOUT))
    if not 30 <= timeout <= 3600:
        return None, "timeout_s must be 30..3600 (max 1h)"
    return {"recipe": recipe, "clean": clean, "timeout_s": timeout}, None


def _validate_sync_args(args):
    ref = (args.get("ref") or "main").strip()
    # branch / tag / commit-ish, conservative.
    if not re.match(r"^[a-zA-Z0-9_/.-]{1,100}$", ref):
        return None, "ref must match ^[a-zA-Z0-9_/.-]{1,100}$"
    timeout = int(args.get("timeout_s", DEFAULT_SYNC_TIMEOUT))
    if not 30 <= timeout <= 1800:
        return None, "timeout_s must be 30..1800"
    return {"ref": ref, "timeout_s": timeout}, None


VALIDATORS = {
    "build_image": _validate_build_image_args,
    "build_recipe": _validate_build_recipe_args,
    "sync": _validate_sync_args,
}


# ---------------------------------------------------------------------------
# HTTP handlers
# ---------------------------------------------------------------------------

class Handler(BaseHTTPRequestHandler):
    server_version = "acos-builder/1"

    def _send_json(self, status, payload):
        body = json.dumps(payload, default=str).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _read_json(self):
        try:
            length = int(self.headers.get("Content-Length", 0) or 0)
            raw = self.rfile.read(length) if length > 0 else b"{}"
            return (json.loads(raw) if raw else {}), None
        except (ValueError, json.JSONDecodeError) as e:
            return None, f"bad json: {e}"

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        if u.path == "/health":
            self._send_json(200, {
                "status": "ok",
                "service": "acos-builder",
                "endpoint": f"http://{LISTEN_HOST}:{LISTEN_PORT}",
                "acos_root": str(ACOS_ROOT),
                "base_dir": str(BASE_DIR),
                "tools": {
                    "make": shutil.which("make") or "(missing)",
                    "git": shutil.which("git") or "(missing)",
                    "podman": shutil.which("podman") or "(missing)",
                    "bash": shutil.which("bash") or "(missing)",
                },
                "active_jobs": _count_active_jobs(),
                "max_concurrent_builds": MAX_CONCURRENT_BUILDS,
            })
            return

        if u.path == "/jobs":
            with _jobs_lock:
                out = [j.to_dict() for j in _jobs.values()]
            self._send_json(200, {"jobs": out, "count": len(out)})
            return

        m = re.match(r"^/jobs/([a-f0-9]{32})$", u.path)
        if m:
            job_id = m.group(1)
            with _jobs_lock:
                job = _jobs.get(job_id)
            if job is None:
                self._send_json(404, {"error": "job not found", "id": job_id})
                return
            self._send_json(200, job.to_dict())
            return

        m = re.match(r"^/jobs/([a-f0-9]{32})/log$", u.path)
        if m:
            job_id = m.group(1)
            with _jobs_lock:
                job = _jobs.get(job_id)
            if job is None:
                self._send_json(404, {"error": "job not found", "id": job_id})
                return
            qs = urllib.parse.parse_qs(u.query)
            try:
                want_bytes = int(qs.get("bytes", ["32000"])[0])
            except ValueError:
                want_bytes = 32000
            want_bytes = max(1024, min(want_bytes, 1_000_000))
            log_text = ""
            if job.log_path.exists():
                size = job.log_path.stat().st_size
                with job.log_path.open("rb") as f:
                    if size > want_bytes:
                        f.seek(size - want_bytes)
                    log_text = f.read().decode("utf-8", errors="replace")
            self._send_json(200, {
                "id": job_id,
                "state": job.state,
                "log_tail": log_text,
                "log_size": (job.log_path.stat().st_size
                             if job.log_path.exists() else 0),
            })
            return

        self._send_json(404, {"error": "not found", "path": u.path})

    def do_POST(self):
        u = urllib.parse.urlparse(self.path)

        # Job creation endpoints.
        if u.path in ("/build_image", "/build_recipe", "/sync"):
            args, err = self._read_json()
            if err:
                self._send_json(400, {"error": err})
                return
            kind = u.path.lstrip("/")
            validator = VALIDATORS[kind]
            normalized, verr = validator(args)
            if verr:
                self._send_json(400, {"error": verr})
                return
            job, err = _start_job(kind, normalized, normalized["timeout_s"])
            if err:
                self._send_json(409, {"error": err})
                return
            self._send_json(202, {"id": job.id, "kind": kind, "state": "pending"})
            return

        m = re.match(r"^/jobs/([a-f0-9]{32})/cancel$", u.path)
        if m:
            job_id = m.group(1)
            with _jobs_lock:
                job = _jobs.get(job_id)
            if job is None:
                self._send_json(404, {"error": "job not found", "id": job_id})
                return
            with job._lock:
                proc = job._proc
                pid = job.pid
            if proc is None or proc.poll() is not None:
                self._send_json(409, {"error": "job is not running", "state": job.state})
                return
            try:
                os.killpg(pid, signal.SIGTERM)
            except (OSError, ProcessLookupError) as e:
                self._send_json(500, {"error": f"kill failed: {e}"})
                return
            with job._lock:
                job.state = "cancelled"
            self._send_json(202, {"id": job_id, "state": "cancelled"})
            return

        self._send_json(404, {"error": "not found", "path": u.path})

    def log_message(self, format, *args):
        # Silence default access log; journald gets stdout/stderr.
        return


# ---------------------------------------------------------------------------
# Bootstrap
# ---------------------------------------------------------------------------

def _setup_paths():
    for p in (LOG_DIR, JOB_DIR):
        try:
            p.mkdir(parents=True, exist_ok=True)
        except OSError as e:
            print(f"fatal: cannot create {p}: {e}", file=sys.stderr)
            sys.exit(2)


def main():
    _setup_paths()
    print(f"acos-builder listening on {LISTEN_HOST}:{LISTEN_PORT}", flush=True)
    print(f"  acos_root  = {ACOS_ROOT}", flush=True)
    print(f"  base_dir   = {BASE_DIR}", flush=True)
    print(f"  log_dir    = {LOG_DIR}", flush=True)
    print(f"  job_dir    = {JOB_DIR}", flush=True)
    httpd = ThreadingHTTPServer((LISTEN_HOST, LISTEN_PORT), Handler)
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nshutting down", flush=True)


if __name__ == "__main__":
    main()
