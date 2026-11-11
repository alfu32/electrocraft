# Microsoft Store submission — Electrocraft

Store ID: **9MVQ93DQWRWF**. Publisher display name: **K3D0**.
Package identity: `K3D0.electrocraft`.
App version: **0.1.13**; MSIX package version: **1.1.13.0**.

## Import the listing

1. In Partner Center, open this product and the submission draft associated with export `listingData-9MVQ93DQWRWF-1152921505702360449.csv`.
2. Choose **Store listings → Import listings → Import folder**.
3. Select the repository's **`packaging/microsoft-store`** folder. It contains exactly one CSV and the 34 referenced PNGs directly beside it. Do not select `packaging`, `microsoft-store-kit`, a ZIP file, the CSV alone or the old `previous-imports` folders. The CSV references assets as `microsoft-store/filename.png`, including the selected folder’s name.
4. If Partner Center reports its ten-desktop-screenshot limit, select the sibling **`packaging/microsoft-store-10`** instead. Its CSV and fourteen referenced PNGs are likewise directly beside each other. All 37 original screenshots are archived separately under `microsoft-store-kit/documentation/screenshots/`; the import folders contain only their referenced assets.
5. Check the import error report. Each imported locale needs a description, a reserved product name and at least one screenshot. Verify right-to-left text and any translated drafts in Partner Center's preview.
6. Upload both released MSIX packages from `packages/` in the **Packages** section, not in the listing import. The manifest identifies Windows Desktop, x64/ARM64, English resources, and minimum Windows build 17763. Changing listing languages does not localize the application or its package resources.
7. Complete the account-level sections below, save each section, then submit for certification when ready.

This package prepares local artifacts. It does not update Partner Center or submit the app.

## Fields prepared in the CSV

All 38 exported locales include a description, short description, title, developer name, desktop screenshots, app icon, square art, poster art and promotional hero art. The two English listings also include the existing features, search terms, system requirements and captions. The CSV preserves the export’s column order, Field/ID/Type metadata and populated settings. Asset paths include the selected folder name and the PNG filename.

ReleaseNotes is deliberately empty for an initial Store submission, as Microsoft instructs. A separate update-notes document is provided for a later update. AdditionalLicenseTerms is empty: this package does not invent an EULA. Copyright/trademark ownership statements and the VoiceTitle field are also left empty. Mobile, Xbox, Holographic, Surface Hub-specific, trailer, legacy promotional-image and unused feature rows remain empty because they are not applicable or no such asset was supplied. The existing app name remains Electrocraft.

## Partner Center fields outside this CSV

- **Category:** suggested Education, with an appropriate science/electronics subcategory if offered. Confirm the category in the portal.
- **Website:** https://github.com/alfu32/electrocraft
- **Support:** https://github.com/alfu32/electrocraft/issues
- **Privacy-policy URL:** publish `documentation/privacy.html` at a public HTTPS URL, then enter that real URL. A local file is not a privacy URL. The repository URL for a document becomes valid only after that document is pushed and publicly accessible.
- **Age ratings:** complete the IARC questionnaire based on the app. This is an electronics design tool; no rating is fabricated here.
- **Pricing/markets/publishing date:** choose in Partner Center; the listing CSV cannot set them.
- **Restricted capability:** `runFullTrust`; copy the capability rationale from `CERTIFICATION.md`.
- **License/rights:** verify publication rights to the app and assets and resolve the license-text gap described in `LICENSES.md`.

The interface and the supplied screenshots are in English. Describe any additional languages as listing translations, not as translated app interfaces. Review machine-drafted copy with a speaker of the target language before publication.

## Official references checked 2026-11-10

- [Folder imports and required fields](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/import-and-export-store-listings)
- [Screenshot counts, dimensions, icon sizes and hero-art rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/screenshots-and-images)
- [Description, features, short descriptions and release notes](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/add-and-edit-store-listing-info)
- [Keywords and supplemental information](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/add-additional-information)
- [Supported language codes](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#supported-languages)
