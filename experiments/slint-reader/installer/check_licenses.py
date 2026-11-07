"""Fail if a crate that ships in the Windows app has no entry in the license inventory.

    cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc > metadata.json
    python check_licenses.py metadata.json <app folder>/licenses/dependency-inventory.json

Build-only crates (build scripts, proc macros used at compile time) are also in the
inventory, so only missing runtime crates are reported.
"""
import json, sys

meta = json.load(open(sys.argv[1], encoding="utf8"))
inventory = json.load(open(sys.argv[2], encoding="utf8"))
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
shipped, stack = set(), [meta["resolve"]["root"]]
while stack:
    pid = stack.pop()
    if pid in shipped:
        continue
    shipped.add(pid)
    for dep in nodes[pid]["deps"]:
        if any(kind["kind"] is None for kind in dep["dep_kinds"]):
            stack.append(dep["pkg"])
names = sorted(f"{packages[p]['name']}-{packages[p]['version']}" for p in shipped if packages[p]["source"])
covered = {f"{e['name']}-{e['version']}" for e in inventory}
missing = [n for n in names if n not in covered]
print(f"{len(names)} shipped crates, {len(missing)} without a license entry")
if missing:
    print("Regenerate licenses/ and dependency-inventory.json for:", ", ".join(missing))
    sys.exit(1)
