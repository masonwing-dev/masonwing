#!/usr/bin/env python3
"""Idempotent synthetic fixtures for Masonwing-owned local services only."""
from __future__ import annotations

import json
import time
import urllib.error
import urllib.request

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

from local_config import settings

BUCKET = "masonwing-artifacts-local"
SYNTHETIC_ARTIFACT = "Bằng chứng tổng hợp — không có dữ liệu cá nhân thật.".encode()


def bao_request(path: str, token: str, body: dict | None = None, method: str | None = None):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request("http://127.0.0.1:39858/v1/" + path, data=data,
        headers={"X-Vault-Token": token, "Content-Type": "application/json"}, method=method)
    # A just-mounted KV v2 engine can briefly return its documented upgrade-in-progress response.
    for attempt in range(20):
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                payload = response.read()
                return json.loads(payload) if payload else None
        except urllib.error.HTTPError as error:
            if error.code != 400:
                raise
            payload = error.read().decode(errors="replace")
            if "upgrad" not in payload.lower() or attempt == 19:
                raise RuntimeError(f"OpenBao request failed at {path}: HTTP 400 {payload}") from None
            time.sleep(0.25)


def main():
    config = settings()
    if config.get("MASONWING_ENV") != "LOCAL":
        raise SystemExit("Synthetic seeding requires MASONWING_ENV=LOCAL")
    s3 = boto3.client("s3", endpoint_url="http://127.0.0.1:39856", region_name="us-east-1",
        aws_access_key_id=config["MINIO_ROOT_USER"], aws_secret_access_key=config["MINIO_ROOT_PASSWORD"],
        config=Config(signature_version="s3v4", s3={"addressing_style": "path"}, retries={"max_attempts": 0}))
    try:
        s3.head_bucket(Bucket=BUCKET)
    except ClientError as error:
        if error.response["ResponseMetadata"]["HTTPStatusCode"] != 404:
            raise
        s3.create_bucket(Bucket=BUCKET)
    s3.put_bucket_versioning(Bucket=BUCKET, VersioningConfiguration={"Status": "Enabled"})
    key = "fixtures/tenant_a/synthetic-evidence.txt"
    try:
        s3.head_object(Bucket=BUCKET, Key=key)
    except ClientError as error:
        if error.response["ResponseMetadata"]["HTTPStatusCode"] != 404:
            raise
        s3.put_object(Bucket=BUCKET, Key=key, Body=SYNTHETIC_ARTIFACT, ContentType="text/plain; charset=utf-8")
    root_token = config["BAO_DEV_ROOT_TOKEN_ID"]
    mounts = bao_request("sys/mounts", root_token)
    if "masonwing/" not in mounts.get("data", mounts):
        bao_request("sys/mounts/masonwing", root_token, {"type": "kv", "options": {"version": "2"}})
    for tenant in ("tenant_a", "tenant_b"):
        path = f"masonwing/data/{tenant}/fixture"
        try:
            bao_request(path, root_token)
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            bao_request(path, root_token, {"data": {"marker": f"SYNTHETIC_{tenant.upper()}_ONLY"}})
        policy = f'path "masonwing/data/{tenant}/*" {{ capabilities = ["read"] }}'
        bao_request(f"sys/policies/acl/masonwing-{tenant}-readonly", root_token, {"policy": policy}, "PUT")
    print("Seeded local private/versioned S3 artifact and tenant-scoped OpenBao fixtures; no credentials emitted.")


if __name__ == "__main__":
    main()
