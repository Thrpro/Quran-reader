#!/usr/bin/env python3
"""Checks the Quran resources against resource-manifest.json, without changing them.

    python3 verify_resources.py [folder] [--expected-manifest-sha256 HASH]

`folder` holds resource-manifest.json and the Resources folder (default: this
script's folder). The same checks as Verify-Quran-Resources.ps1 in the Windows
package: every listed file has its size and SHA-256, no path is linked or
escapes the folder, and no other file is present. Prints a JSON summary.
"""
import hashlib, json, os, sys
from pathlib import Path

FILES, ORIGINAL = 1569, 1567


def fail(message):
    raise SystemExit(f"Quran resource check failed: {message}")


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def verify(folder, expected_manifest=None, resources=None):
    """`resources` overrides where the Resources folder is (a macOS bundle
    keeps it in Contents/Resources/Mushaf)."""
    folder = Path(folder)
    root = Path(resources) if resources else folder / "Resources"
    if root.is_symlink() or not root.is_dir():
        fail("the Resources folder is missing or linked")
    manifest_path = folder / "resource-manifest.json"
    manifest_hash = sha256(manifest_path)
    if expected_manifest and manifest_hash != expected_manifest.lower():
        fail("resource-manifest.json differs from the expected hash")
    pack = json.loads(manifest_path.read_text(encoding="utf8"))
    if (pack.get("schema") != "quran-mushaf-resource-pack-v1"
            or len(pack.get("files", [])) != FILES or len(pack.get("resources", [])) != ORIGINAL):
        fail("unexpected resource pack schema or file count")
    seen, total = set(), 0
    for entry in pack["files"]:
        rel = entry.get("path") or ""
        parts = rel.split("/")
        if ":" in rel or "\\" in rel or any(p in ("", ".", "..") for p in parts):
            fail(f"unsafe resource path {rel!r}")
        if rel in seen:
            fail(f"duplicate resource path {rel}")
        path = root
        for part in parts:
            path = path / part
            if path.is_symlink():
                fail(f"linked resource path {rel}")
        if not path.is_file() or path.stat().st_size != entry["bytes"]:
            fail(f"size differs: {rel}")
        if sha256(path) != entry["sha256"]:
            fail(f"hash differs: {rel}")
        seen.add(rel)
        total += entry["bytes"]
    for directory, _, names in os.walk(root):
        for name in names:
            rel = (Path(directory) / name).relative_to(root).as_posix()
            if rel not in seen:
                fail(f"unexpected resource file: {rel}")
    return {"passed": True, "resourcePackId": pack.get("id"), "manifestSha256": manifest_hash,
            "files": len(seen), "bytes": total, "originalResources": ORIGINAL,
            "writesPerformed": False}


if __name__ == "__main__":
    args = sys.argv[1:]
    expected = None
    if "--expected-manifest-sha256" in args:
        i = args.index("--expected-manifest-sha256")
        expected = args[i + 1]
        del args[i:i + 2]
    folder = args[0] if args else Path(__file__).resolve().parent
    print(json.dumps(verify(folder, expected), indent=2))
