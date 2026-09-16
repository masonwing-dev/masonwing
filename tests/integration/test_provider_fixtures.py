"""Qualifies the synthetic provider's fault behavior, not the product effect broker."""
from http.client import RemoteDisconnected
from concurrent.futures import ThreadPoolExecutor
from uuid import uuid4

import pytest

pytestmark = pytest.mark.integration
PROVIDER = "http://127.0.0.1:39859/cases/"
DIGEST = "sha256:" + "a" * 64


def test_accepted_then_disconnected_can_be_reconciled_without_resending(http):
    base = PROVIDER + uuid4().hex
    with pytest.raises((RemoteDisconnected, ConnectionResetError)):
        http(base + "/effects", {"key": "effect-1", "mode": "accept_then_disconnect", "content_digest": DIGEST})
    status, receipt, _ = http(base + "/lookup/effect-1")
    assert status == 200
    assert receipt["effect_state"] == "APPLIED"
    assert receipt["provider_mode"] == "MOCK"
    counters = http(base + "/counters")[1]
    assert (counters["transmitted_requests"], counters["mutation_count"], counters["lookup_count"]) == (1, 1, 1)


def test_concurrent_same_operation_produces_one_synthetic_mutation(http):
    base = PROVIDER + uuid4().hex
    payload = {"key": "effect-1", "mode": "accept", "content_digest": DIGEST}
    with ThreadPoolExecutor(max_workers=8) as pool:
        responses = list(pool.map(lambda _: http(base + "/effects", payload), range(16)))
    assert {r[0] for r in responses} <= {200, 201}
    assert len({r[1]["remote_id"] for r in responses}) == 1
    assert http(base + "/counters")[1]["mutation_count"] == 1
    assert http(base + "/effects", {**payload, "content_digest": "sha256:" + "b" * 64})[0] == 409
    assert http(base + "/counters")[1]["mutation_count"] == 1


@pytest.mark.parametrize("mode,status", [("reject", 422), ("rate_limit", 429)])
def test_known_rejection_causes_no_mutation(http, mode, status):
    base = PROVIDER + uuid4().hex
    actual, body, headers = http(base + "/effects", {"key": "e", "mode": mode, "content_digest": DIGEST})
    assert actual == status
    assert body["effect_state"] == "NOT_APPLIED"
    assert http(base + "/counters")[1]["mutation_count"] == 0
    if status == 429:
        assert headers["Retry-After"] == "60"


def test_unknown_usage_stays_null(http):
    base = PROVIDER + uuid4().hex
    status, body, _ = http(base + "/effects", {"key": "e", "mode": "unknown_cost", "content_digest": DIGEST})
    assert status == 201
    assert body["usage_microunits"] is None
    assert http(base + "/lookup/e")[1]["usage_microunits"] is None
