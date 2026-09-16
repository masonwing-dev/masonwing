"""Register real product scenarios here; a missing implementation is a RED test.

Each driver seeds its Given, performs the real When, and asserts every Then,
including persistence, audit and provider counters. Register individual IDs only
after writing the concrete assertions. Unit-only evidence must stay in unit tests.
"""
from collections.abc import Callable
from typing import Any

Driver = Callable[[dict[str, Any]], None]
DRIVERS: dict[str, Driver] = {}


def scenario(qualified_id: str):
    def register(driver: Driver) -> Driver:
        if qualified_id in DRIVERS:
            raise ValueError(f"Duplicate scenario driver: {qualified_id}")
        DRIVERS[qualified_id] = driver
        return driver
    return register


def execute(case: dict[str, Any]) -> None:
    driver = DRIVERS.get(case["qualified_id"])
    if driver is None:
        raise NotImplementedError(
            f"{case['qualified_id']} — {case.get('title', case['kind'])}\n"
            f"Given: {case.get('preconditions', [])}\n"
            f"When: {case.get('steps', [])}\n"
            f"Then: {case.get('expected', [])}\n"
            f"Evidence: {case.get('evidence_required', [])}\n"
            "Implement a real scenario driver; do not mark this skipped, xfail, or PASS."
        )
    driver(case)
