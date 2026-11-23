"""Add license notices for shipped crates that the pinned inventory does not cover.

    cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc > metadata.json
    python add_licenses.py metadata.json <app folder>/licenses

licenses/ and dependency-inventory.json come from the pinned v0.16.0 resource ZIP.
For each runtime crate missing from the inventory, this copies the crate's own
license and notice files into licenses/<name>-<version>/ and adds an inventory
entry in the same form. Existing entries and files are never changed.
"""
import json, re, shutil, sys
from pathlib import Path

NOTICE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT)", re.IGNORECASE)
# MPL-2.0 section 3.2(a): recipients of the executable must be told where the
# source of the MPL-covered files is.
MPL_SOURCE = """{name} {version} is used unmodified, under the Mozilla Public License 2.0
(LICENSE in this folder). Its source code is available at
https://crates.io/crates/{name}/{version}{repository}
"""

meta = json.load(open(sys.argv[1], encoding="utf8"))
licenses = Path(sys.argv[2])
inventory_path = licenses / "dependency-inventory.json"
inventory = json.load(open(inventory_path, encoding="utf8"))
covered = {(e["name"], e["version"]) for e in inventory}

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


def apache_text():
    """The Apache-2.0 text, from a crate in the inventory, for crates that
    offer Apache-2.0 but ship no license file."""
    for entry in inventory:
        for notice in entry["notices"]:
            if notice.lower().endswith(("license-apache", "license-apache-2.0")):
                text = (licenses.parent / notice).read_text(encoding="utf8")
                if "Apache License" in text and "Version 2.0" in text:
                    return text
    raise SystemExit("No Apache-2.0 text found in the inventory")


MIT = """MIT License

Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""
GENERATED = """{name} {version} declares "{license}" in its Cargo.toml but ships no
license file. The license text beside this note was added by this project's
packaging: the standard text{holders}.
"""


def standard_license(package):
    """For a crate without license files: the standard text of a license it
    offers, as ({file name: text}, note), or None."""
    license = package["license"] or ""
    if "LicenseRef-" in license:
        # A project's own licenses (Slint's): the texts another of its crates ships.
        for entry in inventory:
            if entry["license"] == license and entry["notices"]:
                texts = {Path(n).name: (licenses.parent / n).read_text(encoding="utf8")
                         for n in entry["notices"]}
                return texts, f", as shipped by {entry['name']} {entry['version']}"
        return None
    if "Apache-2.0" in license:
        return {"LICENSE-APACHE": apache_text()}, ""
    if "MIT" in license.replace("/", " ").split() or license == "MIT":
        authors = [a.split("<")[0].strip() for a in package.get("authors") or []]
        holders = ", ".join(a for a in authors if a) or f"the {package['name']} authors"
        return {"LICENSE-MIT": MIT.format(holders=holders)}, f", with the authors named there ({holders})"
    return None


added = []
for pid in sorted(shipped, key=lambda p: (packages[p]["name"], packages[p]["version"])):
    package = packages[pid]
    name, version = package["name"], package["version"]
    if not package["source"] or (name, version) in covered:
        continue
    source = Path(package["manifest_path"]).parent
    folder = licenses / f"{name}-{version}"
    if folder.exists():
        raise SystemExit(f"{folder} exists but has no inventory entry")
    files = [f for f in sorted(source.iterdir()) if f.is_file() and NOTICE.match(f.name)]
    if package.get("license_file"):
        extra = source / package["license_file"]
        if extra not in files:
            files.append(extra)
    license = package["license"] or ""
    standard = None if files else standard_license(package)
    if not files and not standard:
        raise SystemExit(f"{name}-{version} ({license}) ships no license file")
    folder.mkdir()
    for f in files:
        shutil.copyfile(f, folder / f.name)
    if standard:
        texts, holders = standard
        for file_name, text in texts.items():
            (folder / file_name).write_text(text, encoding="utf8")
        (folder / "NOTE.txt").write_text(GENERATED.format(
            name=name, version=version, license=license, holders=holders), encoding="utf8")
    if "MPL-2.0" in license:
        repository = f"\nand {package['repository']}" if package.get("repository") else ""
        (folder / "SOURCE.txt").write_text(
            MPL_SOURCE.format(name=name, version=version, repository=repository), encoding="utf8")
    inventory.append({
        "name": name,
        "version": version,
        "license": license,
        "manifest": f"crates.io\\{name}-{version}\\Cargo.toml",
        "licenseFile": package.get("license_file"),
        "notices": [f"licenses/{folder.name}/{f.name}" for f in sorted(folder.iterdir())],
    })
    added.append(f"{name}-{version}")

inventory.sort(key=lambda e: (e["name"], e["version"]))
# As in the pinned file: two-space indent, CRLF line ends.
with open(inventory_path, "w", encoding="utf8", newline="\r\n") as out:
    json.dump(inventory, out, indent=2, ensure_ascii=False)
    out.write("\n")
print(f"{len(added)} license entries added" + (": " + ", ".join(added) if added else ""))
