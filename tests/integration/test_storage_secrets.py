from uuid import uuid4

import pytest

from scripts.local_config import settings

pytestmark = pytest.mark.integration
BUCKET = "masonwing-artifacts-local"


def test_artifact_bytes_are_versioned_and_private(s3, http):
    key = "integration/" + uuid4().hex
    first = "Tài liệu tổng hợp A".encode()
    second = "Tài liệu tổng hợp B".encode()
    versions = []
    try:
        assert s3.get_bucket_versioning(Bucket=BUCKET)["Status"] == "Enabled"
        one = s3.put_object(Bucket=BUCKET, Key=key, Body=first)
        versions.append(one["VersionId"])
        two = s3.put_object(Bucket=BUCKET, Key=key, Body=second)
        versions.append(two["VersionId"])
        assert versions[0] != versions[1]
        assert s3.get_object(Bucket=BUCKET, Key=key, VersionId=versions[0])["Body"].read() == first
        assert s3.get_object(Bucket=BUCKET, Key=key)["Body"].read() == second
        # Anonymous S3 errors use XML, so inspect directly without the JSON helper.
        import urllib.request
        import urllib.error
        with pytest.raises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(f"http://127.0.0.1:39856/{BUCKET}/{key}", timeout=5)
        assert error.value.code == 403
    finally:
        for version in versions:
            s3.delete_object(Bucket=BUCKET, Key=key, VersionId=version)


def test_openbao_tenant_token_reads_only_its_scoped_secret(http):
    base = "http://127.0.0.1:39858/v1/"
    root = {"X-Vault-Token": settings()["BAO_DEV_ROOT_TOKEN_ID"]}
    status, issued, _ = http(base + "auth/token/create", {
        "policies": ["masonwing-tenant_a-readonly"], "no_default_policy": True,
        "ttl": "5m", "renewable": False,
    }, root)
    assert status == 200
    token = issued["auth"]["client_token"]
    headers = {"X-Vault-Token": token}
    try:
        status, body, _ = http(base + "masonwing/data/tenant_a/fixture", headers=headers)
        assert status == 200
        assert body["data"]["data"]["marker"] == "SYNTHETIC_TENANT_A_ONLY"
        assert http(base + "masonwing/data/tenant_b/fixture", headers=headers)[0] == 403
        assert http(base + "masonwing/data/tenant_a/forbidden-write", {"data": {"marker": "denied"}}, headers)[0] == 403
    finally:
        assert http(base + "auth/token/revoke", {"token": token}, root)[0] == 204
