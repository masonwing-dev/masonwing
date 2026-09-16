import json
import urllib.error
import urllib.request

import boto3
import psycopg
import pytest
from botocore.config import Config

from scripts.local_config import settings


@pytest.fixture
def db():
    with psycopg.connect("postgresql://masonwing_app:masonwing-local-app@127.0.0.1:39852/masonwing",
                         autocommit=True, connect_timeout=5) as connection:
        yield connection


@pytest.fixture
def s3():
    config = settings()
    return boto3.client("s3", endpoint_url="http://127.0.0.1:39856", region_name="us-east-1",
        aws_access_key_id=config["MINIO_ROOT_USER"], aws_secret_access_key=config["MINIO_ROOT_PASSWORD"],
        config=Config(signature_version="s3v4", s3={"addressing_style": "path"}, retries={"max_attempts": 0}))


@pytest.fixture
def http():
    def request(url, body=None, headers=None, method=None):
        data = None if body is None else (body if isinstance(body, bytes) else json.dumps(body).encode())
        req = urllib.request.Request(url, data=data,
            headers={"Content-Type": "application/json", **(headers or {})}, method=method)
        try:
            response = urllib.request.urlopen(req, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else None, dict(response.headers)
    return request
