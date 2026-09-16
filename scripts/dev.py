#!/usr/bin/env python3
"""Operate only the masonwing-dev Compose project; never delete persistent volumes."""
from __future__ import annotations

import argparse
import json
import shutil
import socket
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROJECT = "masonwing-dev"


def compose(*args: str, check: bool = True, capture: bool = False):
    return subprocess.run(["docker", "compose", "--project-name", PROJECT, *args], cwd=ROOT,
                          check=check, text=True, capture_output=capture)


def bootstrap_env():
    path = ROOT / ".env"
    if not path.exists():
        shutil.copyfile(ROOT / ".env.example", path)
        path.chmod(0o600)
    (ROOT / ".dev/evidence").mkdir(parents=True, exist_ok=True)


def preflight():
    bootstrap_env()
    config = json.loads(compose("config", "--format", "json", capture=True).stdout)
    published = []
    for service in config["services"].values():
        for mapping in service.get("ports", []):
            port = int(mapping["published"])
            if mapping.get("host_ip") != "127.0.0.1" or not 39850 <= port <= 39859:
                raise SystemExit(f"Unsafe local port mapping: {mapping}")
            published.append(port)
    if len(published) != len(set(published)):
        raise SystemExit("Duplicate host ports within Masonwing Compose")
    running = compose("ps", "--format", "json", capture=True).stdout.strip()
    own_ports = set()
    if running:
        records = json.loads(running) if running.startswith("[") else [json.loads(x) for x in running.splitlines()]
        for record in records:
            own_ports.update(p["PublishedPort"] for p in (record.get("Publishers") or []))
    conflicts = []
    for port in published:
        if port in own_ports:
            continue
        with socket.socket() as listener:
            try:
                listener.bind(("127.0.0.1", port))
            except OSError:
                conflicts.append(port)
    if conflicts:
        raise SystemExit(f"Ports owned by another process: {conflicts}. No services were changed.")
    print(f"Port preflight passed: {len(published)} loopback ports, project {PROJECT}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["preflight", "up", "down", "status", "logs", "build"])
    args = parser.parse_args()
    bootstrap_env()
    if args.action == "preflight":
        preflight()
    elif args.action == "up":
        preflight()
        compose("build", "api", "web", "provider-fixtures")
        compose("up", "--detach", "--no-build", "--wait", "--wait-timeout", "180")
    elif args.action == "down":
        compose("down")
    elif args.action == "status":
        compose("ps", "--all")
    elif args.action == "logs":
        compose("logs", "--tail", "80")
    elif args.action == "build":
        compose("build", "api", "web", "provider-fixtures")


if __name__ == "__main__":
    main()
