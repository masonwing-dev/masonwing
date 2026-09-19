#!/usr/bin/env python3
"""Run selected local implementation checks and retain actual output and status.

This records application/infrastructure evidence, never product acceptance.
Endpoints used by the opt-in postgres/native checks are fixed loopback fixtures.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CHECKS: dict[str, list[tuple[str, list[str]]]] = {
    "contracts": [
        ("spec-drift", ["python3", "scripts/sync_specs.py", "--check"]),
        ("type-drift", ["node", "scripts/generate-types.mjs", "--check"]),
    ],
    "format": [("rust-format", ["cargo", "fmt", "--all", "--", "--check"])],
    "rust": [("rust-workspace", ["cargo", "test", "--locked", "--workspace", "--all-targets", "--jobs", "2"])],
    "postgres": [("postgres-runtime", ["cargo", "test", "--locked", "-p", "masonwing-data-postgres", "--features", "local-postgres-tests", "--test", "registry_postgres", "--jobs", "2"])],
    "temporal": [("temporal-cancellation-probe", ["cargo", "test", "--locked", "-p", "masonwing-workflow-temporal", "--features", "local-temporal-tests", "--test", "cancellation_temporal", "--jobs", "2"])],
    "native": [("native-local-flow", ["cargo", "run", "--locked", "-p", "masonwing-host-api", "--example", "local_plugin_flow", "--jobs", "2", "--", "--local"])],
    "runflow": [("run-local-flow", ["cargo", "run", "--locked", "-p", "masonwing-host-api", "--example", "local_run_flow", "--jobs", "2", "--", "--local"])],
    "runs": [("run-resume-postgres", ["cargo", "test", "--locked", "-p", "masonwing-worker", "--features", "local-postgres-tests", "--test", "crash_resume_postgres", "--jobs", "2"])],
    "minio": [("minio-streaming", ["python3", "tests/artifacts/run_minio_adapter_smoke.py"])],
    "scaffold": [("scaffold", [".venv/bin/python", "-m", "pytest", "tests/scaffold", "-q"])],
    "auth": [("auth-schema", [".venv/bin/python", "-m", "pytest", "tests/auth", "-q"])],
    "http": [("http-admission", [".venv/bin/python", "-m", "pytest", "tests/integration/test_http.py", "-q"])],
    "runhttp": [("run-commands-http", [".venv/bin/python", "-m", "pytest", "tests/integration/test_run_commands.py", "-q"])],
    "infra": [("local-infrastructure", [".venv/bin/python", "-m", "pytest", "tests/integration", "-q"])],
    "browser": [("browser-e2e", ["pnpm", "exec", "playwright", "test", "--workers=1"])],
    "web": [("web-types", ["pnpm", "typecheck"]), ("web-tests", ["pnpm", "test"]), ("web-build", ["pnpm", "build"])],
    "lint": [("rust-clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--jobs", "2", "--", "-D", "warnings"])],
}


def source_digest() -> str:
    digest = hashlib.sha256()
    files = [ROOT / "Cargo.toml", ROOT / "Cargo.lock"]
    for directory in ("crates", "platform-plugins", "products"):
        for path in (ROOT / directory).rglob("*"):
            if path.is_file() and (path.suffix == ".rs" or path.name == "Cargo.toml"):
                files.append(path)
    for path in sorted(files):
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("steps", nargs="+", choices=tuple(CHECKS))
    args = parser.parse_args()
    started = dt.datetime.now(dt.timezone.utc)
    output = ROOT / ".evidence" / ("runtime-" + started.strftime("%Y%m%d-%H%M%S-%f"))
    output.mkdir(parents=True, exist_ok=False)
    report = {
        "kind": "IMPLEMENTATION_VERIFICATION",
        "product_acceptance": "NOT_ACCEPTED",
        "started_at": started.isoformat(),
        "source_digest_before": source_digest(),
        "results": [],
    }
    selected = [item for step in dict.fromkeys(args.steps) for item in CHECKS[step]]
    for name, command in selected:
        start = time.monotonic()
        log = output / f"{name}.log"
        with log.open("w") as stream:
            try:
                result = subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, check=False)
                exit_code = result.returncode
            except OSError as error:
                stream.write(f"Failed to start {command[0]}: {error.__class__.__name__}\n")
                exit_code = 127
        report["results"].append({"name": name, "command": command, "exit_code": exit_code,
                                  "elapsed_seconds": round(time.monotonic() - start, 3), "log": str(log.relative_to(ROOT))})
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{name}: {'PASS' if exit_code == 0 else 'FAIL'} — {log.relative_to(ROOT)}", flush=True)
    report["source_digest_after"] = source_digest()
    report["source_unchanged_during_checks"] = report["source_digest_before"] == report["source_digest_after"]
    report["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
    (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Evidence: {output.relative_to(ROOT) / 'verification.json'}", flush=True)
    return int(not report["source_unchanged_during_checks"] or any(item["exit_code"] for item in report["results"]))


if __name__ == "__main__":
    raise SystemExit(main())
