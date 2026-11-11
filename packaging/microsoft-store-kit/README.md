# Electrocraft Store listing support kit

**Import `/home/alfu64/Development/electrocraft/packaging/microsoft-store` in Partner Center.**

That folder contains one CSV and its 34 referenced PNGs directly beside it: thirty screenshots and four artwork files. It contains no nested folders, supporting documents, installers or additional CSV files. CSV references use `microsoft-store/filename.png`, including the selected folder’s name.

This `microsoft-store-kit` folder holds supporting material and is **not an import folder**. The earlier import layout is retained under `previous-imports/` for reference only; do not upload it.

If Partner Center enforces the documented ten-desktop-screenshot limit, the separate sibling `packaging/microsoft-store-10` has one CSV and fourteen referenced PNGs. It uses the same flat layout with `microsoft-store-10/filename.png` asset paths.

## Contents

- `documentation/`: submission steps, privacy HTML, certification notes, quick start, source manual, font notices, and a gallery/archive of all 37 screenshots.
- `source/`: original exported CSV, listing text, screenshot order and reproducible generation scripts.
- `artwork/`: editable SVGs and rendered PNGs.
- `packages/`: released x64/ARM64 MSIX files and verified checksums, uploaded separately in Partner Center's Packages section.
- `locales/` holds listing copy for all 38 locale columns in the latest export. The separate `en` and `en-us` columns share the English copy. All 38 columns share the existing PNGs.
- `validation-report.json`: local checks; not a claim that Partner Center has accepted the import.

## Rebuild and check

```sh
python3 packaging/microsoft-store-kit/source/render_artwork.py
python3 packaging/microsoft-store-kit/source/build_submission.py
python3 -m unittest discover -s packaging/microsoft-store-kit/source -p 'test_*.py'
node packaging/microsoft-store-kit/source/verify_browser_upload.mjs
```

The import check compares asset references against paths relative to the selected folder, checks there is exactly one CSV, rejects nested directories, and rejects unreferenced assets. The builder also validates field limits, PNG dimensions, source screenshot hashes and installer metadata. The separate browser check opens a disposable headless Chrome profile, selects each folder through a real directory input, reads the CSV, resolves each reference relative to the selected directory, and reads every PNG signature. It does not import into Partner Center. Dependencies: Python 3, Pillow and CairoSVG for artwork rendering.

See [SUBMISSION.md](documentation/SUBMISSION.md) for remaining privacy-URL, licensing and account-level submission work. No Partner Center import or publication is performed by these scripts.
