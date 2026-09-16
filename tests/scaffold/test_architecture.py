"""Enforce structural ownership and complete scaffold coverage, not domain acceptance."""
import json
import tomllib
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]


def test_every_feature_has_a_context_screen_and_work_package():
    contexts = json.loads((ROOT / "contracts/domain-contexts.json").read_text())["contexts"]
    expected_count = 45 if (ROOT / "gleanbird-requirements-v1.0.1").is_dir() else 23
    assert len(contexts) == expected_count
    assert len({(c["product"], c["feature"]["id"]) for c in contexts}) == expected_count
    for context in contexts:
        assert context["screen"]
        assert context["work_package"]["feature_id"] == context["feature"]["id"]
        if not context["commands"]:
            # F-009 owns transactional data invariants, not an independent command surface.
            assert (context["product"], context["feature"]["id"]) == ("masonwing", "F-009")
            assert context["screen"]["actions"] == []
        assert context["cases"]


def test_domain_packages_can_only_use_public_workspace_dependencies():
    packages = list((ROOT / "products").glob("**/Cargo.toml"))
    expected_count = 10 if (ROOT / "gleanbird-requirements-v1.0.1").is_dir() else 2
    assert len(packages) == expected_count
    for path in packages:
        manifest = tomllib.loads(path.read_text())
        for name, dependency in manifest.get("dependencies", {}).items():
            if name.startswith("masonwing-"):
                assert name in {"masonwing-sdk", "masonwing-contracts"}, (path, name)
            if isinstance(dependency, dict) and "path" in dependency:
                resolved = (path.parent / dependency["path"]).resolve()
                assert resolved in {ROOT / "crates/sdk", ROOT / "crates/contracts"}, resolved
        assert manifest["package"].get("publish") is False or manifest["package"].get("publish") == {"workspace": True}


def test_compose_only_gateway_publishes_exact_requested_loopback_ports():
    compose = yaml.safe_load((ROOT / "compose.yaml").read_text())
    assert compose["name"] == "masonwing-dev"
    assert compose["networks"]["default"]["internal"] is True
    gateway = compose["services"]["gateway"]
    assert set(gateway["ports"]) == {f"127.0.0.1:{p}:{p}" for p in range(39850, 39860)}
    for name, service in compose["services"].items():
        if name != "gateway":
            assert not service.get("ports"), name
            assert "edge" not in service.get("networks", []), name
        if not service.get("build"):
            assert "@sha256:" in service["image"], name
    for name in ("api", "worker", "component-runner", "remote-runner"):
        service = compose["services"][name]
        assert service["read_only"] is True
        assert service["cap_drop"] == ["ALL"]
        assert service["environment"]["MASONWING_EXTERNAL_MUTATIONS"] == "false"
        assert service["environment"]["MASONWING_LIVE_BUDGET_MICROUNITS"] == "0"
    serialized = (ROOT / "compose.yaml").read_text()
    assert "/var/run/docker.sock" not in serialized
    assert "privileged: true" not in serialized


def test_keycloak_fixture_uses_only_code_flow_with_pkce_and_exact_redirect():
    realm = json.loads((ROOT / "infra/keycloak/masonwing-realm.json").read_text())
    client = realm["clients"][0]
    assert client["publicClient"] is False
    assert client["attributes"]["pkce.code.challenge.method"] == "S256"
    assert client["standardFlowEnabled"] is True
    assert client["directAccessGrantsEnabled"] is False
    assert client["implicitFlowEnabled"] is False
    assert client["redirectUris"] == ["http://localhost:39850/auth/callback"]
