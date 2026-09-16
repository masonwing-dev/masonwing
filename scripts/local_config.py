"""Read synthetic local settings without evaluating shell expressions."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def settings() -> dict[str, str]:
    values = {}
    for filename in (".env.example", ".env"):
        path = ROOT / filename
        if path.exists():
            for line in path.read_text().splitlines():
                line = line.strip()
                if line and not line.startswith("#") and "=" in line:
                    key, value = line.split("=", 1)
                    values[key.strip()] = value.strip()
    return values
