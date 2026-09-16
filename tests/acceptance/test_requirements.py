"""Executable collection for every baseline case; intentionally RED until implemented."""
import json
from pathlib import Path

import pytest

from tests.acceptance.drivers import execute

CATALOG = json.loads(Path(__file__).with_name("catalog.json").read_text())["cases"]


@pytest.mark.acceptance
@pytest.mark.parametrize("case", CATALOG, ids=[c["qualified_id"] for c in CATALOG])
def test_specified_behavior(case):
    execute(case)
