#!/usr/bin/env python3
"""Record actual scaffold checks with exit codes, timings and persistent log paths."""
from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--integration", action="store_true")
    parser.add_argument("--browser", action="store_true")
    args = parser.parse_args()
    output = ROOT / ".dev/evidence"
    output.mkdir(parents=True, exist_ok=True)
    checks = [
        ("spec-drift", ["python3", "scripts/sync_specs.py", "--check"]),
        ("type-drift", ["node", "scripts/generate-types.mjs", "--check"]),
        ("rust-format", ["cargo", "fmt", "--all", "--", "--check"]),
        ("rust-check", ["cargo", "check", "--locked", "--workspace", "--jobs", "2"]),
        ("rust-clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--jobs", "2", "--", "-D", "warnings"]),
        ("rust-tests", ["cargo", "test", "--locked", "--workspace", "--jobs", "2"]),
        ("scaffold-tests", [".venv/bin/python", "-m", "pytest", "tests/scaffold", "-q", "--junitxml=.dev/evidence/scaffold.xml"]),
        ("web-types", ["pnpm", "typecheck"]),
        ("web-tests", ["pnpm", "test"]),
        ("web-build", ["pnpm", "build"]),
        ("integration-tests", [".venv/bin/python", "-m", "pytest", "tests/integration", "-q", "--junitxml=.dev/evidence/integration.xml"]),
    ]
    if args.browser:
        checks.append(("browser-tests", ["pnpm", "test:e2e"]))
    if args.browser:
        checks.append(("browser-tests", ["pnpm", "test:e2e"]))
    report = {"kind": "SCAFFOLD_VERIFICATION", "product_acceptance": "NOT_ACCEPTED",
              "started_at": dt.datetime.now(dt.timezone.utc).isoformat(), "results": []}
    for name, command in checks:
        start = time.monotonic()
        log = output / f"{name}.log"
        with log.open("w") as stream:
            result = subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, check=False)
        item = {"name": name, "command": command, "exit_code": result.returncode,
                "elapsed_seconds": round(time.monotonic() - start, 3), "log": str(log.relative_to(ROOT))}
        report["results"].append(item)
        (output / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{name}: {'PASS' if result.returncode == 0 else 'FAIL'} ({item['elapsed_seconds']}s) — {item['log']}", flush=True)
    return int(any(item["exit_code"] for item in report["results"]))


if __name__ == "__main__":
    raise SystemExit(main())
