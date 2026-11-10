#!/usr/bin/env python3
"""Pack a strict-confinement Snap (core24, mesa-2404 GPU content) from the
release binary, as Meshcraft does. Requires `snap` (snapd) for `snap pack`."""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

ROOT = common.ROOT
SNAP_YAML = """name: electrocraft
title: Electrocraft
version: '{version}'
summary: Draw electronic schematics and simulate them live
description: |
  Place parts, wire them, and watch the circuit run: voltage-coloured wires,
  moving current dots, glowing LEDs, live switches and scopes.
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
    plugs: [home, removable-media, desktop, desktop-legacy, wayland, x11, opengl, gpu-2404]
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", type=Path, default=ROOT / "target/release/electrocraft")
    ap.add_argument("--arch", default={"x86_64": "amd64", "aarch64": "arm64"}[common.host_arch()])
    ap.add_argument("--output", type=Path, default=ROOT / "dist")
    a = ap.parse_args()
    version = common.app_version()
    root = ROOT / "target/snap/prime"
    if root.exists():
        shutil.rmtree(root)
    (root / "opt/electrocraft").mkdir(parents=True)
    shutil.copy2(a.binary, root / "opt/electrocraft/electrocraft")
    (root / "bin").mkdir()
    launcher = root / "bin/electrocraft"
    launcher.write_text(
        '#!/bin/sh\nexport XDG_CONFIG_HOME="$SNAP_USER_COMMON/.config"\n'
        'exec "$SNAP/gpu-2404/bin/gpu-2404-provider-wrapper" "$SNAP/opt/electrocraft/electrocraft" "$@"\n'
    )
    launcher.chmod(0o755)
    (root / "meta/gui").mkdir(parents=True)
    shutil.copy2(ROOT / "packaging/electrocraft.png", root / "meta/gui/electrocraft.png")
    desktop = common.render_desktop(version).replace("Icon=electrocraft", "Icon=${SNAP}/meta/gui/electrocraft.png")
    (root / "meta/gui/electrocraft.desktop").write_text(desktop)
    (root / "meta/snap.yaml").write_text(SNAP_YAML.format(version=version, arch=a.arch))
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "1767225600"))
    for p in sorted(root.rglob("*"), reverse=True):
        os.utime(p, (epoch, epoch))
    a.output.mkdir(parents=True, exist_ok=True)
    subprocess.run(["snap", "pack", str(root), str(a.output.resolve()), "--compression=xz"], check=True)


if __name__ == "__main__":
    main()
