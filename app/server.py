#!/usr/bin/env python3
"""DupeLens local web UI server.

Runs only on 127.0.0.1 and shells out to core/target/release/dupelens-core.
"""
from __future__ import annotations

import json
import subprocess
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "app"
CORE = ROOT / "core" / "target" / "release" / "dupelens-core"
HOST = "127.0.0.1"
PORT = 8787

jobs: dict[str, dict] = {}
jobs_lock = threading.Lock()


def json_bytes(value: object, status: int = 200) -> tuple[int, bytes, str]:
    return status, (json.dumps(value, indent=2) + "\n").encode("utf-8"), "application/json"


def read_body(handler: BaseHTTPRequestHandler) -> dict:
    length = int(handler.headers.get("Content-Length", "0") or "0")
    if length <= 0:
        return {}
    return json.loads(handler.rfile.read(length).decode("utf-8"))


def safe_path(value: str) -> str:
    value = (value or "").strip()
    if not value:
        raise ValueError("path is required")
    return str(Path(value).expanduser())


def start_scan(payload: dict) -> str:
    if not CORE.exists():
        raise FileNotFoundError(f"Rust core binary not found: {CORE}. Run: cd core && cargo build --release")
    mode = payload.get("mode", "single")
    if mode not in {"single", "compare"}:
        raise ValueError("mode must be single or compare")
    primary = safe_path(payload.get("primary", ""))
    backup = safe_path(payload.get("backup", "")) if mode == "compare" else ""
    if mode == "compare" and Path(primary).resolve(strict=False) == Path(backup).resolve(strict=False):
        raise ValueError("Primary and backup folders are the same. Use Scan one folder instead, or choose a different backup/import folder.")
    scan_mode = payload.get("scan_mode", "balanced")
    if scan_mode not in {"quick", "balanced", "deep"}:
        raise ValueError("scan_mode must be quick, balanced, or deep")
    recursive = bool(payload.get("recursive", True))
    include_hidden = bool(payload.get("include_hidden", False))

    job_id = uuid.uuid4().hex
    job = {
        "id": job_id,
        "status": "running",
        "started_at": time.time(),
        "events": [],
        "result": None,
        "error": None,
        "command": [],
    }
    args = [str(CORE), "--mode", scan_mode, "--ui-json", "--json-events"]
    if mode == "compare":
        args.append("--compare")
    if recursive:
        args.append("-r")
    if include_hidden:
        args.append("--include-hidden")
    args.append(primary)
    if mode == "compare":
        args.append(backup)
    job["command"] = args
    with jobs_lock:
        jobs[job_id] = job
    threading.Thread(target=run_scan, args=(job_id, args), daemon=True).start()
    return job_id


def run_scan(job_id: str, args: list[str]) -> None:
    try:
        proc = subprocess.Popen(
            args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            cwd=str(ROOT),
        )
        stderr_lines: list[str] = []

        def read_events() -> None:
            assert proc.stderr is not None
            for line in proc.stderr:
                line = line.strip()
                if not line:
                    continue
                stderr_lines.append(line)
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    event = {"event": "log", "message": line}
                with jobs_lock:
                    jobs[job_id]["events"].append(event)

        reader = threading.Thread(target=read_events, daemon=True)
        reader.start()
        stdout, _ = proc.communicate()
        reader.join(timeout=1)
        if proc.returncode != 0:
            with jobs_lock:
                jobs[job_id]["status"] = "failed"
                jobs[job_id]["error"] = "\n".join(stderr_lines[-20:]) or f"core exited with {proc.returncode}"
            return
        result = json.loads(stdout)
        with jobs_lock:
            jobs[job_id]["status"] = "completed"
            jobs[job_id]["result"] = result
            jobs[job_id]["finished_at"] = time.time()
    except Exception as exc:  # noqa: BLE001 - local server should surface errors
        with jobs_lock:
            jobs[job_id]["status"] = "failed"
            jobs[job_id]["error"] = str(exc)


class Handler(BaseHTTPRequestHandler):
    server_version = "DupeLensLocal/0.1"

    def log_message(self, format: str, *args) -> None:  # noqa: A002 - stdlib API name
        # Keep the Mac Terminal quiet during scans. The browser polls /api/jobs
        # every few hundred milliseconds, and the default stdlib handler prints
        # one line per request, which looks like the scan is doing something
        # wrong even when it is working normally.
        return

    def send_payload(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/api/health":
            status, body, ctype = json_bytes({"ok": True, "core_exists": CORE.exists(), "core": str(CORE)})
            self.send_payload(status, body, ctype)
            return
        if path.startswith("/api/jobs/"):
            job_id = path.rsplit("/", 1)[-1]
            with jobs_lock:
                job = jobs.get(job_id)
                snapshot = json.loads(json.dumps(job)) if job else None
            if not snapshot:
                status, body, ctype = json_bytes({"error": "job not found"}, 404)
            else:
                status, body, ctype = json_bytes(snapshot)
            self.send_payload(status, body, ctype)
            return
        if path in {"", "/"}:
            file_path = APP / "index.html"
        else:
            rel = path.lstrip("/")
            if ".." in Path(rel).parts:
                self.send_payload(403, b"forbidden", "text/plain")
                return
            file_path = APP / rel
        if not file_path.exists() or not file_path.is_file():
            self.send_payload(404, b"not found", "text/plain")
            return
        ctype = "text/html; charset=utf-8"
        if file_path.suffix == ".css":
            ctype = "text/css; charset=utf-8"
        elif file_path.suffix == ".js":
            ctype = "text/javascript; charset=utf-8"
        self.send_payload(200, file_path.read_bytes(), ctype)

    def do_POST(self) -> None:  # noqa: N802
        try:
            if self.path == "/api/scan":
                job_id = start_scan(read_body(self))
                status, body, ctype = json_bytes({"job_id": job_id}, 202)
                self.send_payload(status, body, ctype)
                return
            status, body, ctype = json_bytes({"error": "not found"}, 404)
            self.send_payload(status, body, ctype)
        except Exception as exc:  # noqa: BLE001
            status, body, ctype = json_bytes({"error": str(exc)}, 400)
            self.send_payload(status, body, ctype)


def main() -> int:
    print("DupeLens local UI")
    print(f"Open: http://{HOST}:{PORT}")
    print("Keep this Terminal window open while using the UI. Press Ctrl+C here to stop the local server.")
    if not CORE.exists():
        print(f"Core binary missing: {CORE}")
        print("Build it first: cd core && cargo build --release")
    httpd = ThreadingHTTPServer((HOST, PORT), Handler)
    httpd.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
