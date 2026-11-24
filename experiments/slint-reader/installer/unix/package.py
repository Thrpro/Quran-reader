#!/usr/bin/env python3
"""Packages the reader for Linux or macOS from a built executable.

    python3 package.py --target linux --exe <built exe> --version 1.2.3 \\
        --out <new folder> --metadata <cargo metadata json>

Linux: an AppImage, a .deb and a .tar.gz. macOS: a .dmg holding the app
bundle. Each comes with SHA256SUMS-<target>.txt in <out>/release.

As in package.ps1 for Windows, the Quran resources are not built here: they
come from an earlier release ZIP, pinned by its SHA-256, and are checked file
by file against the resource manifest, whose own SHA-256 is pinned too.
Nothing in Resources is modified. Licenses come from the same ZIP, with
notices added for newer crates (add_licenses.py), and the package fails if any
shipped crate has none.

--layout-only builds the folders and the .tar.gz but no AppImage, .deb or
.dmg, so the layout can be checked on any system. The AppImage needs
--appimagetool and --runtime (pinned downloads, see the CI workflow); the .deb
needs dpkg-deb; the .dmg needs macOS. On macOS the app is signed with
$MACOS_SIGN_IDENTITY (a Developer ID) if set, else ad hoc; with
$MACOS_NOTARY_PROFILE (a notarytool keychain profile) the .dmg is notarized and
stapled too.
"""
import argparse, os, plistlib, shutil, struct, subprocess, sys
import tarfile, urllib.request, zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
INSTALLER = HERE.parent
READER = INSTALLER.parent
sys.path.insert(0, str(HERE))
from verify_resources import verify, sha256  # noqa: E402

RESOURCE_ZIP = ("https://github.com/Thrpro/Quran-reader/releases/download/v0.16.0/"
                "QuranMushafReader-Windows-x64-0.16.0.zip")
RESOURCE_ZIP_SHA256 = "97a8d7e7be094996e68ab369bff6303c8588463cf247a9a3a4c900c007fc58e0"
MANIFEST_SHA256 = "378e25fde0bbbb24e185393cbc16a704c56dd10b47391cfaae2347e9e90ccc8a"

APP_ID = "quran-mushaf-reader"           # executable, desktop entry, window name
APP_NAME = "Quran Mushaf Reader"
BUNDLE_ID = "io.github.thrpro.quran-mushaf-reader"
HOMEPAGE = "https://github.com/Thrpro/Quran-reader"
DESCRIPTION = "Read the Quran page by page in the Madani Mushaf, with recitation and tafsir"

# The executable links fontconfig, FreeType, ALSA and the C++ runtime (for
# Skia), and loads the rest when it opens
# a window: xkbcommon, Wayland or X11, and EGL or GLX for GPU drawing.
DEB_DEPENDS = ("libc6 (>= 2.39), libgcc-s1, libstdc++6, libfontconfig1, libfreetype6, libasound2t64 | libasound2, "
               "libxkbcommon0, libwayland-client0 | libx11-6")
DEB_RECOMMENDS = ("libwayland-cursor0, libwayland-egl1, libx11-xcb1, libxcursor1, libxi6, "
                  "libxrandr2, libxkbcommon-x11-0, libegl1, libgl1")
# Shared libraries the executable may name directly (checked after the build).
LINKED = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "libstdc++.so.6", "libdl.so.2", "libpthread.so.0",
          "librt.so.1", "ld-linux-x86-64.so.2", "libfontconfig.so.1", "libfreetype.so.6",
          "libasound.so.2"}

DESKTOP = f"""[Desktop Entry]
Type=Application
Name={APP_NAME}
Name[ar]=قارئ المصحف
GenericName=Quran reader
Comment={DESCRIPTION}
Exec={APP_ID}
Icon={APP_ID}
Terminal=false
Categories=Education;Literature;
Keywords=Quran;Mushaf;Tafsir;Recitation;قرآن;مصحف;
StartupWMClass={APP_ID}
"""


def run(*command, **kw):
    print("+", " ".join(str(c) for c in command), flush=True)
    subprocess.run([str(c) for c in command], check=True, **kw)


def copy_tree(source, target):
    shutil.copytree(source, target, symlinks=False)


def resource_base(work, zip_source):
    """The pinned release ZIP's Resources, manifest and licenses, verified."""
    archive = work / "base.zip"
    if zip_source.startswith("https://"):
        with urllib.request.urlopen(zip_source, timeout=120) as response, open(archive, "wb") as f:
            shutil.copyfileobj(response, f)
    else:
        shutil.copyfile(zip_source, archive)
    actual = sha256(archive)
    if actual != RESOURCE_ZIP_SHA256:
        raise SystemExit(f"Resource base ZIP SHA-256 is {actual}, expected {RESOURCE_ZIP_SHA256}")
    target = work / "base"
    with zipfile.ZipFile(archive) as z:
        for name in z.namelist():
            parts = name.replace("\\", "/").split("/")
            if name.startswith(("/", "\\")) or ":" in name or ".." in parts:
                raise SystemExit(f"Unsafe path in the resource ZIP: {name}")
        z.extractall(target)
    folders = [p for p in target.iterdir() if p.is_dir()]
    if len(folders) != 1:
        raise SystemExit("The resource ZIP should hold one folder")
    base = folders[0]
    verify(base, MANIFEST_SHA256)
    return base


def licenses(base, work, metadata):
    folder = work / "licenses"
    copy_tree(base / "licenses", folder)
    # The Windows runtime's notice is not part of these packages.
    (folder / "Microsoft-Visual-C++-Runtime.txt").unlink(missing_ok=True)
    run(sys.executable, INSTALLER / "add_licenses.py", metadata, folder)
    run(sys.executable, INSTALLER / "check_licenses.py", metadata,
        folder / "dependency-inventory.json")
    return folder


def executable(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(0o755)


def start_here(version, target):
    text = (HERE / "START-HERE.md").read_text(encoding="utf8").replace("{VERSION}", version)
    target.write_text(text, encoding="utf8", newline="\n")


def tar_folder(folder, archive):
    """A .tar.gz with plain permissions: 755 for folders and executables."""
    def clean(info):
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        executable = info.isdir() or info.name.endswith(f"/{APP_ID}")
        info.mode = 0o755 if executable else 0o644
        return info
    with tarfile.open(archive, "w:gz", format=tarfile.PAX_FORMAT) as tar:
        tar.add(folder, arcname=folder.name, filter=clean)


def linux(args, base, license_folder, work, release):
    exe = Path(args.exe)
    # Installed layout: bin/../share/quran-mushaf-reader/Resources, found by
    # resource_root() in main.rs. Used by both the AppImage and the .deb.
    root = work / "AppDir"
    usr = root / "usr"
    executable(exe, usr / "bin" / APP_ID)
    share = usr / "share" / APP_ID
    share.mkdir(parents=True)
    copy_tree(base / "Resources", share / "Resources")
    shutil.copyfile(base / "resource-manifest.json", share / "resource-manifest.json")
    copy_tree(license_folder, share / "licenses")
    start_here(args.version, share / "START-HERE.md")
    shutil.copyfile(HERE / "verify_resources.py", share / "verify_resources.py")
    (usr / "share/applications").mkdir(parents=True)
    (usr / f"share/applications/{APP_ID}.desktop").write_text(DESKTOP, encoding="utf8", newline="\n")
    icons = usr / "share/icons/hicolor/256x256/apps"
    icons.mkdir(parents=True)
    shutil.copyfile(READER / "assets/app.png", icons / f"{APP_ID}.png")
    doc = usr / "share/doc" / APP_ID
    doc.mkdir(parents=True)
    (doc / "copyright").write_text(
        f"{APP_NAME}\nSource: {HOMEPAGE}\n\nLicense: see the LICENSE file in the source and\n"
        f"/usr/share/{APP_ID}/licenses for the libraries and Quran resources.\n",
        encoding="utf8", newline="\n")
    verify(share, MANIFEST_SHA256)

    # Portable .tar.gz: the executable with Resources beside it.
    portable = work / f"Quran-Mushaf-Reader-{args.version}-linux-x86_64"
    executable(exe, portable / APP_ID)
    copy_tree(base / "Resources", portable / "Resources")
    shutil.copyfile(base / "resource-manifest.json", portable / "resource-manifest.json")
    copy_tree(license_folder, portable / "licenses")
    start_here(args.version, portable / "START-HERE.md")
    shutil.copyfile(HERE / "verify_resources.py", portable / "verify_resources.py")
    (portable / f"{APP_ID}.desktop").write_text(DESKTOP, encoding="utf8", newline="\n")
    shutil.copyfile(READER / "assets/app.png", portable / f"{APP_ID}.png")
    verify(portable, MANIFEST_SHA256)
    tar_folder(portable, release / f"QuranMushafReader-{args.version}-linux-x86_64.tar.gz")
    if args.layout_only:
        return

    # Only system libraries the packages name as dependencies may be linked.
    needed = subprocess.run(["readelf", "-d", exe], check=True, capture_output=True,
                            text=True).stdout
    names = {line.split("[")[1].split("]")[0] for line in needed.splitlines() if "(NEEDED)" in line}
    print("Linked libraries:", ", ".join(sorted(names)))
    if names - LINKED:
        raise SystemExit(f"Unexpected linked libraries: {sorted(names - LINKED)}")

    # .deb
    deb = work / "deb"
    copy_tree(usr, deb / "usr")
    (deb / "DEBIAN").mkdir()
    size = sum(f.stat().st_size for f in (deb / "usr").rglob("*") if f.is_file()) // 1024
    (deb / "DEBIAN/control").write_text(
        f"Package: {APP_ID}\nVersion: {args.version}\nSection: education\nPriority: optional\n"
        f"Architecture: amd64\nInstalled-Size: {size}\nDepends: {DEB_DEPENDS}\n"
        f"Recommends: {DEB_RECOMMENDS}\nMaintainer: Thrpro <{HOMEPAGE}>\nHomepage: {HOMEPAGE}\n"
        f"Description: {DESCRIPTION}\n The Madani Mushaf drawn page by page, with page turns,\n"
        f" verse-by-verse recitation, tafsir and translation.\n", encoding="utf8", newline="\n")
    for path in [deb, *deb.rglob("*")]:
        if path.is_dir():
            path.chmod(0o755)
        elif path != deb / "usr/bin" / APP_ID:
            path.chmod(0o644)
    run("dpkg-deb", "--root-owner-group", "-Zxz", "--build", deb,
        release / f"{APP_ID}_{args.version}_amd64.deb")

    # AppImage: the same tree with an entry point, desktop entry and icon at its root.
    os.symlink(f"usr/bin/{APP_ID}", root / "AppRun")
    shutil.copyfile(usr / f"share/applications/{APP_ID}.desktop", root / f"{APP_ID}.desktop")
    shutil.copyfile(icons / f"{APP_ID}.png", root / f"{APP_ID}.png")
    os.symlink(f"{APP_ID}.png", root / ".DirIcon")
    for tool in (args.appimagetool, args.runtime):
        Path(tool).chmod(0o755)
    env = dict(os.environ, ARCH="x86_64", APPIMAGE_EXTRACT_AND_RUN="1", VERSION=args.version)
    run(args.appimagetool, "--no-appstream", "--runtime-file", args.runtime, root,
        release / f"QuranMushafReader-{args.version}-x86_64.AppImage", env=env)


def icns(png, target):
    """An .icns holding the 256-pixel PNG as 256x256 and as 128x128@2x."""
    data = png.read_bytes()
    entries = b"".join(kind + struct.pack(">I", 8 + len(data)) + data for kind in (b"ic08", b"ic13"))
    target.write_bytes(b"icns" + struct.pack(">I", 8 + len(entries)) + entries)


def macos(args, base, license_folder, work, release):
    stage = work / "dmg"
    app = stage / f"{APP_NAME}.app"
    contents = app / "Contents"
    executable(Path(args.exe), contents / "MacOS" / APP_ID)
    resources = contents / "Resources"
    # resource_root() in main.rs: Contents/MacOS/../Resources/Mushaf.
    copy_tree(base / "Resources", resources / "Mushaf")
    shutil.copyfile(base / "resource-manifest.json", resources / "resource-manifest.json")
    copy_tree(license_folder, resources / "licenses")
    start_here(args.version, resources / "START-HERE.md")
    icns(READER / "assets/app.png", resources / "AppIcon.icns")
    with open(contents / "Info.plist", "wb") as f:
        plistlib.dump({
            "CFBundleDevelopmentRegion": "en",
            "CFBundleDisplayName": APP_NAME,
            "CFBundleExecutable": APP_ID,
            "CFBundleIconFile": "AppIcon",
            "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": APP_NAME,
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": args.version,
            "CFBundleVersion": args.version,
            "LSApplicationCategoryType": "public.app-category.education",
            "LSMinimumSystemVersion": "11.0",
            "NSHighResolutionCapable": True,
            "NSHumanReadableCopyright": f"Source and licenses: {HOMEPAGE}",
        }, f)
    (contents / "PkgInfo").write_text("APPL????", encoding="ascii")
    verify(resources, MANIFEST_SHA256, resources=resources / "Mushaf")
    if args.layout_only:
        return

    identity = os.environ.get("MACOS_SIGN_IDENTITY", "")
    if identity:
        run("codesign", "--force", "--options", "runtime", "--timestamp", "--sign", identity, app)
    else:
        # Apple silicon runs only signed code; an ad hoc signature, not a Developer ID.
        run("codesign", "--force", "--sign", "-", app)
    run("codesign", "--verify", "--strict", "--verbose=2", app)
    os.symlink("/Applications", stage / "Applications")
    dmg = release / f"QuranMushafReader-{args.version}-macos-arm64.dmg"
    run("hdiutil", "create", "-volname", APP_NAME, "-srcfolder", stage, "-fs", "HFS+",
        "-format", "UDZO", dmg)
    if identity:
        run("codesign", "--force", "--timestamp", "--sign", identity, dmg)
    profile = os.environ.get("MACOS_NOTARY_PROFILE", "")
    if identity and profile:
        run("xcrun", "notarytool", "submit", dmg, "--keychain-profile", profile, "--wait")
        run("xcrun", "stapler", "staple", dmg)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--target", choices=["linux", "macos"], required=True)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--metadata", required=True)
    parser.add_argument("--resource-zip", default=RESOURCE_ZIP)
    parser.add_argument("--appimagetool")
    parser.add_argument("--runtime")
    parser.add_argument("--layout-only", action="store_true")
    args = parser.parse_args()
    if not all(p.isdigit() for p in args.version.split(".")) or args.version.count(".") != 2:
        raise SystemExit(f"Version must look like 1.2.3, not {args.version!r}")
    if args.target == "linux" and not args.layout_only and not (args.appimagetool and args.runtime):
        raise SystemExit("The AppImage needs --appimagetool and --runtime")
    out = Path(args.out).resolve()
    # Never overwrite an earlier package.
    out.mkdir(parents=True, exist_ok=False)
    release = out / "release"
    release.mkdir()
    work = out / "work"
    work.mkdir()
    base = resource_base(work, args.resource_zip)
    license_folder = licenses(base, work, args.metadata)
    (linux if args.target == "linux" else macos)(args, base, license_folder, work, release)
    sums = [f"{sha256(f)}  {f.name}" for f in sorted(release.iterdir()) if f.is_file()]
    # Named by system, as all three systems' files share one GitHub release.
    text = "".join(line + "\n" for line in sums)
    (release / f"SHA256SUMS-{args.target}.txt").write_text(text, encoding="utf8", newline="\n")
    for line in sums:
        print(line)


if __name__ == "__main__":
    main()
