Online part catalogs (JLCPCB/LCSC, DigiKey, Mouser, TME, Farnell/element14) let you
find real parts, place them with their values, record where to buy them and export a
bill of materials.

## Consent comes first

Catalogs are **off** until the user turns on "Allow online part catalogs" and a
provider in Settings (with a confirmation naming what is sent). You cannot change
this, and there is no tool for it. If `parts_search` says catalogs are off, tell the
user which Settings page to open; do not retry. Keys live in the system keychain.
Only JLCPCB works without a key; the others need the user's own keys.

## Searching

`parts_search` (`query`, optional `provider` = `jlcpcb`, `digikey`, `mouser`, `tme`,
`farnell`; default all enabled) starts a search and returns at once. Results arrive
asynchronously: call `parts_results` until `pending` is false. Each result has the
vendor and order code, manufacturer and part number, stock, price breaks, package,
lifecycle and normalized parameters (`resistance` in ohms, `capacitance` in farads,
`tolerance` as a fraction, `power_rating`, `voltage_rating`...). Show
the provider's attribution text (included) when you quote its data.

There is no tool that places a search result directly (the Parts panel's Place button
is for the user). To use a result: `schematic_place` the matching component, set its
value with `schematic_set_param` and record the part with `sourcing_set`.

## Sourcing

A part's sourcing block holds the manufacturer part number, manufacturer, preferred
vendor and each vendor's order code, last unit price, stock and when it was checked.

- `sourcing_set` (`refdes` as a comma list, `mpn`, `manufacturer`, `preferred_vendor`).
- `sourcing_find_all`: look the parts up at every enabled vendor by manufacturer part
  number (asynchronous: describe the document later to see the vendor entries).
- `sourcing_refresh` (`refdes`), `sourcing_clear` (`refdes`).

Farnell's terms forbid storing its data: nothing from it is cached, and the schematic
keeps only its order code and link.

## Bill of materials

`bom_export_path` (`path` ending in `.csv`) writes the BOM grouped by identical parts,
plus one file per preferred vendor beside it (`name-jlcpcb.csv`, ...). Instruments and
ideal sources are left out.
