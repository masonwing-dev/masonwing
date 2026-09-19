"""HTTP integration for durable-execution command surface (WP-011..014).

Exercises receipt admission, tenant isolation, CSRF rejection, idempotency
conflicts and unauthenticated barriers across the durable execution commands:
run.start, run.cancel, approval.decide, effect.propose, effect.dispatch,
effect.reconcile, effect.compensate, budget.configure.

Authentication uses the real loopback Keycloak Authorization Code + PKCE flow;
authorization uses real Cedar evaluation over tenant-scoped PostgreSQL rows.
"""
from __future__ import annotations

import json
import subprocess
from pathlib import Path
from uuid import uuid4

import pytest
from jsonschema import Draft202012Validator, FormatChecker

from tests.integration.oidc_session import begin_login

pytestmark = pytest.mark.integration

ROOT = Path(__file__).resolve().parents[2]
SCHEMA = json.loads((ROOT / "contracts/masonwing/contracts.json").read_text())
ERROR = Draft202012Validator({**SCHEMA, "$ref": "#/$defs/Error"}, format_checker=FormatChecker())

DURABLE_COMMANDS = (
    "run.start",
    "run.cancel",
    "approval.decide",
    "effect.propose",
    "effect.dispatch",
    "effect.reconcile",
    "effect.compensate",
    "budget.configure",
)


def _operator_sql(statement: str) -> None:
    subprocess.run(
        [
            "docker",
            "compose",
            "--project-name",
            "masonwing-dev",
            "exec",
            "-T",
            "postgres",
            "psql",
            "-U",
            "masonwing_migrator",
            "-d",
            "masonwing",
            "-v",
            "ON_ERROR_STOP=1",
            "-q",
        ],
        input=statement.encode(),
        check=True,
        capture_output=True,
    )


@pytest.fixture(scope="module")
def oidc_user():
    """Single authenticated Keycloak session shared across module tests."""
    return begin_login()


@pytest.fixture
def ephemeral_tenant(oidc_user):
    """Provisions an isolated tenant with OWNER role for the authenticated user."""
    tenant = str(uuid4())
    membership = str(uuid4())
    sql = f"""BEGIN;
SELECT set_config('app.tenant_id','{tenant}',true);
INSERT INTO tenants(id,name) VALUES('{tenant}','Ephemeral pytest tenant');
INSERT INTO memberships(tenant_id,principal_id,membership_id,role,status,membership_epoch,permission_epoch)
  VALUES('{tenant}','{oidc_user.principal_id}','{membership}','OWNER','ACTIVE',1,1);
INSERT INTO membership_roles(tenant_id,membership_id,role) VALUES('{tenant}','{membership}','OWNER');
INSERT INTO authorization_policies(tenant_id,policy_version,policy_epoch,cedar_source,is_current)
  VALUES('{tenant}','1.0.0',1,'permit(principal, action, resource);',true);
COMMIT;"""
    _operator_sql(sql)
    try:
        yield tenant
    finally:
        tables = [
            "effect_receipts",
            "effects",
            "approvals",
            "cost_ledger",
            "cost_reservations",
            "budget_accounts",
            "budget_settings",
            "run_checkpoints",
            "runs",
            "connections",
            "resource_projections",
            "artifact_lineage",
            "artifact_rights",
            "delegations",
            "product_compositions",
            "workflow_definitions",
            "plugin_revocations",
            "plugin_installations",
            "plugin_versions",
            "artifact_signatures",
            "publisher_keys",
            "command_receipts",
            "audit_events",
            "outbox_events",
            "artifacts",
            "authorization_policies",
            "membership_roles",
            "memberships",
        ]
        cleanup = (
            f"BEGIN; SELECT set_config('app.tenant_id','{tenant}',true);\n"
            + "\n".join(f"DELETE FROM {t} WHERE tenant_id='{tenant}';" for t in tables)
            + f"\nDELETE FROM tenants WHERE id='{tenant}'; COMMIT;"
        )
        _operator_sql(cleanup)


# --- Section 1: Unauthenticated admission barriers -------------------------


@pytest.mark.parametrize("operation", DURABLE_COMMANDS)
def test_durable_commands_deny_unauthenticated_requests_before_processing(http, operation):
    url = f"http://127.0.0.1:39851/v1/tenants/tenant_a/commands/{operation}"
    status, body, _ = http(url, body={"synthetic": "data"}, method="POST")
    assert status == 401
    ERROR.validate(body)
    assert body["effect_state"] == "NOT_SENT"
    assert body["retryable"] is False
    assert "command_id" not in body


# --- Section 2: Authenticated admission & gate checks -----------------------


@pytest.mark.parametrize("operation", DURABLE_COMMANDS)
def test_authenticated_command_rejects_missing_csrf_token(oidc_user, ephemeral_tenant, operation):
    url = f"{oidc_user.origin}/v1/tenants/{ephemeral_tenant}/commands/{operation}"
    headers = {"Origin": oidc_user.origin, "Idempotency-Key": str(uuid4())}
    status, body, _ = oidc_user.request(url, body={}, method="POST", headers=headers)
    assert status == 403
    ERROR.validate(body)
    assert body["code"] == "CSRF_REJECTED"
    assert body["effect_state"] == "NOT_SENT"


@pytest.mark.parametrize("operation", DURABLE_COMMANDS)
def test_authenticated_command_rejects_foreign_origin(oidc_user, ephemeral_tenant, operation):
    url = f"{oidc_user.origin}/v1/tenants/{ephemeral_tenant}/commands/{operation}"
    headers = oidc_user.headers()
    headers["Origin"] = "https://evil.example.test"
    headers["Idempotency-Key"] = str(uuid4())
    status, body, _ = oidc_user.request(url, body={}, method="POST", headers=headers)
    assert status == 403
    ERROR.validate(body)
    assert body["code"] == "CSRF_REJECTED"


@pytest.mark.parametrize("operation", DURABLE_COMMANDS)
def test_authenticated_command_returns_404_for_unadmitted_tenant(oidc_user, operation):
    foreign_tenant = str(uuid4())
    url = f"{oidc_user.origin}/v1/tenants/{foreign_tenant}/commands/{operation}"
    headers = {**oidc_user.headers(), "Idempotency-Key": str(uuid4())}
    status, body, _ = oidc_user.request(url, body=b"{invalid-json", method="POST", headers=headers, raw=b"{not")
    assert status == 404
    ERROR.validate(body)
    assert body["effect_state"] == "NOT_SENT"


@pytest.mark.parametrize("operation", DURABLE_COMMANDS)
def test_authenticated_command_validates_json_schema(oidc_user, ephemeral_tenant, operation):
    url = f"{oidc_user.origin}/v1/tenants/{ephemeral_tenant}/commands/{operation}"
    headers = {**oidc_user.headers(), "Idempotency-Key": str(uuid4())}
    # Send empty object which fails required fields on every durable command
    status, body, _ = oidc_user.request(url, body={}, method="POST", headers=headers)
    # budget.configure requires step-up authentication before body parsing.
    # All other durable commands run schema validation first.
    if operation == "budget.configure":
        assert status == 403
        ERROR.validate(body)
        assert body["code"] == "STEP_UP_REQUIRED"
        assert body["effect_state"] == "NOT_SENT"
    else:
        assert status == 400
        ERROR.validate(body)
        assert body["code"] == "SCHEMA_INVALID"
        assert body["effect_state"] == "NOT_SENT"


# --- Section 3: Idempotency conflict & receipt semantics -------------------


def test_budget_configure_requires_step_up_and_rejects_normal_session(oidc_user, ephemeral_tenant):
    """budget.configure is gated by step-up before its body is ever parsed (REQ-146/ADM)."""
    url = f"{oidc_user.origin}/v1/tenants/{ephemeral_tenant}/commands/budget.configure"
    headers = {**oidc_user.headers(), "Idempotency-Key": str(uuid4())}
    body = {
        "currency": "USD",
        "limit_microunits": 500_000,
        "period": "RUN",
        "expected_version": 1,
    }
    status, error, _ = oidc_user.request(url, body=body, method="POST", headers=headers)
    assert status == 403
    ERROR.validate(error)
    assert error["code"] == "STEP_UP_REQUIRED"
    assert error["effect_state"] == "NOT_SENT"




def test_durable_commands_reject_missing_target_with_not_found(oidc_user, ephemeral_tenant):
    """Commands targeting non-existent resources return 404 before execution."""
    for op, field_name, payload in [
        ("run.cancel", "run_id", {"expected_version": 1, "reason": "test"}),
        (
            "approval.decide",
            "proposal_id",
            {
                "decision": "APPROVE",
                "expected_version": 1,
                "content_digest": "sha256:" + "0" * 64,
                "reason": "test",
            },
        ),
        ("effect.dispatch", "effect_id", {"expected_version": 1}),
        ("effect.reconcile", "effect_id", {"expected_version": 1}),
    ]:
        url = f"{oidc_user.origin}/v1/tenants/{ephemeral_tenant}/commands/{op}"
        headers = {**oidc_user.headers(), "Idempotency-Key": str(uuid4())}
        body = {field_name: str(uuid4()), **payload}
        status, error, _ = oidc_user.request(url, body=body, method="POST", headers=headers)
        assert status == 404, f"{op} expected 404, got {status}: {error}"
        ERROR.validate(error)
        assert error["effect_state"] == "NOT_SENT"
