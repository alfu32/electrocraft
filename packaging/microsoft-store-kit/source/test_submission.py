"""Check that asset references resolve relative to the selected import folder."""
import csv
from pathlib import Path
import tempfile
import unittest
import build_submission


class DirectoryImportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.folder = Path(self.temp.name) / 'microsoft-store'
        self.folder.mkdir()
        self.template = [
            ['Field', 'ID', 'Type (Type)', 'default', 'en-us'],
            ['DesktopScreenshot1', '100', 'Relative path (or URL to file in Partner Center)', '', ''],
        ]
        self.csv = self.folder / 'listingData.csv'
        (self.folder / 'shot.png').write_bytes(b'\x89PNG\r\n\x1a\n')
        self.write_reference('microsoft-store/shot.png')

    def write_reference(self, value, default=False):
        with self.csv.open('w', encoding='utf-8-sig', newline='') as stream:
            writer = csv.writer(stream)
            writer.writerow(self.template[0])
            writer.writerow(self.template[1][:3] + ([value, ''] if default else ['', value]))

    def check(self):
        return build_submission.verify_import(self.folder, self.template, ['en-us'])

    def test_folder_relative_path_and_default_column(self):
        for default in (False, True):
            with self.subTest(default=default):
                self.write_reference('microsoft-store/shot.png', default=default)
                result = self.check()
                self.assertEqual(result['asset_references'], 1)
                self.assertIn('microsoft-store/shot.png', result['relative_upload_paths'])

    def test_rejects_wrong_root_missing_file_and_normalized_paths(self):
        for ref in ['./shot.png', 'microsoft-store/assets/shot.png', 'other-folder/shot.png',
                    'shot.png', './missing.png']:
            with self.subTest(reference=ref):
                self.write_reference(ref)
                with self.assertRaisesRegex(AssertionError, 'absent from uploaded file list'):
                    self.check()

    def test_parent_kit_with_extra_csv_is_not_an_import(self):
        (self.folder / 'source.csv').write_text('original export')
        with self.assertRaisesRegex(AssertionError, 'exactly one CSV'):
            self.check()

    def test_nested_assets_are_rejected(self):
        (self.folder / 'assets').mkdir()
        with self.assertRaisesRegex(AssertionError, 'must be flat'):
            self.check()

    def test_rejects_screenshot_slots_above_ten_including_default(self):
        self.template.append([
            'DesktopScreenshot11', '110',
            'Relative path (or URL to file in Partner Center)', '', '',
        ])
        for default in (False, True):
            with self.subTest(default=default):
                self.write_reference('microsoft-store/shot.png')
                with self.csv.open('a', encoding='utf-8', newline='') as stream:
                    csv.writer(stream).writerow(
                        self.template[-1][:3] +
                        (['microsoft-store/shot.png', ''] if default else
                         ['', 'microsoft-store/shot.png'])
                    )
                with self.assertRaisesRegex(AssertionError, 'at most 10'):
                    self.check()

    def test_unreferenced_files_are_not_uploaded(self):
        (self.folder / 'extra.png').write_bytes(b'\x89PNG\r\n\x1a\n')
        with self.assertRaisesRegex(AssertionError, 'Unreferenced upload assets'):
            self.check()


if __name__ == '__main__':
    unittest.main()
