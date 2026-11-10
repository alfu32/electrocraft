#!/usr/bin/env python3
"""Pack a strict-confinement Snap (core24 base, mesa-2404 GPU content), as
Meshcraft does: electrocraft_<version>_<arch>.snap plus a .sha256 sidecar.

The binaries load their windowing libraries at run time (dlopen). core24
provides libc, dbus, ffi and xkbcommon; mesa-2404 provides OpenGL/EGL, libX11,
libxcb and the Wayland client libraries. Everything else they load is copied
from the build host into $SNAP/usr/lib with its own dependencies, together
with the XKB keyboard data. Requires `snap` (snapd) and mksquashfs.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

ROOT = common.ROOT
BINARIES = ("electrocraft", "electrocraft-cli", "electrocraft-datasheet")

# Loaded at run time by winit/egui (X11 path) and not in core24 or mesa-2404.
BUNDLE = (
    "libXcursor.so.1",
    "libXrandr.so.2",
    "libXi.so.6",
    "libXinerama.so.1",
    "libXrender.so.1",
    "libxkbcommon-x11.so.0",
    "libxcb-xkb.so.1",
)

# Provided inside the snap's runtime: the core24 base or the mesa-2404 GPU content.
PROVIDED = {
    # core24
    "linux-vdso.so.1", "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1", "libc.so.6", "libm.so.6",
    "libdl.so.2", "libpthread.so.0", "librt.so.1", "libgcc_s.so.1", "libstdc++.so.6",
    "libdbus-1.so.3", "libffi.so.8", "libxkbcommon.so.0", "libsystemd.so.0", "libbsd.so.0",
    "libmd.so.0", "libz.so.1", "liblzma.so.5", "libzstd.so.1", "liblz4.so.1", "libcap.so.2",
    "libgcrypt.so.20", "libgpg-error.so.0", "libexpat.so.1",
    # mesa-2404
    "libX11.so.6", "libX11-xcb.so.1", "libXau.so.6", "libXdmcp.so.6", "libXext.so.6",
    "libXfixes.so.3", "libXxf86vm.so.1", "libxcb.so.1", "libxcb-dri2.so.0", "libxcb-dri3.so.0",
    "libxcb-glx.so.0", "libxcb-present.so.0", "libxcb-randr.so.0", "libxcb-shm.so.0",
    "libxcb-sync.so.1", "libxcb-xfixes.so.0", "libwayland-client.so.0", "libwayland-cursor.so.0",
    "libwayland-egl.so.1", "libwayland-server.so.0", "libGL.so.1", "libEGL.so.1", "libGLX.so.0",
    "libGLdispatch.so.0", "libOpenGL.so.0", "libGLESv2.so.2", "libgbm.so.1", "libdrm.so.2",
}

SNAP_YAML = """name: electrocraft
title: Electrocraft
version: '{version}'
summary: Draw electronic schematics and simulate them live
description: |
  Place parts, wire them, and watch the circuit run: voltage-coloured wires,
  moving current dots, glowing LEDs, live switches and an oscilloscope.
  Analog and digital parts, DC/transient/AC analyses, SPICE model import,
  components in JavaScript or as nested schematics, and a datasheet
  model-fitting tool.
base: core24
architectures: [{arch}]
plugs:
  gpu-2404:
    interface: content
    content: gpu-2404
    default-provider: mesa-2404
    target: $SNAP/gpu-2404
confinement: strict
grade: stable
license: MIT OR Apache-2.0
apps:
  electrocraft:
    command: bin/electrocraft
    common-id: io.github.alfu32.electrocraft
    # network and password-manager-service: online part catalogs (off until
    # the user enables them) and their keys in the system keychain.
    plugs: [home, removable-media, desktop, desktop-legacy, wayland, x11, opengl, gpu-2404, network, password-manager-service]
  cli:
    command: bin/electrocraft-cli
    plugs: [home, removable-media]
  datasheet:
    command: bin/electrocraft-datasheet
    plugs: [home, removable-media, desktop, desktop-legacy, wayland, x11, opengl, gpu-2404]
"""

LAUNCHER = """#!/bin/sh
# Settings and recent files live in the snap's own folder.
export XDG_CONFIG_HOME="$SNAP_USER_COMMON/.config"
export LD_LIBRARY_PATH="$SNAP/usr/lib${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}"
if [ -d "$SNAP/usr/share/X11/xkb" ]; then
  export XKB_CONFIG_ROOT="$SNAP/usr/share/X11/xkb"
fi
exec "$SNAP/gpu-2404/bin/gpu-2404-provider-wrapper" "$SNAP/opt/electrocraft/{binary}" "$@"
"""


def snap_arch(arch: str) -> str:
    return {"x86_64": "amd64", "amd64": "amd64", "aarch64": "arm64", "arm64": "arm64"}[arch]


def snap_name(version: str, arch: str) -> str:
    return f"electrocraft_{version}_{snap_arch(arch)}.snap"


def host_library(soname: str) -> Path | None:
    """Path of a library on the build host (ldconfig cache)."""
    out = subprocess.run(["ldconfig", "-p"], capture_output=True, text=True, check=True).stdout
    machine = "x86-64" if common.host_arch() == "x86_64" else "AArch64"
    for line in out.splitlines():
        m = re.match(r"\s*(\S+) \(([^)]*)\) => (\S+)", line)
        if m and m.group(1) == soname and machine in m.group(2):
            return Path(m.group(3))
    return None


def needed(lib: Path) -> list[str]:
    """Direct and indirect dependencies (sonames) of a library."""
    out = subprocess.run(["ldd", str(lib)], capture_output=True, text=True, check=True).stdout
    return [line.split()[0] for line in out.splitlines() if line.strip() and "=>" in line]


def library_closure(roots=BUNDLE, resolve=host_library, deps=needed) -> dict[str, Path]:
    """Every library to copy: the roots and their dependencies not provided."""
    todo, out = list(roots), {}
    while todo:
        name = todo.pop()
        if name in out or name in PROVIDED:
            continue
        path = resolve(name)
        if path is None:
            raise SystemExit(f"{name} is not installed on the build host")
        out[name] = path
        todo.extend(d for d in deps(path) if d not in PROVIDED)
    return out


def stage(root: Path, version: str, arch: str, bin_dir: Path, libs: dict[str, Path], xkb: Path | None):
    (root / "opt/electrocraft").mkdir(parents=True)
    for b in BINARIES:
        shutil.copy2(bin_dir / b, root / "opt/electrocraft" / b)
    (root / "bin").mkdir()
    for b in BINARIES:
        launcher = root / "bin" / b
        launcher.write_text(LAUNCHER.format(binary=b))
        launcher.chmod(0o755)
    (root / "gpu-2404").mkdir()
    (root / "usr/lib").mkdir(parents=True)
    for name, path in sorted(libs.items()):
        shutil.copy2(path.resolve(), root / "usr/lib" / name)
    if xkb is not None and xkb.is_dir():
        shutil.copytree(xkb, root / "usr/share/X11/xkb", symlinks=True)
    (root / "meta/gui").mkdir(parents=True)
    shutil.copy2(ROOT / "packaging/electrocraft.png", root / "meta/gui/electrocraft.png")
    desktop = common.render_desktop(version)
    desktop = desktop.replace("Icon=electrocraft", "Icon=${SNAP}/meta/gui/electrocraft.png")
    desktop = "\n".join(l for l in desktop.splitlines() if not l.startswith("X-AppImage-")) + "\n"
    (root / "meta/gui/electrocraft.desktop").write_text(desktop)
    (root / "meta/snap.yaml").write_text(SNAP_YAML.format(version=version, arch=snap_arch(arch)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    ap.add_argument("--arch", default=common.host_arch())
    ap.add_argument("--output", type=Path, default=ROOT / "dist")
    ap.add_argument("--xkb", type=Path, default=Path("/usr/share/X11/xkb"))
    a = ap.parse_args()
    version = common.app_version()
    root = ROOT / "target/snap/prime"
    if root.exists():
        shutil.rmtree(root)
    stage(root, version, a.arch, a.bin_dir, library_closure(), a.xkb)
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "1767225600"))
    for p in sorted(root.rglob("*"), reverse=True):
        os.utime(p, (epoch, epoch), follow_symlinks=False)
    a.output.mkdir(parents=True, exist_ok=True)
    subprocess.run(["snap", "pack", str(root), str(a.output.resolve()), "--compression=xz"], check=True)
    artifact = a.output / snap_name(version, a.arch)
    if not artifact.is_file():
        raise SystemExit(f"{artifact} was not produced")
    (artifact.parent / (artifact.name + ".sha256")).write_text(f"{common.sha256(artifact)}  {artifact.name}\n")
    print(artifact)


if __name__ == "__main__":
    main()
