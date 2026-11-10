#!/usr/bin/env python3
"""Portable archive: electrocraft-X.Y.Z-<target>.tar.gz (or .zip on Windows)
with both binaries, licences, the desktop entry and icon, and a file manifest."""

import argparse
import hashlib
import json
import platform
import shutil
import sys
import tarfile
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

ROOT = common.ROOT


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--target", required=True, help="e.g. x86_64-unknown-linux-gnu")
    ap.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    ap.add_argument("--output", type=Path, default=ROOT / "dist")
    a = ap.parse_args()
    version = common.app_version()
    exe = ".exe" if "windows" in a.target else ""
    stem = f"electrocraft-{version}-{a.target}"
    stage = ROOT / "target/portable" / stem
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    for b in ("electrocraft", "electrocraft-cli", "electrocraft-datasheet"):
        shutil.copy2(a.bin_dir / (b + exe), stage / (b + exe))
    for f in ("README.md", "LICENSE-MIT", "LICENSE-APACHE"):
        if (ROOT / f).is_file():
            shutil.copy2(ROOT / f, stage / f)
    (stage / "licenses").mkdir()
    for f in (ROOT / "assets/fonts").glob("*LICENSE*"):
        shutil.copy2(f, stage / "licenses" / f.name)
    if "linux" in a.target:
        (stage / "electrocraft.desktop").write_text(common.render_desktop(version))
        shutil.copy2(ROOT / "packaging/electrocraft.png", stage / "electrocraft.png")
    manifest = {
        "name": "electrocraft",
        "version": version,
        "target": a.target,
        "files": {
            str(p.relative_to(stage)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(stage.rglob("*")) if p.is_file()
        },
    }
    (stage / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")
    a.output.mkdir(parents=True, exist_ok=True)
    if exe:
        out = a.output / f"{stem}.zip"
        with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
            for p in sorted(stage.rglob("*")):
                z.write(p, Path(stem) / p.relative_to(stage))
    else:
        out = a.output / f"{stem}.tar.gz"
        with tarfile.open(out, "w:gz") as t:
            t.add(stage, arcname=stem)
    (out.parent / (out.name + ".sha256")).write_text(f"{common.sha256(out)}  {out.name}\n")
    print(out)


if __name__ == "__main__":
    main()
