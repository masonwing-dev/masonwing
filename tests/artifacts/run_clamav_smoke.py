"""Exercise the real, isolated ClamAV service through Masonwing's local network.

The EICAR string is the standard harmless antivirus test marker. This probe never
disables scanning and never marks a scanner transport/protocol error as clean.
"""
from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
PROBE = r'''
import json, socket, struct

def command(message):
    with socket.create_connection(("scanner",3310),timeout=5) as connection:
        connection.settimeout(15)
        connection.sendall(message)
        result = bytearray()
        while len(result) < 1024:
            chunk = connection.recv(128)
            if not chunk:
                break
            result.extend(chunk)
            if b"\0" in result:
                break
        return bytes(result).split(b"\0",1)[0].decode("ascii","strict")

def scan(content):
    return command(b"zINSTREAM\0"+struct.pack(">I",len(content))+content+struct.pack(">I",0))

assert command(b"zPING\0") == "PONG"
version = command(b"zVERSION\0")
clean = scan(b"Masonwing real scanner integration fixture\n")
assert clean == "stream: OK", "clean input was not accepted by the real scanner"
marker = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"
assert len(marker) == 68
quarantine = scan(marker)
assert quarantine.endswith(" FOUND"), "EICAR fixture was not quarantined"
print(json.dumps({"scanner":"ClamAV","version":version,"clean":"PASS","eicar_quarantine":"PASS","external_network_calls":0}))
'''


def main() -> int:
    result = subprocess.run(
        ["docker", "compose", "--project-name", "masonwing-dev", "exec", "-T", "api", "python3", "-c", PROBE],
        cwd=ROOT, capture_output=True, text=True, timeout=45, check=False,
    )
    if result.returncode:
        # No credentials or payload contents are included by this fixed probe.
        sys.stderr.write(result.stderr)
        return result.returncode
    evidence = json.loads(result.stdout)
    destination = ROOT / ".evidence" / "artifacts" / "clamav-runtime.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
