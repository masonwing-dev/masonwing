#!/usr/bin/env python3
"""Local-only, synthetic provider with durable operation keys and observable faults.

This is a test dependency, never a real CMS/model implementation or live evidence.
SQLite persists counters and operation markers across container restarts. Each case
owns its own namespace so concurrent suites cannot reset each other's evidence.
"""
from __future__ import annotations

import hashlib
import json
import re
import socket
import sqlite3
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

DATABASE = Path("/data/provider-fixtures.sqlite3")
CASE_ROUTE = re.compile(r"^/cases/([A-Za-z0-9_-]{1,100})/(effects|counters|lookup)(?:/([A-Za-z0-9_-]{1,100}))?$")
MODES = {"accept", "accept_then_disconnect", "reject", "rate_limit", "unknown_cost"}


def connect():
    db = sqlite3.connect(DATABASE, timeout=5)
    db.row_factory = sqlite3.Row
    return db


def initialize():
    DATABASE.parent.mkdir(parents=True, exist_ok=True)
    with closing(connect()) as db, db:
        db.executescript("""
        PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS counters (
          case_id TEXT PRIMARY KEY,
          accepted_requests INTEGER NOT NULL DEFAULT 0,
          transmitted_requests INTEGER NOT NULL DEFAULT 0,
          mutation_count INTEGER NOT NULL DEFAULT 0,
          lookup_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS effects (
          case_id TEXT NOT NULL,
          operation_key TEXT NOT NULL,
          fingerprint TEXT NOT NULL,
          receipt TEXT NOT NULL,
          PRIMARY KEY (case_id, operation_key)
        );
        """)


class Handler(BaseHTTPRequestHandler):
    server_version = "MasonwingSyntheticProvider/0.1"

    def setup(self):
        super().setup()
        self.connection.settimeout(5)

    def log_message(self, format, *args):
        # Never emit request bodies, credentials, headers or full path query values.
        print(json.dumps({"service": "provider-fixtures", "method": self.command}), flush=True)

    def send_json(self, status: int, value: object, headers: dict | None = None):
        body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            return self.send_json(200, {"status": "ok", "mode": "MOCK", "external_mutations": False})
        match = CASE_ROUTE.fullmatch(self.path)
        if not match:
            return self.send_json(404, {"code": "NOT_FOUND"})
        case, action, key = match.groups()
        with closing(connect()) as db, db:
            db.execute("INSERT OR IGNORE INTO counters(case_id) VALUES (?)", (case,))
            if action == "counters" and key is None:
                row = db.execute("SELECT * FROM counters WHERE case_id=?", (case,)).fetchone()
                return self.send_json(200, {**dict(row), "provider_mode": "MOCK"})
            if action == "lookup" and key:
                db.execute("UPDATE counters SET lookup_count=lookup_count+1 WHERE case_id=?", (case,))
                row = db.execute("SELECT receipt FROM effects WHERE case_id=? AND operation_key=?", (case, key)).fetchone()
                result = (200, json.loads(row["receipt"])) if row else (404, {"code": "REMOTE_NOT_FOUND"})
            else:
                result = (404, {"code": "NOT_FOUND"})
        self.send_json(*result)

    def do_POST(self):
        match = CASE_ROUTE.fullmatch(self.path)
        if not match or match.group(2) != "effects" or match.group(3):
            return self.send_json(404, {"code": "NOT_FOUND"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self.send_json(400, {"code": "INVALID_LENGTH"})
        if not 1 <= length <= 65536 or self.headers.get("Transfer-Encoding"):
            self.close_connection = True
            return self.send_json(413, {"code": "FIXTURE_BODY_LIMIT"})
        try:
            body = json.loads(self.rfile.read(length))
        except (ValueError, TimeoutError):
            return self.send_json(400, {"code": "INVALID_JSON"})
        if not isinstance(body, dict) or set(body) != {"key", "mode", "content_digest"}:
            return self.send_json(400, {"code": "INVALID_FIXTURE"})
        key, mode, digest = body["key"], body["mode"], body["content_digest"]
        if not all(isinstance(v, str) for v in (key, mode, digest)) \
            or not re.fullmatch(r"[A-Za-z0-9_-]{1,100}", key) \
            or mode not in MODES or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
            return self.send_json(400, {"code": "INVALID_FIXTURE"})
        case = match.group(1)
        fingerprint = hashlib.sha256(digest.encode()).hexdigest()
        disconnect = False
        with closing(connect()) as db, db:
            db.execute("BEGIN IMMEDIATE")
            db.execute("INSERT OR IGNORE INTO counters(case_id) VALUES (?)", (case,))
            db.execute("UPDATE counters SET transmitted_requests=transmitted_requests+1 WHERE case_id=?", (case,))
            old = db.execute("SELECT fingerprint,receipt FROM effects WHERE case_id=? AND operation_key=?", (case, key)).fetchone()
            if old:
                result = (200, json.loads(old["receipt"])) if old["fingerprint"] == fingerprint else (409, {"code": "IDEMPOTENCY_CONFLICT"})
                if result[0] == 200:
                    db.execute("UPDATE counters SET accepted_requests=accepted_requests+1 WHERE case_id=?", (case,))
            elif mode == "reject":
                result = (422, {"code": "PROVIDER_REJECTED", "effect_state": "NOT_APPLIED"})
            elif mode == "rate_limit":
                result = (429, {"code": "RATE_LIMITED", "effect_state": "NOT_APPLIED"})
            else:
                receipt = {"provider_mode": "MOCK", "operation_key": key,
                    "remote_id": hashlib.sha256(f"{case}:{key}".encode()).hexdigest()[:32],
                    "content_digest": digest, "effect_state": "APPLIED",
                    "usage_microunits": None if mode == "unknown_cost" else 12}
                db.execute("INSERT INTO effects VALUES (?,?,?,?)", (case, key, fingerprint, json.dumps(receipt)))
                db.execute("UPDATE counters SET accepted_requests=accepted_requests+1, mutation_count=mutation_count+1 WHERE case_id=?", (case,))
                result = (201, receipt)
                disconnect = mode == "accept_then_disconnect"
        # Commit before disconnect reproduces the ambiguous provider outcome boundary.
        if disconnect:
            self.close_connection = True
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()
            return
        self.send_json(*result, headers={"Retry-After": "60"} if result[0] == 429 else None)


if __name__ == "__main__":
    initialize()
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
