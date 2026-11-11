#!/usr/bin/env python3
"""Build and verify offline Partner Center folder imports. Requires Pillow.

Run from any working directory. The export sets the locale columns and order.
English copy is stored in the en and en-us files. The remaining copy is in
localized-copy.json.
"""
import csv
import hashlib
import html
import json
from pathlib import Path
import shutil
import xml.etree.ElementTree as ET
import zipfile
from PIL import Image

BASE = Path(__file__).resolve().parents[1]
REPO = BASE.parents[1]
SOURCE = BASE / 'source'
TEMPLATE = SOURCE / 'listingData-9MVQ93DQWRWF-1152921505702360449.csv'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def document(title, body):
    return f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title><style>
body{{font:16px/1.65 system-ui,sans-serif;max-width:1100px;margin:40px auto;padding:0 24px;background:#f8fafc;color:#172438}}
a{{color:#1655a0}}h1,h2{{line-height:1.2}}section{{margin:36px 0}}figure{{margin:25px 0;padding:20px;background:white;border:1px solid #d6deeb;border-radius:12px}}img{{max-width:100%;height:auto}}figcaption{{margin-top:12px}}code{{overflow-wrap:anywhere}}@media print{{figure{{break-inside:avoid}}body{{background:white}}}}
</style>{body}</html>'''


def local_records(locales):
    records = json.loads((SOURCE / 'localized-copy.json').read_text(encoding='utf-8'))
    for locale in ('en', 'en-us'):
        records[locale] = json.loads((SOURCE / f'{locale}.json').read_text(encoding='utf-8'))
    for locale in locales:
        assert locale in records, f'Missing listing copy for {locale}'
        record = records[locale]
        assert record['locale'] == locale
        for field in ('Title', 'Description', 'ShortDescription'):
            assert record.get(field), f'{locale}: missing {field}'
        assert record['Title'] == 'Electrocraft', 'Use the reserved product name'
    return [records[locale] for locale in locales]


def values(record, selection, captions, folder):
    fields = {k:record.get(k,'') for k in ['Description','ReleaseNotes','Title','ShortTitle','SortTitle','VoiceTitle','ShortDescription','DevStudio','CopyrightTrademarkInformation','AdditionalLicenseTerms']}
    for prefix, key in [('Feature','features'),('SearchTerm','keywords'),('MinimumHardwareReq','minimum_hardware'),('RecommendedHardwareReq','recommended_hardware')]:
        for i,text in enumerate(record.get(key,[]),1):
            fields[f'{prefix}{i}'] = text
    for i,name in enumerate(selection,1):
        fields[f'DesktopScreenshot{i}'] = f'{folder}/{name}'
        localized_captions = record.get('captions', {})
        if record['locale'] in ('en', 'en-us'):
            localized_captions = captions
        if name in localized_captions:
            fields[f'DesktopScreenshotCaption{i}'] = localized_captions[name]
    for name in ['StoreLogo300x300','StoreLogo1080x1080','StoreLogo720x1080','PromoImage1920x1080']:
        fields[name] = f'{folder}/{name}.png'
    return fields


def check_lengths(fields):
    limits = {'Title':256,'Description':10000,'ReleaseNotes':1500,'ShortDescription':1000,'ShortTitle':50,'SortTitle':255,'VoiceTitle':255,'CopyrightTrademarkInformation':200}
    for field,value in fields.items():
        limit = limits.get(field)
        if field.startswith(('Feature','MinimumHardwareReq','RecommendedHardwareReq','DesktopScreenshotCaption')): limit=200
        if field.startswith('SearchTerm'): limit=40
        if limit is not None: assert len(value)<=limit, f'{field}: {len(value)} > {limit}'
    keywords=[v for k,v in fields.items() if k.startswith('SearchTerm')]
    assert len(keywords)<=7 and sum(len(k.split()) for k in keywords)<=21
    assert fields.get('Description') and fields.get('DesktopScreenshot1')


def verify_import(folder, source_rows, locales):
    files = list(folder.rglob('*.csv'))
    assert len(files)==1, 'Import folder must contain exactly one CSV'
    with files[0].open(encoding='utf-8-sig',newline='') as stream:
        rows=list(csv.reader(stream))
    assert [r[:3] for r in rows]==[r[:3] for r in source_rows], 'Field/ID/Type metadata changed'
    assert rows[0] == source_rows[0][:4] + locales
    # Match the selected folder name and the PNG filename.
    assert all(path.is_file() for path in folder.iterdir()), 'Import must be flat'
    uploaded = {f'{folder.name}/{path.name}': path for path in folder.iterdir()}
    refs=set()
    for row in rows[1:]:
        for value in row[3:]:
            if row[2].startswith('Relative path') and value:
                assert value in uploaded, f'Asset absent from uploaded file list: {value}'
                assert uploaded[value].suffix.lower() == '.png'
                refs.add(value)
    expected = {path.name for path in folder.iterdir() if path.suffix != '.csv'}
    assert {uploaded[ref].name for ref in refs} == expected, 'Unreferenced upload assets'
    return {'folder':folder.name,'locales':locales,'csv_rows':len(rows),'asset_references':len(refs),'relative_upload_paths':sorted(uploaded),'status':'passed'}


def main():
    screenshot_data=json.loads((SOURCE/'screenshots.json').read_text())
    captions=screenshot_data['captions']
    screenshots=sorted((REPO/'docs/screenshots').glob('*.png'))
    assert set(p.name for p in screenshots)==set(captions), 'Every screenshot needs a caption'
    selection=screenshot_data['selected_30']
    assert len(selection)==len(set(selection))==30 and set(selection)<=set(captions)
    with TEMPLATE.open(encoding='utf-8-sig',newline='') as stream:
        template=[row for row in csv.reader(stream, strict=True) if any(row)]
    locales = template[0][4:]
    records = local_records(locales)
    default_locale_index = template[0].index('en-us')
    image_manifest=[]
    for path in screenshots:
        with Image.open(path) as image:
            image.verify()
        with Image.open(path) as image:
            assert image.format=='PNG' and image.mode in ('RGB','RGBA')
            assert image.width>=1366 and image.height>=768 and path.stat().st_size<50*1024*1024
            image_manifest.append({'file':path.name,'width':image.width,'height':image.height,'bytes':path.stat().st_size,'sha256':digest(path),'caption':captions[path.name],'slot_30':selection.index(path.name)+1 if path.name in selection else None,'slot_10':selection.index(path.name)+1 if path.name in selection[:10] else None,'interface_language':'en-us','capture_platform':'Linux'})
    for name,size in [('StoreLogo300x300',(300,300)),('StoreLogo1080x1080',(1080,1080)),('StoreLogo720x1080',(720,1080)),('PromoImage1920x1080',(1920,1080)),('StoreLogoOverride150x150',(150,150)),('StoreLogoOverride71x71',(71,71))]:
        with Image.open(BASE/'artwork'/f'{name}.png') as image:
            assert image.size==size and image.format=='PNG'
    results=[]
    for count in (30,10):
        folder=BASE.parent / ('microsoft-store' if count == 30 else 'microsoft-store-10')
        folder.mkdir(exist_ok=True)
        for name in selection[:count]:
            path=REPO/'docs/screenshots'/name
            shutil.copy2(path,folder/path.name)
            assert digest(path)==digest(folder/path.name)
        for name in ['StoreLogo300x300','StoreLogo1080x1080','StoreLogo720x1080','PromoImage1920x1080']:
            shutil.copy2(BASE/'artwork'/f'{name}.png',folder/f'{name}.png')
        localized={r['locale']:values(r, selection[:count], captions, folder.name) for r in records}
        for v in localized.values(): check_lengths(v)
        with (folder/TEMPLATE.name).open('w',encoding='utf-8-sig',newline='') as stream:
            writer=csv.writer(stream)
            writer.writerow(template[0])
            for row in template[1:]:
                cells = row[:4]
                for index, locale in enumerate(locales, start=4):
                    value = row[index] or localized[locale].get(row[0], '')
                    cells.append(' '.join(value.splitlines()))
                # Keep explicit defaults from the export and use US English
                # as the fallback for remaining fields.
                if not cells[3]:
                    cells[3] = cells[default_locale_index]
                writer.writerow(cells)
        results.append(verify_import(folder, template, locales))
    for record in records:
        dest=BASE/'locales'/record['locale']
        dest.mkdir(parents=True,exist_ok=True)
        fields=values(record, selection, captions, 'microsoft-store')
        write_json(dest/'listing.json',fields)
        for key,name in [('Title','title.txt'),('Description','description.txt'),('ShortDescription','short-description.txt'),('ReleaseNotes','release-notes.txt')]:
            if record.get(key):
                (dest/name).write_text(record[key]+'\n',encoding='utf-8')
        for key in ['features','keywords','minimum_hardware','recommended_hardware']:
            if record.get(key):
                (dest/(key.replace('_','-')+'.txt')).write_text('\n'.join(record[key])+'\n',encoding='utf-8')
    # Documentation is outside import folders so only the intended CSV is uploaded.
    shot_archive=BASE/'documentation/screenshots'
    shot_archive.mkdir(exist_ok=True)
    for path in screenshots:
        shutil.copy2(path,shot_archive/path.name)
    manual=BASE/'documentation/manual'
    manual.mkdir(exist_ok=True)
    for path in (REPO/'docs/manual').glob('*.md'): shutil.copy2(path,manual/path.name)
    licenses=BASE/'documentation/licenses'
    licenses.mkdir(exist_ok=True)
    for path in (REPO/'assets/fonts').glob('*LICENSE*'): shutil.copy2(path,licenses/path.name)
    body='<h1>Electrocraft screenshot gallery</h1><p>All '+str(len(screenshots))+' supplied PNGs. English interface; original captures are unmodified. Thirty slots are populated in microsoft-store; the first ten form microsoft-store-10.</p>'
    for item in sorted(image_manifest,key=lambda x: (x['slot_30'] or 999,x['file'])):
        body+=f'<figure><img loading="lazy" src="screenshots/{html.escape(item["file"])}" alt="{html.escape(item["caption"])}"><figcaption><strong>{html.escape(item["caption"])}</strong><br><code>{item["file"]}</code> · 30-slot position: {item["slot_30"] or "archive only"} · 10-slot position: {item["slot_10"] or "archive only"}</figcaption></figure>'
    (BASE/'documentation/screenshots.html').write_text(document('Electrocraft screenshots',body),encoding='utf-8')
    privacy=(BASE/'documentation/PRIVACY.md').read_text()
    body=''
    for paragraph in privacy.split('\n\n'):
        if paragraph.startswith('# '): body+='<h1>'+html.escape(paragraph[2:])+'</h1>'
        elif paragraph.startswith('## '): body+='<h2>'+html.escape(paragraph[3:])+'</h2>'
        else: body+='<p>'+html.escape(paragraph)+'</p>'
    (BASE/'documentation/privacy.html').write_text(document('Electrocraft privacy information',body),encoding='utf-8')
    write_json(BASE/'screenshots-manifest.json',image_manifest)
    packages=[]
    ident=json.loads((REPO/'packaging/windows/store-identity.json').read_text())
    for path in sorted((BASE/'packages').glob('*.msix')):
        checksum_file=path.with_suffix('.msix.sha256')
        expected=checksum_file.read_text().split()[0]
        assert digest(path)==expected, f'Checksum mismatch: {path}'
        with zipfile.ZipFile(path) as package:
            doc=ET.fromstring(package.read('AppxManifest.xml'))
        ns={'m':'http://schemas.microsoft.com/appx/manifest/foundation/windows10'}
        identity=doc.find('m:Identity',ns)
        assert identity.get('Name')==ident['identityName'] and identity.get('Publisher')==ident['publisher']
        assert identity.get('Version')=='1.1.13.0'
        assert identity.get('ProcessorArchitecture') in ('x64','arm64')
        resources=[x.get('Language') for x in doc.findall('m:Resources/m:Resource',ns)]
        assert resources==['en-us']
        packages.append({'file':path.name,'sha256':digest(path),'manifest_identity':dict(identity.attrib),'resource_languages':resources})
    assert {p['manifest_identity']['ProcessorArchitecture'] for p in packages}=={'x64','arm64'}, 'Both released Windows MSIX packages are required'
    write_json(BASE/'packages/manifest.json',packages)
    ready = set(locales)
    coverage = [
        {key: record[key] for key in ('locale', 'language', 'translation_status')}
        for record in records
    ]
    write_json(BASE / 'language-coverage.json', coverage)
    rows = [
        '# Listing-language coverage', '',
        f'The listing includes all {len(records)} locale columns from the Partner Center export.',
        'The application interface and screenshots are in English.', '',
        '| Locale | Language | Copy |', '| --- | --- | --- |',
    ]
    for record in coverage:
        rows.append(f"| {record['locale']} | {record['language']} | {record['translation_status']} |")
    (BASE / 'documentation/LANGUAGES.md').write_text('\n'.join(rows) + '\n', encoding='utf-8')
    write_json(BASE/'validation-report.json',{'status':'passed','scope':'local artifacts only; not Partner Center acceptance or Microsoft certification','imports':results,'source_screenshots':len(screenshots),'selected_screenshots':30,'reviewed_image_dimensions':True,'original_screenshot_hashes_match':True,'msix_checksums_and_identities_match':True,'packages':len(packages),'prepared_locales':sorted(ready),'unresolved':['Review the localized descriptions before publication','Publish a real public privacy-policy URL','Project-level declared license texts are absent','Complete Partner Center age ratings, pricing and account-level fields','Review screenshot platform parity and Windows certification requirements']})
    print(json.dumps({'imports':results,'screenshots':len(screenshots),'packages':len(packages),'locales':sorted(ready)},indent=2))


if __name__=='__main__':
    main()
