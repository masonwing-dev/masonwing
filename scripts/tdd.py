#!/usr/bin/env python3
"""Run one exact namespaced acceptance ID, or the whole RED backlog if CASE is empty."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    selected = os.environ.get("MASONWING_TDD_CASE", "").strip()
    catalog = json.loads((ROOT / "tests/acceptance/catalog.json").read_text())["cases"]
    identifiers = {case["qualified_id"] for case in catalog}
    if selected and selected not in identifiers:
        print(f"Unknown exact acceptance ID: {selected}", file=sys.stderr)
        print("Use a qualified_id from tests/acceptance/catalog.json.", file=sys.stderr)
        return 2
    target = (f"tests/acceptance/test_requirements.py::test_specified_behavior[{selected}]"
              if selected else "tests/acceptance")
    # Exact pytest node selection supports @ and : without -k expression parsing.
    # Pass argv directly; never evaluate a CASE value as shell code.
    return subprocess.run([sys.executable, "-m", "pytest", target, "-q"], cwd=ROOT, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
