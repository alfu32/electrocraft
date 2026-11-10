# Circuit Simulator — Differential Spec: Parts Catalogs

Nov 10, 2026 · @Asdfghjkar

## 1. Scope

This change adds online part catalogs from five providers (JLCPCB, DigiKey, Mouser, TME, Farnell), a Parts view that switches between them, and a vendor field on every component. It applies on top of the base specification, "Circuit Simulator — Specification & Plan". Where the two conflict, this document wins.

**Adds:** a new crate `ecs-catalog`; a Settings page for online catalogs; a Parts panel; a Sourcing section in the inspector; a `sourcing` block in the schematic file; BOM export; phase P9.

**Does not touch:** the simulation engine, the component contract, scripting, SPICE import, or the canvas renderer. A catalog part only feeds parameters into existing built-in components.

**Core rule:** no catalog is contacted until the user explicitly approves it in Settings (section 3).

## 2. Providers

| Provider | Interface | What the user supplies | Notes |
| --- | --- | --- | --- |
| JLCPCB (and LCSC) | Community jlcsearch API for live search; optional offline snapshot from the jlcparts project; official JLCPCB Open API if the user has credentials | Nothing for jlcsearch and the snapshot; app ID and keys for the official API | Community services are unofficial and may change or disappear. Shows the JLCPCB Basic or Extended flag. |
| DigiKey | Product Information API v4 | Client ID and client secret (OAuth 2.0 client credentials) | Free developer account |
| Mouser | Search API | API key | Free key, may need approval. At most 50 results per request. |
| TME | TME API | Token and private key from developers.tme.eu | Requests are signed with the private key |
| Farnell (also Newark, element14) | element14 Product Search API | API key | One API covers all three storefronts; the user picks the store (for example uk.farnell.com). Keys can take a few days to be approved. |

- The app ships **no credentials and no catalog data**. Each user registers their own keys and downloads any snapshot themselves, so every request falls under the user's own agreement with the provider.
- Before implementing each provider, the coding agent reads its current terms and records the allowed cache lifetimes and rate limits in that provider's module documentation.
- Providers sit behind one trait (section 5), so more can be added later without UI changes.

## 3. Consent and privacy

Online catalog access is **off by default**, and nothing is sent to any provider until the user turns it on in Settings.

- **Settings > Online catalogs** has a master switch, "Allow online part catalogs", off by default.
- Under it, one entry per provider: an enable toggle (off by default), the credential fields, the store or region where relevant, a link to the provider's terms, and one line saying what is sent (search text, part numbers, the user's API key).
- A provider may make network requests only when **both** the master switch and its own toggle are on. This covers searches, refreshing stored prices, and snapshot downloads.
- The first time a provider is enabled, a confirmation dialog names the provider and what will be sent. The approval is stored in user settings, never in project files.
- Nothing else turns access on: no first-run prompt that enables it, no shortcut that bypasses Settings. The Parts view only offers a button that opens the Settings page.
- Turning a provider off stops its requests immediately and cancels any in flight. The user can also delete that provider's local cache.
- Credentials are stored in the operating system's keychain (the `keyring` crate), never in plain files or schematics.
- Opening a datasheet or product page launches the system browser. That is the user's own action and is not gated.
- With catalogs off, vendor data already saved in a schematic stays visible, marked with the date it was fetched.

## 4. Parts view

- A new dockable **Parts** panel next to the component palette.
- **Provider selector** at the top lists all five providers. Enabled ones can be selected; disabled ones are greyed with "Enable in Settings". An extra choice, "All enabled", searches every enabled provider and groups results by manufacturer part number.
- **Search** by keyword, manufacturer part number or vendor part number. Filters: category, package, in stock only, lifecycle status, and parametric ranges for common categories (resistance, capacitance, tolerance, voltage rating, power rating).
- **Results table:** manufacturer part number, manufacturer, description, package, two or three key parameters, stock, unit price at quantity 1 and at a quantity the user sets, vendor part number.
- **Detail pane:** all parameters, price breaks, lifecycle status, datasheet and product links, and whether a simulation model exists (section 6).
- **Actions:** *Place* (button or drag onto the canvas) creates a new component; *Assign to selection* sets this part as the source of the selected component(s).
- Each provider's errors (bad key, quota reached, offline) show inline in its own results area. One failing provider never blocks the others.

## 5. Provider layer (`ecs-catalog`)

A new library crate. It depends on `ecs-units` only; `ecs-app` uses it. `ecs-schematic` stores sourcing as plain data and does not depend on this crate.

```rust
pub trait CatalogProvider: Send + Sync {
    fn id(&self) -> ProviderId;            // Jlcpcb, Digikey, Mouser, Tme, Farnell
    fn capabilities(&self) -> Capabilities; // keyword, mpn, parametric, price, stock
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError>;
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError>;
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError>;
}
```

- **Consent gate:** every provider is wrapped in a `ConsentGate` that checks settings before each call. A call to a provider that is not approved returns `CatalogError::NotApproved` without touching the network.
- **Runtime:** a background thread with a Tokio runtime; HTTP through `reqwest` with rustls. The UI sends queries over a channel and receives results; it never blocks on the network.
- **Normalized part:** provider, vendor part number, manufacturer part number, manufacturer, description, category, package, parameters, price breaks (quantity, unit price, currency), stock, lead time, lifecycle (active, not recommended for new designs, obsolete), RoHS, datasheet URL, product URL, fetched-at time.
- **Parameter normalization:** each provider maps its attribute names to canonical keys (resistance, tolerance, power rating, capacitance, voltage rating, dielectric, inductance, current rating, package) with SI values parsed through `ecs-units`. Attributes it cannot map are kept as raw text.
- **Rate limiting and cache:** a request budget per provider; retry with backoff on HTTP 429; an SQLite cache in the user data folder, with lifetimes within each provider's terms (defaults: parameters 30 days, price and stock 24 hours).

## 6. From catalog part to simulated component

| Catalog category | What "Place" creates | Parameters filled |
| --- | --- | --- |
| Resistors | Built-in resistor | Resistance, tolerance; power rating stored as metadata |
| Capacitors | Built-in capacitor | Capacitance, ESR if given; voltage rating and dielectric stored as metadata |
| Inductors | Built-in inductor | Inductance, DC resistance; current rating stored as metadata |
| Diodes, Zeners, LEDs | Matching built-in with its default model | Zener voltage, LED color where given |
| Transistors, op-amps, regulators, other ICs | Generic built-in of the same class, marked "generic model" | None. The inspector offers to attach a SPICE model: search the user's library by part number, or open SPICE import (base spec 8.1). |
| Anything else | Not placeable | Can only be assigned to an existing component |

Sourcing never changes simulation parameters silently. Assigning a part to an existing component offers "Apply parameters from part", which shows the differences before applying.

## 7. Sourcing fields on components

Every component instance gets a `sourcing` block, saved in the `.ecs` file:

| Field | Meaning |
| --- | --- |
| `mpn`, `manufacturer` | The part's identity, independent of any vendor |
| `preferred_vendor` | One of the five providers, or none. This is the component's vendor field. |
| `vendors` | One entry per vendor where the part was found: provider, vendor part number, product URL, last unit price with quantity and currency, last stock, fetched-at time |

The list lets a part be recorded at several vendors at once; `preferred_vendor` says which one is used.

**Inspector, new Sourcing section**

- Manufacturer part number and manufacturer, editable by hand.
- Preferred vendor: a dropdown of all five providers plus none. Every provider is selectable even when disabled, so a vendor can be recorded offline.
- For the preferred vendor: vendor part number, price, stock and the date they were fetched.
- Buttons: *Search at vendor* (opens the Parts view on that provider, searching by part number if known, otherwise by the component's value and type); *Find at all vendors* (part-number search on every enabled provider, adding each hit to `vendors`); *Refresh*; *Clear*.
- If a search needs a provider that is not enabled, the button says so and opens Settings.

**Several components selected:** set the preferred vendor for all of them at once, or run *Find at all vendors* for those without sourcing.

**Bill of materials:** export a CSV with reference, value, quantity, manufacturer part number, manufacturer, preferred vendor, vendor part number and unit price, plus one CSV per vendor grouped by preferred vendor.

## 8. Changes to the base spec

| Base section | Change |
| --- | --- |
| 2. Decision log | Add D10: five catalog providers behind one trait. D11: online access is opt-in per provider in Settings and off by default. D12: credentials come from the user; catalog data is never bundled. *Merged as D17, D18 and D19: the base spec already uses D10–D16.* |
| 3. Architecture | Add the `ecs-catalog` crate (section 5). It runs its own background thread; the simulation thread is unaffected. |
| 9. Editor & UI | Add the Parts panel (section 4), the inspector Sourcing section (section 7) and Settings > Online catalogs (section 3). |
| 10. File formats | `.ecs` gains a `sourcing` block per component instance; bump `format_version` and migrate older files with an empty block. The catalog cache lives in the user data folder. Credentials are never written to project files. |
| 11. Testing | Add the tests in section 9 of this document. |
| 12. Implementation plan | Add phase P9 (section 9 of this document). |

## 9. Phase P9 and acceptance

P9 depends on base phase P4 (it needs the editor and inspector). It can run alongside P5 to P7, before the v1 release gate.

**Deliverables**

1. `ecs-catalog` with the consent gate, cache, rate limiting and parameter normalization.
2. The five providers.
3. Settings > Online catalogs, with keychain storage of credentials.
4. Parts panel, inspector Sourcing section, BOM export.
5. `.ecs` format bump and migration.

**Tests**

- Each provider is tested against recorded HTTP responses. CI never calls live services.
- With consent off (master switch, or a single provider), a mock transport asserts that zero network requests are made.
- Parameter normalization per provider, and part-to-component mapping per category (section 6).
- `.ecs` save and load with sourcing data gives identical bytes; older files load with an empty sourcing block.

**Done when**

- With catalogs off, the app makes no network request.
- With a provider enabled, a user can search it, place a resistor from the results with its value filled in, set a preferred vendor on an existing component, find that part at another enabled vendor, and export a BOM.
