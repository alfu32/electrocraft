#!/usr/bin/env python3
"""Build an AppGlider-friendly AppImage from a release binary.

    cargo build --release --locked -p ecs-app -p ecs-cli
    python3 scripts/release/appimage.py            # -> dist/electrocraft-X.Y.Z-linux-<arch>.AppImage

The appimagetool and type 2 runtime are pinned by SHA-256 in
packaging/appimage-inputs.json (the same pins Meshcraft uses). appimagetool runs
with APPIMAGE_EXTRACT_AND_RUN=1, so FUSE is not required.
"""

import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

ROOT = common.ROOT


def tools(arch: str, cache: Path):
    pins = json.loads((ROOT / "packaging/appimage-inputs.json").read_text())
    # appimagetool runs on the build host; the runtime matches the target.
    selected = pins.get(arch, pins) if arch != "x86_64" else pins
    host = common.host_arch()
    host_pins = pins.get(host, pins) if host != "x86_64" else pins
    tool = common.download(host_pins["tool"]["url"], host_pins["tool"]["sha256"], cache / f"appimagetool-{host}.AppImage")
    tool.chmod(tool.stat().st_mode | stat.S_IEXEC)
    runtime = common.download(selected["runtime"]["url"], selected["runtime"]["sha256"], cache / f"runtime-{arch}")
    return tool, runtime


def build_appdir(appdir: Path, binary: Path, cli: Path | None, version: str):
    if appdir.exists():
        shutil.rmtree(appdir)
    usr = appdir / "usr"
    (usr / "bin").mkdir(parents=True)
    shutil.copy2(binary, usr / "bin/electrocraft")
    if cli and cli.is_file():
        shutil.copy2(cli, usr / "bin/electrocraft-cli")
    datasheet = binary.parent / "electrocraft-datasheet"
    if datasheet.is_file():
        shutil.copy2(datasheet, usr / "bin/electrocraft-datasheet")
    desktop = common.render_desktop(version)
    for d in (appdir, usr / "share/applications"):
        d.mkdir(parents=True, exist_ok=True)
        (d / "electrocraft.desktop").write_text(desktop)
    icon = ROOT / "packaging/electrocraft.png"
    shutil.copy2(icon, appdir / "electrocraft.png")
    shutil.copy2(icon, appdir / ".DirIcon")
    hicolor = usr / "share/icons/hicolor/512x512/apps"
    hicolor.mkdir(parents=True)
    shutil.copy2(icon, hicolor / "electrocraft.png")
    (usr / "share/metainfo").mkdir(parents=True)
    shutil.copy2(ROOT / "packaging/linux/io.github.alfu32.electrocraft.metainfo.xml",
                 usr / "share/metainfo/io.github.alfu32.electrocraft.metainfo.xml")
    (usr / "share/mime/packages").mkdir(parents=True)
    shutil.copy2(ROOT / "packaging/linux/electrocraft-mime.xml", usr / "share/mime/packages/electrocraft.xml")
    doc = usr / "share/doc/electrocraft"
    doc.mkdir(parents=True)
    for name in ("LICENSE-MIT", "LICENSE-APACHE", "README.md"):
        if (ROOT / name).is_file():
            shutil.copy2(ROOT / name, doc / name)
    for name in ("Inter-LICENSE.txt", "NotoSansMono-LICENSE.txt"):
        if (ROOT / "assets/fonts" / name).is_file():
            shutil.copy2(ROOT / "assets/fonts" / name, doc / name)
    shutil.copytree(ROOT / "packaging/licenses/appimage-runtime", doc / "appimage-runtime")
    shutil.copy2(ROOT / "packaging/linux/AppRun", appdir / "AppRun")
    (appdir / "AppRun").chmod(0o755)


def verify(image: Path, version: str, arch: str):
    """Check what AppGlider will read from the finished AppImage."""
    problems = []
    if image.name != common.appimage_name(version, arch):
        problems.append(f"file name {image.name} is not AppGlider's preferred pattern")
    with tempfile.TemporaryDirectory() as tmp:
        env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1")
        for pattern in ("electrocraft.desktop", "electrocraft.png", ".DirIcon"):
            subprocess.run([str(image.resolve()), "--appimage-extract", pattern], cwd=tmp, env=env,
                           check=True, stdout=subprocess.DEVNULL)
        root = Path(tmp) / "squashfs-root"
        groups = common.parse_desktop((root / "electrocraft.desktop").read_text())
        problems += common.check_appglider_desktop(groups, version)
        for f in ("electrocraft.png", ".DirIcon"):
            if not (root / f).exists():
                problems.append(f"{f} missing at the AppImage root")
    if problems:
        raise SystemExit("AppImage check failed:\n  " + "\n  ".join(problems))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--binary", type=Path, default=ROOT / "target/release/electrocraft")
    ap.add_argument("--cli", type=Path, default=ROOT / "target/release/electrocraft-cli")
    ap.add_argument("--arch", default=common.host_arch())
    ap.add_argument("--output", type=Path, default=ROOT / "dist")
    ap.add_argument("--no-update-info", action="store_true", help="do not embed zsync update information")
    a = ap.parse_args()
    version = common.app_version()
    if not a.binary.is_file():
        raise SystemExit(f"{a.binary} not found; run cargo build --release first")
    tool, runtime = tools(a.arch, ROOT / "target/appimage-tools")
    appdir = ROOT / "target/appimage/Electrocraft.AppDir"
    build_appdir(appdir, a.binary, a.cli, version)
    a.output.mkdir(parents=True, exist_ok=True)
    image = a.output / common.appimage_name(version, a.arch)
    cmd = [str(tool), "--no-appstream", "--runtime-file", str(runtime)]
    if not a.no_update_info:
        cmd += ["-u", common.update_information(a.arch)]
    cmd += [str(appdir), str(image)]
    env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1", ARCH=a.arch, VERSION=version)
    subprocess.run(cmd, check=True, env=env, cwd=a.output)
    verify(image, version, a.arch)
    for f in [image, *a.output.glob(image.name + ".zsync")]:
        (f.parent / (f.name + ".sha256")).write_text(f"{common.sha256(f)}  {f.name}\n")
    print(image)


if __name__ == "__main__":
    main()
