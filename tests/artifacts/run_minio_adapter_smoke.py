"""Run the Rust artifact adapter probe against the existing masonwing-dev MinIO.

This runner reads only synthetic local credentials through scripts.local_config,
passes them via the child environment, and never prints credential values.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from scripts.local_config import settings


def main() -> int:
    config = settings()
    env = os.environ.copy()
    env.update(
        {
            "MASONWING_ENV": "LOCAL",
            "S3_ENDPOINT": "http://127.0.0.1:39856",
            "S3_BUCKET": "masonwing-artifacts-local",
            "MINIO_ROOT_USER": config["MINIO_ROOT_USER"],
            "MINIO_ROOT_PASSWORD": config["MINIO_ROOT_PASSWORD"],
            "AWS_REGION": "us-east-1",
        }
    )
    completed = subprocess.run(
        [
            "cargo",
            "test",
            "--locked",
            "-p",
            "masonwing-artifacts-adapter",
            "--test",
            "minio",
            "--jobs",
            "2",
            "--",
            "--ignored",
            "--exact",
            "local_minio_enforces_immutable_put_and_bounded_roundtrip",
        ],
        cwd=ROOT,
        env=env,
        check=False,
    )
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
