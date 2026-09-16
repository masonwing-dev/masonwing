"""Structure/wire checks only. These are not application acceptance evidence."""
import hashlib
import json
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator, FormatChecker
from openapi_spec_validator import validate

from scripts.sync_specs import generate
from tests.acceptance.drivers import execute

ROOT = Path(__file__).resolve().parents[2]


def test_generated_files_match_inputs():
    if not AVAILABLE_PRODUCTS:
        pytest.skip("No requirements bundles in standalone workspace")
    for path, expected in generate().items():
        assert path.read_bytes() == expected, str(path.relative_to(ROOT))


AVAILABLE_PRODUCTS = [p for p in ("masonwing", "gleanbird") if (ROOT / f"{p}-requirements-v1.0.1").is_dir()]


@pytest.mark.parametrize("product", AVAILABLE_PRODUCTS)
def test_original_bundle_checksums(product):
    root = ROOT / f"{product}-requirements-v1.0.1"
    for line in (root / "MANIFEST.sha256").read_text().splitlines():
        if not line.strip():
            continue
        digest, relative = line.split(maxsplit=1)
        assert hashlib.sha256((root / relative.lstrip("*")).read_bytes()).hexdigest() == digest, relative


def test_every_definition_validates_its_syntax_example():
    contracts_dir = ROOT / "contracts/masonwing"
    schema = json.loads((contracts_dir / "contracts.json").read_text())
    examples = json.loads((contracts_dir / "wire-examples.json").read_text())["examples"]
    Draft202012Validator.check_schema(schema)
    assert set(examples) == set(schema["$defs"])
    for name, example in examples.items():
        validator = Draft202012Validator({**schema, "$ref": f"#/$defs/{name}"}, format_checker=FormatChecker())
        validator.validate(example)


def test_openapi_with_dedicated_validator():
    for openapi_path in (ROOT / "contracts").glob("*/openapi.json"):
        validate(json.loads(openapi_path.read_text()))


def test_complete_collection_and_namespace_isolation():
    cases = json.loads((ROOT / "tests/acceptance/catalog.json").read_text())["cases"]
    assert len(cases) == 686
    assert len({c["qualified_id"] for c in cases}) == len(cases)
    for product in AVAILABLE_PRODUCTS:
        original = json.loads((ROOT / f"{product}-requirements-v1.0.1/06-testing/test-cases.json").read_text())["cases"]
        selected = [c for c in cases if c["product"] == product]
        assert [c["id"] for c in selected] == [c["id"] for c in original]
        for actual, source in zip(selected, original, strict=True):
            assert all(actual[key] == value for key, value in source.items())


def test_unimplemented_acceptance_is_red_and_preserves_expected_output():
    case = {"qualified_id": "TEST:UNIMPLEMENTED", "kind": "acceptance",
            "expected": ["provider transmit=0; resource version unchanged"]}
    with pytest.raises(NotImplementedError, match="provider transmit=0"):
        execute(case)


def test_shared_contracts_are_identical():
    if not (ROOT / "contracts/gleanbird").is_dir():
        pytest.skip("Gleanbird contracts not in standalone workspace")
    for name in ("shared-contracts.json", "masonwing-platform.openapi.json"):
        assert (ROOT / "contracts/masonwing" / name).read_bytes() == (ROOT / "contracts/gleanbird" / name).read_bytes()

