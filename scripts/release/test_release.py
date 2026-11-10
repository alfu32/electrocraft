"""Unit tests for the release helpers: python3 -m unittest discover -s scripts/release -p 'test_*.py' -v"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import snap  # noqa: E402
import msix  # noqa: E402
import xml.etree.ElementTree as ET  # noqa: E402


class MsixTests(unittest.TestCase):
    def test_store_identity_and_version_mapping(self):
        ident = msix.identity()
        self.assertEqual(ident["identityName"], "K3D0.electrocraft")
        self.assertTrue(ident["packageFamilyName"].startswith("K3D0.electrocraft_"))
        self.assertEqual(msix.package_version("0.1.6", ident), "1.1.6.0")
        self.assertEqual(msix.package_version("2.10.3", ident), "3.10.3.0")
        with self.assertRaises(ValueError):
            msix.package_version("1.2", ident)
        self.assertEqual(msix.msix_name("0.1.6", "x64"), "electrocraft-0.1.6-windows-x64.msix")

    def test_manifest_has_identity_full_trust_and_file_type(self):
        ns = {"m": msix.NS[""], "uap": msix.NS["uap"], "rescap": msix.NS["rescap"]}
        root = ET.fromstring(msix.manifest("0.1.6", "x64").split("\n", 1)[1])
        ident = root.find("m:Identity", ns)
        self.assertEqual(ident.get("Name"), "K3D0.electrocraft")
        self.assertEqual(ident.get("Publisher"), msix.identity()["publisher"])
        self.assertEqual(ident.get("Version"), "1.1.6.0")
        app = root.find("m:Applications/m:Application", ns)
        self.assertEqual((app.get("Executable"), app.get("EntryPoint")), ("electrocraft.exe", "Windows.FullTrustApplication"))
        self.assertEqual(root.find(".//uap:FileType", ns).text, ".ecs")
        self.assertEqual(root.find(".//rescap:Capability", ns).get("Name"), "runFullTrust")
        for logo in ("StoreLogo", "Square44x44Logo", "Square150x150Logo"):
            self.assertIn(f"Assets\\{logo}.png", msix.manifest("0.1.6", "x64"))
        arm = ET.fromstring(msix.manifest("0.1.6", "arm64").split("\n", 1)[1]).find("m:Identity", ns)
        self.assertEqual(arm.get("ProcessorArchitecture"), "arm64")
        self.assertEqual(msix.msix_name("0.1.6", "arm64"), "electrocraft-0.1.6-windows-arm64.msix")
        workflow = (common.ROOT / ".github/workflows/release.yaml").read_text()
        self.assertIn("windows-11-arm", workflow)
        self.assertIn("msix: arm64", workflow)
        package = (common.ROOT / "packaging/windows/package.ps1").read_text()
        for logo in ("StoreLogo", "Square44x44Logo", "Square150x150Logo"):
            self.assertIn(logo, package)


class SnapTests(unittest.TestCase):
    def test_snap_names_follow_snapcraft(self):
        self.assertEqual(snap.snap_name("1.2.3", "x86_64"), "electrocraft_1.2.3_amd64.snap")
        self.assertEqual(snap.snap_name("1.2.3", "aarch64"), "electrocraft_1.2.3_arm64.snap")

    def test_library_closure_skips_what_core24_and_mesa_provide(self):
        deps = {
            "libXcursor.so.1": ["libXrender.so.1", "libXfixes.so.3", "libX11.so.6", "libc.so.6"],
            "libXrender.so.1": ["libX11.so.6"],
            "libxkbcommon-x11.so.0": ["libxkbcommon.so.0", "libxcb.so.1", "libxcb-xkb.so.1"],
            "libxcb-xkb.so.1": ["libxcb.so.1", "libXau.so.6"],
            "libextra.so.2": [],
        }
        deps["libXi.so.6"] = ["libXext.so.6", "libextra.so.2"]
        got = snap.library_closure(
            roots=("libXcursor.so.1", "libxkbcommon-x11.so.0", "libXi.so.6"),
            resolve=lambda n: Path("/lib") / n,
            deps=lambda p: deps.get(p.name, []),
        )
        self.assertEqual(
            sorted(got),
            ["libXcursor.so.1", "libXi.so.6", "libXrender.so.1", "libextra.so.2", "libxcb-xkb.so.1", "libxkbcommon-x11.so.0"],
        )

    def test_staged_snap_is_strict_core24_with_gpu_content_and_three_apps(self):
        import tempfile
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            bins = t / "bin"
            bins.mkdir()
            for b in snap.BINARIES:
                (bins / b).write_text("#!/bin/sh\n")
            lib = t / "libXi.so.6"
            lib.write_text("")
            root = t / "prime"
            snap.stage(root, "1.2.3", "aarch64", bins, {"libXi.so.6": lib}, None)
            meta = (root / "meta/snap.yaml").read_text()
            for want in ("name: electrocraft", "version: '1.2.3'", "base: core24", "architectures: [arm64]",
                         "confinement: strict", "grade: stable", "default-provider: mesa-2404",
                         "command: bin/electrocraft\n", "command: bin/electrocraft-cli", "command: bin/electrocraft-datasheet"):
                self.assertIn(want, meta)
            launcher = (root / "bin/electrocraft").read_text()
            self.assertIn("gpu-2404-provider-wrapper", launcher)
            self.assertIn('"$SNAP/opt/electrocraft/electrocraft"', launcher)
            self.assertIn('LD_LIBRARY_PATH="$SNAP/usr/lib', launcher)
            self.assertTrue((root / "usr/lib/libXi.so.6").is_file())
            desktop = common.parse_desktop((root / "meta/gui/electrocraft.desktop").read_text())["Desktop Entry"]
            self.assertEqual(desktop["Icon"], "${SNAP}/meta/gui/electrocraft.png")
            self.assertTrue(desktop["Exec"].startswith("electrocraft"))
            self.assertNotIn("X-AppImage-Version", desktop)


class ReleaseTests(unittest.TestCase):
    def test_version_is_semver(self):
        v = common.app_version()
        self.assertRegex(v, r"^\d+\.\d+\.\d+$")

    def test_appimage_name_is_appglider_first_choice(self):
        # appglider/github/assets.py preferred_asset_names(): first pattern.
        self.assertEqual(common.appimage_name("1.2.3", "x86_64"), "electrocraft-1.2.3-linux-x86_64.AppImage")
        self.assertEqual(common.appimage_name("1.2.3", "aarch64"), "electrocraft-1.2.3-linux-aarch64.AppImage")

    def test_update_information(self):
        self.assertEqual(
            common.update_information("x86_64"),
            "gh-releases-zsync|alfu32|electrocraft|latest|electrocraft-*-linux-x86_64.AppImage.zsync",
        )

    def test_desktop_entry_has_everything_appglider_reads(self):
        v = common.app_version()
        groups = common.parse_desktop(common.render_desktop(v))
        self.assertEqual(common.check_appglider_desktop(groups, v), [])
        self.assertEqual(groups["Desktop Entry"]["X-AppImage-Version"], v)

    def test_startup_wm_class_matches_app_id(self):
        main = (common.ROOT / "crates/ecs-app/src/main.rs").read_text()
        groups = common.parse_desktop(common.render_desktop("0.0.0"))
        self.assertIn(f'pub const APP_ID: &str = "{groups["Desktop Entry"]["StartupWMClass"]}";', main)

    def test_repository_icon_is_discoverable(self):
        # AppGlider ranks files named after the project highest.
        self.assertTrue((common.ROOT / "packaging/electrocraft.png").is_file())
        self.assertTrue((common.ROOT / "assets/branding/electrocraft.svg").is_file())


if __name__ == "__main__":
    unittest.main()
