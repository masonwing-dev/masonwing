"""Public HTTP admission/readiness contracts; authenticated flows have separate evidence."""
import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, FormatChecker

pytestmark = pytest.mark.integration
ROOT = Path(__file__).resolve().parents[2]
API = "http://127.0.0.1:39851"
schema = json.loads((ROOT / "contracts/masonwing/contracts.json").read_text())
ERROR = Draft202012Validator({**schema, "$ref": "#/$defs/Error"}, format_checker=FormatChecker())
PRODUCTS = tuple(json.loads((ROOT / "contracts/spec-inventory.json").read_text())["products"])
PATHS = sorted({path for product in PRODUCTS
    for path in json.loads((ROOT / f"contracts/{product}/openapi.json").read_text())["paths"]
    if "/commands/" in path})


@pytest.mark.parametrize("path", PATHS)
def test_all_command_paths_deny_unauthenticated_malformed_body_before_parsing(http, path):
    url = API + path.replace("{tenant_id}", "tenant_a").replace("{tenantId}", "tenant_a")
    status, body, _ = http(url, b"{not json", method="POST")
    assert status == 401, (path, status, body)
    ERROR.validate(body)
    assert body["effect_state"] == "NOT_SENT"
    assert body["retryable"] is False


def test_fake_credentials_never_result_in_a_success_receipt(http):
    status, body, _ = http(API + "/v1/tenants/tenant_a/commands/effect.dispatch", {}, {"Authorization": "Bearer SYNTHETIC_NOT_VALID"})
    assert status == 401
    ERROR.validate(body)
    assert body["effect_state"] == "NOT_SENT"
    assert "command_id" not in body


def test_live_process_does_not_claim_write_readiness(http):
    assert http(API + "/health/live")[0] == 200
    status, body, _ = http(API + "/health/ready")
    assert status == 503
    assert body == {"ready": False, "reason": "QUALIFICATION_INCOMPLETE", "external_mutations": False, "live_budget_microunits": 0}
    status, body, _ = http(API + "/dev/status")
    assert status == 200
    assert body["external_mutations_enabled"] is False
    assert body["live_budget_microunits"] == 0
    assert body["readiness"] == {"ready": False, "status": "QUALIFICATION_INCOMPLETE"}
    assert body["operation_catalog_count"] == len(json.loads((ROOT / "contracts/masonwing/operations.json").read_text())["operations"])
    assert (body["identity"], body["authorization"], body["data_store"], body["artifact_store"]) == ("OIDC", "CEDAR", "POSTGRESQL", "S3")


def test_unknown_operation_is_not_part_of_the_command_catalog(http):
    assert http(API + "/v1/tenants/tenant_a/commands/sql.eval", {})[0] == 404


def test_keycloak_exposes_real_discovery_and_signing_keys(http):
    status, discovery, _ = http("http://127.0.0.1:39853/realms/masonwing/.well-known/openid-configuration")
    assert status == 200
    assert discovery["issuer"] == "http://localhost:39853/realms/masonwing"
    assert "S256" in discovery["code_challenge_methods_supported"]
    status, jwks, _ = http(discovery["jwks_uri"])
    assert status == 200
    assert any(key.get("kid") and key.get("use") == "sig" for key in jwks["keys"])
