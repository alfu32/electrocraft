"""Shared release helpers (standard library only)."""

import hashlib
import platform
import re
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP = "electrocraft"
REPO = "alfu32/electrocraft"
ARCH_ALIASES = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}


def app_version() -> str:
    """The app version: the literal `version` in crates/ecs-app/Cargo.toml."""
    text = (ROOT / "crates/ecs-app/Cargo.toml").read_text()
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        raise SystemExit("no literal version in crates/ecs-app/Cargo.toml")
    return m.group(1)


def host_arch() -> str:
    return ARCH_ALIASES.get(platform.machine().lower(), platform.machine().lower())


def appimage_name(version: str, arch: str) -> str:
    """AppGlider's first-preference asset name: <app>-<version>-linux-<arch>.AppImage."""
    return f"{APP}-{version}-linux-{arch}.AppImage"


def update_information(arch: str) -> str:
    """Embedded AppImage update information (zsync via GitHub releases)."""
    owner, repo = REPO.split("/")
    return f"gh-releases-zsync|{owner}|{repo}|latest|{APP}-*-linux-{arch}.AppImage.zsync"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def download(url: str, digest: str, dest: Path) -> Path:
    """Download once into `dest` and verify the pinned SHA-256."""
    if dest.is_file() and sha256(dest) == digest:
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(url, timeout=120) as r, tmp.open("wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    got = sha256(tmp)
    if got != digest:
        tmp.unlink()
        raise SystemExit(f"checksum mismatch for {url}: {got}")
    tmp.rename(dest)
    return dest


def render_desktop(version: str) -> str:
    text = (ROOT / "packaging/linux/electrocraft.desktop").read_text()
    return text.replace("@VERSION@", version)


def parse_desktop(text: str) -> dict:
    """Parse a desktop file into {group: {key: value}}."""
    groups, current = {}, None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = groups.setdefault(line[1:-1], {})
        elif "=" in line and current is not None:
            k, v = line.split("=", 1)
            current[k.strip()] = v.strip()
    return groups


# Keys AppGlider reads from an AppImage's embedded desktop entry
# (appglider/install/desktopentry.py) plus the identity it extracts.
APPGLIDER_KEYS = ("Name", "Icon", "X-AppImage-Version", "Comment", "Categories", "MimeType",
                  "Keywords", "StartupWMClass", "StartupNotify", "Terminal", "Exec", "Actions")


def check_appglider_desktop(groups: dict, version: str) -> list:
    """Problems that would make AppGlider show less than it can."""
    entry = groups.get("Desktop Entry", {})
    problems = [f"missing {k}" for k in APPGLIDER_KEYS if not entry.get(k)]
    if entry.get("X-AppImage-Version") not in (None, version):
        problems.append(f"X-AppImage-Version {entry.get('X-AppImage-Version')} != {version}")
    if entry.get("Icon") != APP:
        problems.append("Icon must be the bare name 'electrocraft' (AppGlider looks up <Icon>.png)")
    if not re.fullmatch(r"[A-Za-z0-9._+\-]{1,100}", entry.get("StartupWMClass", "")):
        problems.append("StartupWMClass has characters AppGlider rejects")
    for action in filter(None, entry.get("Actions", "").split(";")):
        g = groups.get(f"Desktop Action {action}", {})
        if not g.get("Name") or not g.get("Exec"):
            problems.append(f"action {action} needs Name and Exec")
    return problems
