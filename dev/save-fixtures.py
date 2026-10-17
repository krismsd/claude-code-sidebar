#!/usr/bin/env python3
"""Copy captured hook payloads into dev/fixtures/, scrubbing personal paths.
Keeps the first payload per event, plus named variants passed as  NNNN=variant.
Usage: save-fixtures.py <captured-dir> [0024=agent-handback ...]
Existing fixtures are never overwritten (tests depend on them); delete one first to replace it.
"""
import json, pathlib, re, sys, getpass

src = pathlib.Path(sys.argv[1])
variants = dict(a.split("=", 1) for a in sys.argv[2:])
out = pathlib.Path(__file__).resolve().parent / "fixtures"
user = getpass.getuser()

def scrub(s: str) -> str:
    return re.sub(rf"\b{re.escape(user)}\b", "user", s)

seen = set()
for f in sorted(src.glob("*.json")):
    num, event = f.stem.split("-", 1)
    if event in ("Ping", "unknown"):
        continue
    name = f"{event}.{variants[num]}" if num in variants else event
    if name in seen or (num not in variants and event in seen):
        continue
    seen.add(name)
    dest = out / f"{name}.json"
    if dest.exists():
        print(f"{f.name}: {dest.name} exists, skipped")
        continue
    data = json.loads(scrub(f.read_text()))
    data.pop("_received_ms", None)
    dest.write_text(json.dumps(data, indent=2) + "\n")
    print(f"{f.name} -> {name}.json")
