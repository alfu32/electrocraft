"""Unit tests for the release helpers: python3 -m unittest discover -s scripts/release -p 'test_*.py' -v"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


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
