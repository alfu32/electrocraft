//! Online part catalogs in the app (spec ADD-01): the consent settings, the
//! Parts panel, the inspector's Sourcing section and the BOM export.
//!
//! Nothing here contacts a provider unless the user turned on "Allow online
//! part catalogs" **and** that provider in Settings (with its confirmation);
//! the catalog worker is not even started before the first approved search.

use crate::app::App;
use crate::theme::{self, c};
use ecs_catalog::cache::Cache;
use ecs_catalog::credentials::{Keychain, Memory, Store};
use ecs_catalog::http::ReqwestTransport;
use ecs_catalog::mapping;
use ecs_catalog::providers::Env;
use ecs_catalog::service::{Outcome, Service};
use ecs_catalog::{CatalogQuery, Consent, Lifecycle, ProviderId, SharedConsent, VendorPart};
use ecs_schematic::{Command, Id, Item, Schematic, Sourcing, VendorRef};
use egui::{Color32, RichText, vec2};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// What a lookup job does with its replies.
enum Lookup {
    /// Fill vendor entries of these parts (same manufacturer part number).
    Find { tab: usize, parts: Vec<Id> },
    /// Refresh one part's entries.
    Refresh { tab: usize, part: Id },
}

pub struct PartsUi {
    pub open: bool,
    pub text: String,
    /// None: every enabled provider.
    pub provider: Option<ProviderId>,
    pub category: String,
    pub package: String,
    pub in_stock: bool,
    pub active_only: bool,
    pub range_key: String,
    pub range_min: String,
    pub range_max: String,
    pub results: Vec<(ProviderId, Result<Vec<VendorPart>, String>)>,
    pub pending: BTreeSet<(u64, ProviderId)>,
    pub job: u64,
    pub selected: Option<(ProviderId, usize)>,
    lookups: HashMap<u64, Lookup>,
    /// Parts looked up this session, per schematic part (Apply parameters).
    pub found: HashMap<Id, VendorPart>,
    service: Option<Service>,
    consent: SharedConsent,
    cache: Option<Arc<Cache>>,
    creds: Arc<dyn Store>,
    cred_edit: HashMap<String, String>,
    cred_state: Option<BTreeMap<ProviderId, bool>>,
    /// A provider waiting for the first-enable confirmation.
    pub confirm: Option<ProviderId>,
}

impl Default for PartsUi {
    fn default() -> Self {
        let creds: Arc<dyn Store> = if cfg!(test) {
            Arc::new(Memory::default())
        } else {
            Arc::new(Keychain)
        };
        Self {
            open: false,
            text: String::new(),
            provider: None,
            category: String::new(),
            package: String::new(),
            in_stock: false,
            active_only: false,
            range_key: String::new(),
            range_min: String::new(),
            range_max: String::new(),
            results: Vec::new(),
            pending: BTreeSet::new(),
            job: 0,
            selected: None,
            lookups: HashMap::new(),
            found: HashMap::new(),
            service: None,
            consent: Arc::new(RwLock::new(Consent::default())),
            cache: None,
            creds,
            cred_edit: HashMap::new(),
            cred_state: None,
            confirm: None,
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    crate::packages::config_dir().map(|d| d.join("catalog-cache.sqlite3"))
}

impl PartsUi {
    /// Copy the user's consent into the live gate; anything narrower cancels
    /// work in flight and drops results of providers no longer allowed.
    pub fn set_consent(&mut self, consent: &Consent) {
        let narrower = {
            let old = self.consent.read().map(|c| c.clone()).unwrap_or_default();
            ProviderId::ALL
                .iter()
                .any(|p| old.allows(*p) && !consent.allows(*p))
        };
        if let Ok(mut c) = self.consent.write() {
            *c = consent.clone();
        }
        if narrower {
            if let Some(s) = self.service.as_mut() {
                s.cancel();
            }
            self.pending.clear();
            self.lookups.clear();
            self.results.retain(|(p, _)| consent.allows(*p));
            self.selected = None;
        }
    }

    fn cache(&mut self) -> Arc<Cache> {
        if self.cache.is_none() {
            let c = if cfg!(test) {
                None
            } else {
                cache_path().and_then(|p| Cache::open(&p).ok())
            };
            self.cache = Some(Arc::new(c.unwrap_or_else(Cache::memory)));
        }
        self.cache.clone().unwrap()
    }

    fn service(&mut self, ctx: &egui::Context) -> &mut Service {
        if self.service.is_none() {
            let env = Env {
                transport: Arc::new(ReqwestTransport::default()),
                creds: self.creds.clone(),
                consent: self.consent.clone(),
                cache: self.cache(),
            };
            let ctx = ctx.clone();
            self.service = Some(Service::start(env, Arc::new(move || ctx.request_repaint())));
        }
        self.service.as_mut().unwrap()
    }

    fn allowed(&self) -> Vec<ProviderId> {
        let c = self.consent.read().map(|c| c.clone()).unwrap_or_default();
        ProviderId::ALL
            .into_iter()
            .filter(|p| c.allows(*p))
            .collect()
    }

    fn query(&self) -> CatalogQuery {
        let mut q = CatalogQuery::text(self.text.trim());
        q.in_stock = self.in_stock;
        q.category = self.category.trim().into();
        q.package = self.package.trim().into();
        q.active_only = self.active_only;
        let num = |s: &str| ecs_units::parse(s.trim()).ok();
        if !self.range_key.is_empty()
            && (num(&self.range_min).is_some() || num(&self.range_max).is_some())
        {
            q.ranges.push((
                self.range_key.clone(),
                num(&self.range_min),
                num(&self.range_max),
            ));
        }
        q
    }

    /// Start a search at the chosen provider (or every enabled one).
    /// Returns a message when nothing could be sent.
    pub fn search(&mut self, ctx: &egui::Context) -> Option<String> {
        let allowed = self.allowed();
        let which: Vec<ProviderId> = match self.provider {
            Some(p) => vec![p],
            None => allowed.clone(),
        };
        if self.text.trim().is_empty() {
            return Some("Type a part number, value or description".into());
        }
        if allowed.is_empty() {
            return Some("Online catalogs are off: Settings › Online part catalogs".into());
        }
        let q = self.query();
        let job = self.service(ctx).search(&which, q);
        self.job = job;
        self.results.clear();
        self.selected = None;
        for p in which {
            self.pending.insert((job, p));
        }
        None
    }

    /// The flattened rows shown in the results table.
    pub fn rows(&self) -> Vec<(ProviderId, usize, &VendorPart)> {
        let mut rows: Vec<(ProviderId, usize, &VendorPart)> = Vec::new();
        for (p, r) in &self.results {
            if let Ok(parts) = r {
                for (k, v) in parts.iter().enumerate() {
                    rows.push((*p, k, v));
                }
            }
        }
        if self.provider.is_none() {
            // "All enabled": grouped by manufacturer part number, sources distinct.
            let order: Vec<String> =
                ecs_catalog::group_by_mpn(&rows.iter().map(|r| r.2.clone()).collect::<Vec<_>>())
                    .into_iter()
                    .map(|g| g.0)
                    .collect();
            rows.sort_by_key(|r| {
                order
                    .iter()
                    .position(|m| *m == r.2.mpn.trim().to_uppercase())
                    .unwrap_or(usize::MAX)
            });
        }
        rows
    }

    pub fn selected_part(&self) -> Option<&VendorPart> {
        let (p, k) = self.selected?;
        self.results
            .iter()
            .find(|r| r.0 == p)
            .and_then(|r| r.1.as_ref().ok())
            .and_then(|v| v.get(k))
    }
}

/// What a schematic keeps of a vendor part (Farnell: number and link only).
pub fn vendor_ref(v: &VendorPart) -> VendorRef {
    let provider = v.provider.unwrap_or(ProviderId::Jlcpcb);
    let mut r = VendorRef {
        provider: provider.key().into(),
        vendor_pn: v.vendor_pn.clone(),
        url: v.product_url.clone(),
        ..Default::default()
    };
    if provider.may_store_prices() {
        if let Some(b) = v.price_at(1) {
            r.unit_price = Some(b.unit_price);
            r.price_qty = Some(b.quantity.max(1));
            r.currency = b.currency.clone();
        }
        r.stock = v.stock;
        r.fetched_at = v.fetched_at;
    }
    r
}

/// The number of a known simulation model matching a manufacturer part
/// number: exact, without a packing suffix (`BC547B,112`), without a vendor
/// prefix (`SN74HC00N`) or with trailing package letters (`NE555P`).
pub fn known_model(mpn: &str) -> Option<&'static ecs_model::KnownPart> {
    let base = mpn
        .trim()
        .split([',', '/', '#', ' '])
        .next()
        .unwrap_or("")
        .to_uppercase();
    let mut candidates = vec![base.clone()];
    for prefix in ["SN", "MC", "CD", "HEF", "MM", "DM", "LM", "UA", "NE"] {
        if let Some(rest) = base.strip_prefix(prefix)
            && rest.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            candidates.push(rest.to_string());
        }
    }
    for c in candidates.clone() {
        let mut s = c.clone();
        while s.len() > 4
            && s.chars()
                .last()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '-')
        {
            s.pop();
            candidates.push(s.clone());
        }
    }
    candidates.iter().find_map(|c| ecs_lib::parts::find(c))
}

/// Definitions that only have a generic model of their class.
fn generic_class(def: &str) -> bool {
    matches!(
        def,
        "core.npn"
            | "core.pnp"
            | "core.nmos"
            | "core.pmos"
            | "core.opamp"
            | "core.comparator"
            | "core.ne555"
    )
}

/// Parts left out of a bill of materials (instruments, ideal sources).
fn not_bought(def: &str) -> bool {
    matches!(
        def,
        "core.ground"
            | "core.voltmeter"
            | "core.ammeter"
            | "core.logic_probe"
            | "core.funcgen"
            | "core.vsine"
            | "core.clock"
            | "core.idc"
            | "core.vdc"
            | "core.vcvs"
            | "core.vccs"
            | "core.cccs"
            | "core.ccvs"
            | "core.logic_toggle"
    ) || def.starts_with("local.")
}

fn csv(fields: &[String]) -> String {
    let esc = |s: &String| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.clone()
        }
    };
    fields.iter().map(esc).collect::<Vec<_>>().join(",") + "\n"
}

/// Bill of materials: one CSV for the whole design and one per preferred
/// vendor (`(provider key, csv)`), lines grouped by identical parts.
pub fn bom(doc: &Schematic, cat: &crate::catalog::Catalog) -> (String, Vec<(String, String)>) {
    // Group: definition, value, manufacturer part, vendor.
    let mut lines: Vec<(String, Vec<String>, &ecs_schematic::Part)> = Vec::new();
    let mut parts: Vec<&ecs_schematic::Part> =
        doc.parts.iter().filter(|p| !not_bought(&p.def)).collect();
    parts.sort_by_key(|p| natural(&p.refdes));
    for p in parts {
        let value = value_text(p, cat);
        let pref = p
            .sourcing
            .preferred()
            .map(|v| v.vendor_pn.clone())
            .unwrap_or_default();
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            p.def, value, p.sourcing.mpn, p.part_number, p.sourcing.preferred_vendor, pref
        );
        match lines.iter_mut().find(|l| l.0 == key) {
            Some(l) => l.1.push(p.refdes.clone()),
            None => lines.push((key, vec![p.refdes.clone()], p)),
        }
    }
    let mut all = csv(&[
        "Quantity",
        "References",
        "Component",
        "Value",
        "Manufacturer part number",
        "Manufacturer",
        "Preferred vendor",
        "Vendor part number",
        "Unit price",
        "Currency",
        "Price quantity",
        "Stock",
        "Checked",
        "Link",
    ]
    .map(String::from));
    let mut per: BTreeMap<String, String> = BTreeMap::new();
    for (_, refs, p) in &lines {
        let name = cat
            .def(&p.def)
            .map_or(p.def.clone(), |d| d.meta.name.clone());
        let mpn = if p.sourcing.mpn.is_empty() {
            p.part_number.clone()
        } else {
            p.sourcing.mpn.clone()
        };
        let v = p.sourcing.preferred().cloned().unwrap_or_default();
        let vendor = ProviderId::from_key(&p.sourcing.preferred_vendor)
            .map_or(String::new(), |x| x.name().to_string());
        let checked = if v.fetched_at > 0 {
            date(v.fetched_at)
        } else {
            String::new()
        };
        all += &csv(&[
            refs.len().to_string(),
            refs.join(" "),
            name.clone(),
            value_text(p, cat),
            mpn.clone(),
            p.sourcing.manufacturer.clone(),
            vendor,
            v.vendor_pn.clone(),
            v.unit_price.map(|x| format!("{x}")).unwrap_or_default(),
            v.currency.clone(),
            v.price_qty.map(|x| x.to_string()).unwrap_or_default(),
            v.stock.map(|x| x.to_string()).unwrap_or_default(),
            checked,
            v.url.clone(),
        ]);
        if !p.sourcing.preferred_vendor.is_empty() && !v.vendor_pn.is_empty() {
            let file = per
                .entry(p.sourcing.preferred_vendor.clone())
                .or_insert_with(|| {
                    if p.sourcing.preferred_vendor == "jlcpcb" {
                        csv(&["Comment", "Designator", "Footprint", "LCSC Part #"]
                            .map(String::from))
                    } else {
                        csv(&[
                            "Quantity",
                            "Vendor part number",
                            "Manufacturer part number",
                            "Customer reference",
                        ]
                        .map(String::from))
                    }
                });
            if p.sourcing.preferred_vendor == "jlcpcb" {
                *file += &csv(&[
                    value_or(&value_text(p, cat), &mpn),
                    refs.join(","),
                    String::new(),
                    v.vendor_pn.clone(),
                ]);
            } else {
                *file += &csv(&[
                    refs.len().to_string(),
                    v.vendor_pn.clone(),
                    mpn.clone(),
                    refs.join(" "),
                ]);
            }
        }
    }
    (all, per.into_iter().collect())
}

fn value_or(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else {
        a.to_string()
    }
}

/// The main value of a part as typed (R, C, L...), if it has one.
fn value_text(p: &ecs_schematic::Part, cat: &crate::catalog::Catalog) -> String {
    let Some(def) = cat.def(&p.def) else {
        return String::new();
    };
    for (name, unit) in [("R", "Ω"), ("C", "F"), ("L", "H"), ("BV", "V")] {
        if def.params.iter().any(|x| x.name == name) {
            let t = crate::catalog::param_text(p, def, name);
            return format!("{t}{unit}");
        }
    }
    String::new()
}

/// "R10" after "R9".
fn natural(s: &str) -> (String, u64) {
    let k = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
    (s[..k].to_string(), s[k..].parse().unwrap_or(0))
}

/// Seconds since the epoch as a UTC date.
pub fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Civil from days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn age(secs: u64) -> String {
    let now = ecs_catalog::now();
    let d = now.saturating_sub(secs);
    match d {
        0..3600 => format!("{} min ago", d / 60),
        3600..86_400 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86_400),
    }
}

fn money(v: f64, cur: &str) -> String {
    let digits = if v < 0.1 {
        4
    } else if v < 10.0 {
        3
    } else {
        2
    };
    format!("{v:.digits$} {cur}")
}

fn lifecycle_color(l: Lifecycle) -> Color32 {
    match l {
        Lifecycle::Active => c::OK,
        Lifecycle::NotRecommended => c::WARN,
        Lifecycle::Obsolete => c::DANGER,
        Lifecycle::Unknown => c::TEXT_FAINT,
    }
}

const RANGE_KEYS: [(&str, &str); 8] = [
    ("", "no range"),
    ("resistance", "Resistance (Ω)"),
    ("capacitance", "Capacitance (F)"),
    ("inductance", "Inductance (H)"),
    ("voltage_rating", "Voltage rating (V)"),
    ("power_rating", "Power rating (W)"),
    ("tolerance", "Tolerance (fraction)"),
    ("zener_voltage", "Zener voltage (V)"),
];

enum Act {
    Search,
    Place(VendorPart),
    Assign(VendorPart, Vec<Id>),
    Apply(Id, Vec<(String, String)>),
    Settings,
}

impl App {
    /// Apply catalog replies (each frame).
    pub fn poll_catalogs(&mut self) {
        let Some(svc) = self.parts.service.as_ref() else {
            return;
        };
        let replies = svc.poll();
        for r in replies {
            self.parts.pending.remove(&(r.job, r.provider));
            if r.job == self.parts.job {
                let res = match r.outcome {
                    Outcome::Page(p) => p.map(|p| p.parts),
                    Outcome::Parts(p) => p,
                };
                self.parts
                    .results
                    .push((r.provider, res.map_err(|e| e.to_string())));
                self.parts.results.sort_by_key(|x| x.0);
                continue;
            }
            let Some(lookup) = self.parts.lookups.get(&r.job) else {
                continue;
            };
            let parts = match r.outcome {
                Outcome::Page(p) => p.map(|p| p.parts),
                Outcome::Parts(p) => p,
            };
            let (tab, targets) = match lookup {
                Lookup::Find { tab, parts } => (*tab, parts.clone()),
                Lookup::Refresh { tab, part } => (*tab, vec![*part]),
            };
            if !self.parts.pending.iter().any(|(j, _)| *j == r.job) {
                self.parts.lookups.remove(&r.job);
            }
            if tab != self.active {
                continue;
            }
            match parts {
                Ok(found) => {
                    let mut changes = Vec::new();
                    for id in targets {
                        let Some(part) = self.doc.part(id) else {
                            continue;
                        };
                        let mpn = part.sourcing.mpn.trim().to_uppercase();
                        let mine = part
                            .sourcing
                            .vendor(r.provider.key())
                            .map(|v| v.vendor_pn.clone());
                        // Same vendor number first (refresh), then the same manufacturer part.
                        let best = found
                            .iter()
                            .find(|v| {
                                mine.as_deref()
                                    .is_some_and(|m| v.vendor_pn.eq_ignore_ascii_case(m))
                            })
                            .or_else(|| {
                                found.iter().find(|v| {
                                    !mpn.is_empty() && v.mpn.trim().eq_ignore_ascii_case(&mpn)
                                })
                            })
                            .or_else(|| found.first().filter(|_| mine.is_some()));
                        if let Some(v) = best {
                            let mut s = part.sourcing.clone();
                            s.upsert(vendor_ref(v));
                            if s.manufacturer.is_empty() {
                                s.manufacturer = v.manufacturer.clone();
                            }
                            self.parts.found.insert(id, v.clone());
                            changes.push((id, s));
                        }
                    }
                    if !changes.is_empty() {
                        self.status =
                            format!("{}: updated {} part(s)", r.provider.name(), changes.len());
                        self.edit(Command::SetSourcing(changes));
                    }
                }
                Err(ecs_catalog::CatalogError::NotFound) => {}
                Err(e) => self.status = format!("{}: {e}", r.provider.name()),
            }
        }
    }

    fn selected_parts(&self) -> Vec<Id> {
        self.selection
            .iter()
            .filter_map(|i| match i {
                Item::Part(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Look up parts at every enabled vendor by manufacturer part number.
    pub fn find_at_vendors(&mut self, ctx: &egui::Context, parts: Vec<Id>) {
        let allowed = self.parts.allowed();
        if allowed.is_empty() {
            self.status = "Online catalogs are off: Settings › Online part catalogs".into();
            return;
        }
        let mut by_mpn: BTreeMap<(String, String), Vec<Id>> = BTreeMap::new();
        for id in parts {
            if let Some(p) = self.doc.part(id) {
                let mpn = if p.sourcing.mpn.is_empty() {
                    p.part_number.clone()
                } else {
                    p.sourcing.mpn.clone()
                };
                if !mpn.trim().is_empty() {
                    by_mpn
                        .entry((mpn.trim().to_string(), p.sourcing.manufacturer.clone()))
                        .or_default()
                        .push(id);
                }
            }
        }
        if by_mpn.is_empty() {
            self.status = "Set a manufacturer part number first".into();
            return;
        }
        // Parts known only by their model number get it as manufacturer part.
        let mut fill = Vec::new();
        for ((mpn, _), ids) in &by_mpn {
            for id in ids {
                if let Some(p) = self.doc.part(*id)
                    && p.sourcing.mpn.is_empty()
                {
                    let mut s = p.sourcing.clone();
                    s.mpn = mpn.clone();
                    fill.push((*id, s));
                }
            }
        }
        if !fill.is_empty() {
            self.edit(Command::SetSourcing(fill));
        }
        let tab = self.active;
        let n = by_mpn.len();
        for ((mpn, mfr), ids) in by_mpn {
            let mfr = (!mfr.is_empty()).then_some(mfr);
            let job = self
                .parts
                .service(ctx)
                .by_mpn(&allowed, &mpn, mfr.as_deref());
            for p in &allowed {
                self.parts.pending.insert((job, *p));
            }
            self.parts
                .lookups
                .insert(job, Lookup::Find { tab, parts: ids });
        }
        self.status = format!(
            "Looking up {n} part number(s) at {} vendor(s)…",
            allowed.len()
        );
    }

    /// Refresh the vendor entries of one part.
    pub fn refresh_sourcing(&mut self, ctx: &egui::Context, id: Id) {
        let Some(p) = self.doc.part(id).cloned() else {
            return;
        };
        let allowed = self.parts.allowed();
        let tab = self.active;
        let mut sent = 0;
        for v in &p.sourcing.vendors {
            let Some(pid) = ProviderId::from_key(&v.provider) else {
                continue;
            };
            if !allowed.contains(&pid) {
                continue;
            }
            let job = self.parts.service(ctx).by_vendor_pn(pid, &v.vendor_pn);
            self.parts.pending.insert((job, pid));
            self.parts
                .lookups
                .insert(job, Lookup::Refresh { tab, part: id });
            sent += 1;
        }
        self.status = if sent == 0 {
            "No enabled vendor holds this part: Settings › Online part catalogs".into()
        } else {
            format!(
                "Refreshing {sent} vendor entr{}…",
                if sent == 1 { "y" } else { "ies" }
            )
        };
    }

    /// Place a catalog part: its model when the number is known, else the
    /// class's component with the part's values (generic for semiconductors).
    pub fn place_catalog_part(&mut self, v: &VendorPart) {
        let r = self.canvas_rect;
        let at = ecs_schematic::snap(self.vp.world([r.width() * 0.5, r.height() * 0.5]));
        let (def_id, params, generic) = match (known_model(&v.mpn), mapping::placement(v)) {
            (Some(k), m) if m.as_ref().is_none_or(|m| m.generic || m.def == k.def) => (
                k.def.clone(),
                k.params
                    .iter()
                    .map(|(n, x)| {
                        (
                            n.clone(),
                            ecs_units::format_digits(*x, "", 4).replace(' ', ""),
                        )
                    })
                    .collect::<Vec<_>>(),
                false,
            ),
            (_, Some(m)) => (m.def.to_string(), m.params.clone(), m.generic),
            (_, None) => {
                self.status = format!(
                    "{} has no simulation model: select a part and use Assign to selection",
                    v.mpn
                );
                return;
            }
        };
        let Some(def) = self.cat.def(&def_id).cloned() else {
            self.status = format!("Unknown component {def_id}");
            return;
        };
        let mut p = self.cat.new_part(&self.doc, &def, at);
        for (k, x) in params {
            p.params.insert(k, x);
        }
        p.part_number = v.mpn.clone();
        p.sourcing = Sourcing {
            mpn: v.mpn.clone(),
            manufacturer: v.manufacturer.clone(),
            preferred_vendor: v.provider.map_or(String::new(), |x| x.key().into()),
            vendors: vec![vendor_ref(v)],
        };
        let refdes = p.refdes.clone();
        let created = self.edit(Command::AddPart(Box::new(p)));
        if let Some(Item::Part(id)) = created.first() {
            self.parts.found.insert(*id, v.clone());
        }
        self.selection = created.into_iter().collect();
        self.status = if generic {
            format!(
                "Placed {refdes} ({}) with a generic {} model",
                v.mpn, def.meta.name
            )
        } else {
            format!("Placed {refdes} ({})", v.mpn)
        };
    }

    /// Record a catalog part as the sourcing of the selected parts.
    pub fn assign_catalog_part(&mut self, v: &VendorPart, parts: &[Id]) {
        let mut changes = Vec::new();
        for id in parts {
            let Some(p) = self.doc.part(*id) else {
                continue;
            };
            let mut s = p.sourcing.clone();
            if !s.mpn.eq_ignore_ascii_case(&v.mpn) {
                // Another part: other vendors' entries no longer apply.
                s.vendors.clear();
            }
            s.mpn = v.mpn.clone();
            s.manufacturer = v.manufacturer.clone();
            s.upsert(vendor_ref(v));
            if s.preferred_vendor.is_empty() || s.preferred().is_none() {
                s.preferred_vendor = v.provider.map_or(String::new(), |x| x.key().into());
            }
            self.parts.found.insert(*id, v.clone());
            changes.push((*id, s));
        }
        let n = changes.len();
        self.edit(Command::SetSourcing(changes));
        self.status = format!("{} assigned to {n} part(s)", v.mpn);
    }

    /// Parameter changes a catalog part would bring to a placed part.
    pub fn param_diff(&self, id: Id, v: &VendorPart) -> Vec<(String, String, String)> {
        let Some(part) = self.doc.part(id) else {
            return Vec::new();
        };
        let Some(def) = self.cat.def(&part.def) else {
            return Vec::new();
        };
        let new: Vec<(String, String)> = match known_model(&v.mpn).filter(|k| k.def == part.def) {
            Some(k) => k
                .params
                .iter()
                .map(|(n, x)| {
                    (
                        n.clone(),
                        ecs_units::format_digits(*x, "", 4).replace(' ', ""),
                    )
                })
                .collect(),
            None => match mapping::placement(v) {
                Some(m) if m.def == part.def => m.params,
                _ => Vec::new(),
            },
        };
        new.into_iter()
            .filter(|(n, _)| def.params.iter().any(|p| p.name == *n))
            .filter_map(|(n, x)| {
                let old = crate::catalog::param_text(part, def, &n);
                let same = match (
                    crate::catalog::parse_param(def, &n, &old),
                    crate::catalog::parse_param(def, &n, &x),
                ) {
                    (Some(a), Some(b)) => (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-30),
                    _ => old == x,
                };
                (!same).then_some((n, old, x))
            })
            .collect()
    }

    /// Settings › Online part catalogs.
    pub fn catalog_settings(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.catalogs.clone();
        if self.parts.cred_state.is_none() {
            self.parts.cred_state = Some(
                ProviderId::ALL
                    .into_iter()
                    .map(|p| (p, self.parts.creds.complete(p)))
                    .collect(),
            );
        }
        ui.checkbox(
            &mut self.settings.catalogs.master,
            RichText::new("Allow online part catalogs").strong(),
        );
        ui.label(
            RichText::new("Off: Electrocraft contacts no catalog. On: only the providers below that you enable, only when you search or look up a part. Keys stay in the system keychain, never in files or schematics.")
                .color(c::TEXT_DIM)
                .small(),
        );
        ui.add_space(4.0);
        let master = self.settings.catalogs.master;
        for p in ProviderId::ALL {
            let mut on = self.settings.catalogs.enabled.contains(&p);
            ui.add_enabled_ui(master, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut on, RichText::new(p.name()).strong())
                        .changed()
                    {
                        if on {
                            // Approval goes through the confirmation first.
                            self.parts.confirm = Some(p);
                        } else {
                            self.settings.catalogs.enabled.retain(|x| *x != p);
                        }
                    }
                    ui.hyperlink_to(RichText::new("terms").small(), p.terms_url());
                });
            });
            if !master || !self.settings.catalogs.enabled.contains(&p) {
                continue;
            }
            ui.indent(("cat", p.key()), |ui| {
                ui.label(RichText::new(p.sends()).color(c::TEXT_FAINT).small());
                let have = self.parts.cred_state.as_ref().and_then(|m| m.get(&p).copied()).unwrap_or(false);
                if !p.credential_fields().is_empty() {
                    ui.label(RichText::new(if have { "Credentials: stored in the system keychain" } else { "Credentials: missing" }).color(if have { c::OK } else { c::WARN }).small());
                }
                let mut save = false;
                for (field, label) in p.credential_fields() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(*label).color(c::TEXT_DIM).small());
                        let key = format!("{}.{field}", p.key());
                        let text = self.parts.cred_edit.entry(key).or_default();
                        let r = ui.add(egui::TextEdit::singleline(text).password(true).desired_width(170.0).hint_text(if have { "•••• (stored)" } else { "" }));
                        save |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    });
                }
                ui.horizontal(|ui| {
                    if !p.credential_fields().is_empty() {
                        save |= ui.button("Save to keychain").clicked();
                        if have && ui.button("Forget").clicked() {
                            for (f, _) in p.credential_fields() {
                                let _ = self.parts.creds.delete(p, f);
                            }
                            self.parts.cred_state = None;
                        }
                    }
                    let n = self.parts.cache().entries(p);
                    if ui.add_enabled(n > 0, egui::Button::new(format!("Delete cached data ({n})"))).clicked() {
                        self.parts.cache().clear(p);
                    }
                });
                if save {
                    let mut err = None;
                    for (f, _) in p.credential_fields() {
                        let key = format!("{}.{f}", p.key());
                        let v = self.parts.cred_edit.get(&key).cloned().unwrap_or_default();
                        if !v.trim().is_empty()
                            && let Err(e) = self.parts.creds.set(p, f, v.trim())
                        {
                            err = Some(e);
                        }
                        self.parts.cred_edit.remove(&key);
                    }
                    self.parts.cred_state = None;
                    self.status = err.unwrap_or_else(|| format!("{} credentials saved to the keychain", p.name()));
                }
                let (hint, default) = match p {
                    ProviderId::Digikey => ("Site,currency", "US,USD"),
                    ProviderId::Tme => ("Country,currency,language", "GB,EUR,en"),
                    ProviderId::Farnell => ("Store", "uk.farnell.com"),
                    _ => ("", ""),
                };
                if !hint.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(hint).color(c::TEXT_DIM).small());
                        let v = self.settings.catalogs.region.entry(p.key().into()).or_default();
                        ui.add(egui::TextEdit::singleline(v).hint_text(default).desired_width(150.0));
                    });
                }
                if p == ProviderId::Jlcpcb {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Offline snapshot").color(c::TEXT_DIM).small()).on_hover_text("A jlcparts cache.sqlite3 (yaqwsx.github.io/jlcparts): searched locally, without network");
                        ui.add(egui::TextEdit::singleline(&mut self.settings.catalogs.jlc_snapshot).hint_text("none: jlcsearch online").desired_width(150.0));
                        if ui.small_button("…").clicked()
                            && let Some(f) = rfd::FileDialog::new().add_filter("SQLite", &["sqlite3", "sqlite", "db"]).pick_file()
                        {
                            self.settings.catalogs.jlc_snapshot = f.display().to_string();
                        }
                    });
                }
                ui.label(RichText::new(p.attribution()).color(c::TEXT_FAINT).small().italics());
            });
        }
        // Region entries left empty are not kept.
        self.settings
            .catalogs
            .region
            .retain(|_, v| !v.trim().is_empty());
        if self.settings.catalogs != before {
            self.parts.set_consent(&self.settings.catalogs);
            self.settings.save();
        }
    }

    /// The first-enable confirmation (spec 3): what is sent, and where.
    pub fn catalog_confirm(&mut self, ctx: &egui::Context) {
        let Some(p) = self.parts.confirm else { return };
        let mut decided = None;
        egui::Window::new(RichText::new(format!("Enable {}?", p.name())).font(theme::bold(14.0)))
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label("When you search or look up parts, Electrocraft will send:");
                ui.label(RichText::new(p.sends()).strong());
                ui.add_space(4.0);
                ui.label(RichText::new("Nothing is sent in the background, and you can turn this off at any time. Results follow the provider's terms:").color(c::TEXT_DIM).small());
                ui.hyperlink(p.terms_url());
                if !p.may_store_prices() {
                    ui.label(RichText::new("Its terms forbid storing its data: results are never cached, and schematics keep only the order code and link.").color(c::WARN).small());
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Enable").strong()).clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
        if let Some(yes) = decided {
            self.parts.confirm = None;
            if yes && !self.settings.catalogs.enabled.contains(&p) {
                self.settings.catalogs.enabled.push(p);
                self.parts.set_consent(&self.settings.catalogs);
                self.settings.save();
            }
        }
    }

    /// The Parts panel.
    pub fn parts_window(&mut self, ctx: &egui::Context) {
        if !self.parts.open {
            return;
        }
        let mut open = true;
        let mut acts: Vec<Act> = Vec::new();
        let allowed = self.parts.allowed();
        let selected_parts = self.selected_parts();
        egui::Window::new(RichText::new("Parts").font(theme::bold(14.0)))
            .id(egui::Id::unique("parts-window"))
            .open(&mut open)
            .fixed_size(vec2(940.0, 540.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                if allowed.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Online catalogs are off: no provider is contacted.").color(c::WARN));
                        if ui.button("Settings…").clicked() {
                            acts.push(Act::Settings);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    let label = self.parts.provider.map_or("All enabled".to_string(), |p| p.name().to_string());
                    egui::ComboBox::from_id_salt("parts-provider").width(190.0).selected_text(label).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.parts.provider, None, "All enabled");
                        for p in ProviderId::ALL {
                            let txt = if allowed.contains(&p) { p.name().to_string() } else { format!("{} (off)", p.name()) };
                            ui.selectable_value(&mut self.parts.provider, Some(p), txt);
                        }
                    });
                    let r = ui.add(egui::TextEdit::singleline(&mut self.parts.text).hint_text("Part number, value or description (e.g. 10k 0603, NE555)").desired_width(420.0));
                    if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) | ui.button(RichText::new("Search").strong()).clicked() {
                        acts.push(Act::Search);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.parts.category).hint_text("category").desired_width(110.0));
                    ui.add(egui::TextEdit::singleline(&mut self.parts.package).hint_text("package").desired_width(80.0));
                    ui.checkbox(&mut self.parts.in_stock, "In stock");
                    ui.checkbox(&mut self.parts.active_only, "Active only");
                    let cur = RANGE_KEYS.iter().find(|k| k.0 == self.parts.range_key).map_or("no range", |k| k.1);
                    egui::ComboBox::from_id_salt("parts-range").width(140.0).selected_text(cur).show_ui(ui, |ui| {
                        for (k, l) in RANGE_KEYS {
                            ui.selectable_value(&mut self.parts.range_key, k.to_string(), l);
                        }
                    });
                    if !self.parts.range_key.is_empty() {
                        ui.add(egui::TextEdit::singleline(&mut self.parts.range_min).hint_text("min").desired_width(54.0));
                        ui.label("–");
                        ui.add(egui::TextEdit::singleline(&mut self.parts.range_max).hint_text("max").desired_width(54.0));
                    }
                });
                // Per-provider state, errors inline.
                let shown: Vec<ProviderId> = match self.parts.provider {
                    Some(p) => vec![p],
                    None => allowed.clone(),
                };
                ui.horizontal_wrapped(|ui| {
                    for p in &shown {
                        if self.parts.pending.contains(&(self.parts.job, *p)) {
                            ui.spinner();
                            ui.label(RichText::new(p.name()).color(c::TEXT_DIM).small());
                        } else if let Some((_, r)) = self.parts.results.iter().find(|r| r.0 == *p) {
                            match r {
                                Ok(v) => ui.label(RichText::new(format!("{}: {}", p.name(), v.len())).color(c::TEXT_DIM).small()),
                                Err(e) => ui.label(RichText::new(format!("{}: {e}", p.name())).color(c::DANGER).small()),
                            };
                        }
                        ui.add_space(8.0);
                    }
                });
                ui.separator();
                let rows: Vec<(ProviderId, usize, VendorPart)> = self.parts.rows().into_iter().map(|(p, k, v)| (p, k, v.clone())).collect();
                ui.horizontal_top(|ui| {
                    let list_w = 940.0 - 290.0;
                    ui.allocate_ui(vec2(list_w, ui.available_height()), |ui| {
                        egui::ScrollArea::both().id_salt("parts-results").max_width(list_w).auto_shrink([false, false]).show(ui, |ui| {
                            egui::Grid::new("parts-grid").striped(true).num_columns(7).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                                for h in ["Vendor", "Part", "Stock", "Price", "Status", "Manufacturer", "Description"] {
                                    ui.label(RichText::new(h).color(c::TEXT_FAINT).small());
                                }
                                ui.end_row();
                                let mut last_mpn = String::new();
                                for (p, k, v) in &rows {
                                    let sel = self.parts.selected == Some((*p, *k));
                                    let grouped = self.parts.provider.is_none() && v.mpn.eq_ignore_ascii_case(&last_mpn);
                                    last_mpn = v.mpn.clone();
                                    ui.label(RichText::new(p.key()).color(c::TEXT_DIM).small()).on_hover_text(p.attribution());
                                    let name = if grouped { format!("  ↳ {}", v.vendor_pn) } else { format!("{}  {}", v.mpn, v.vendor_pn) };
                                    if ui.selectable_label(sel, RichText::new(name).font(theme::mono(11.5))).clicked() {
                                        self.parts.selected = Some((*p, *k));
                                    }
                                    ui.label(RichText::new(v.stock.map_or("–".into(), |s| s.to_string())).small());
                                    ui.label(RichText::new(v.price_at(1).map_or("–".into(), |b| money(b.unit_price, &b.currency))).small());
                                    let mut status = v.lifecycle.label().to_string();
                                    if let Some(b) = v.jlc_basic {
                                        status = if b { "basic".into() } else { "extended".into() };
                                    }
                                    ui.label(RichText::new(status).color(lifecycle_color(v.lifecycle)).small());
                                    ui.label(RichText::new(&v.manufacturer).small());
                                    let d: String = v.description.chars().take(48).collect();
                                    ui.label(RichText::new(d).small()).on_hover_text(&v.description);
                                    ui.end_row();
                                }
                            });
                            if rows.is_empty() && self.parts.pending.is_empty() {
                                ui.label(RichText::new("No results yet.").color(c::TEXT_FAINT));
                            }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(270.0);
                        egui::ScrollArea::vertical().id_salt("parts-detail").auto_shrink([false, false]).show(ui, |ui| {
                            let Some(v) = self.parts.selected_part().cloned() else {
                                ui.label(RichText::new("Select a result to see its details.").color(c::TEXT_FAINT));
                                return;
                            };
                            ui.label(RichText::new(&v.mpn).font(theme::bold(14.0)));
                            ui.label(RichText::new(&v.manufacturer).color(c::TEXT_DIM));
                            ui.label(RichText::new(&v.description).small());
                            ui.add_space(4.0);
                            if let Some(p) = v.provider {
                                ui.label(RichText::new(format!("{}: {}", p.name(), v.vendor_pn)).strong());
                            }
                            if !v.category.is_empty() {
                                ui.label(RichText::new(&v.category).color(c::TEXT_DIM).small());
                            }
                            if !v.package.is_empty() {
                                ui.label(RichText::new(format!("Package: {}", v.package)).small());
                            }
                            if let Some(s) = v.stock {
                                ui.label(RichText::new(format!("Stock: {s}")).small());
                            }
                            if !v.lead_time.is_empty() {
                                ui.label(RichText::new(format!("Lead time: {}", v.lead_time)).small());
                            }
                            ui.label(RichText::new(format!("Lifecycle: {}", v.lifecycle.label())).color(lifecycle_color(v.lifecycle)).small());
                            if !v.prices.is_empty() {
                                ui.add_space(4.0);
                                for b in v.prices.iter().take(6) {
                                    ui.label(RichText::new(format!("{:>6}+  {}", b.quantity, money(b.unit_price, &b.currency))).font(theme::mono(11.0)));
                                }
                            }
                            let params: Vec<_> = v.params.iter().filter(|(k, _)| ecs_catalog::normalize::KEYS.iter().any(|x| x.0 == k.as_str())).collect();
                            if !params.is_empty() {
                                ui.add_space(4.0);
                                for (k, x) in params {
                                    let t = match x {
                                        ecs_catalog::ParamValue::Number(n) => {
                                            let unit = ecs_catalog::normalize::KEYS.iter().find(|u| u.0 == k.as_str()).map_or("", |u| u.1);
                                            if unit == "fraction" { format!("{:.3} %", n * 100.0) } else { ecs_units::format_digits(*n, unit, 4) }
                                        }
                                        ecs_catalog::ParamValue::Text(t) => t.clone(),
                                    };
                                    ui.label(RichText::new(format!("{k}: {t}")).small());
                                }
                            }
                            ui.horizontal_wrapped(|ui| {
                                if !v.datasheet_url.is_empty() {
                                    ui.hyperlink_to("Datasheet", &v.datasheet_url);
                                }
                                if !v.product_url.is_empty() {
                                    ui.hyperlink_to("Product page", &v.product_url);
                                }
                            });
                            ui.add_space(6.0);
                            let model = known_model(&v.mpn);
                            let place = mapping::placement(&v);
                            let can_place = model.is_some() || place.is_some();
                            let hint = match (&model, &place) {
                                (Some(k), _) => format!("Model {} ({})", k.number, k.def),
                                (None, Some(m)) if m.generic => format!("Generic model ({})", m.def),
                                (None, Some(m)) => m.def.to_string(),
                                (None, None) => "No simulation model: assign it to a placed part".into(),
                            };
                            ui.label(RichText::new(hint).color(c::TEXT_FAINT).small());
                            ui.horizontal(|ui| {
                                if ui.add_enabled(can_place, egui::Button::new(RichText::new("Place").strong())).clicked() {
                                    acts.push(Act::Place(v.clone()));
                                }
                                if ui
                                    .add_enabled(!selected_parts.is_empty(), egui::Button::new("Assign to selection"))
                                    .on_hover_text("Record this part as the sourcing of the selected parts")
                                    .clicked()
                                {
                                    acts.push(Act::Assign(v.clone(), selected_parts.clone()));
                                }
                            });
                            if let [one] = selected_parts.as_slice() {
                                let diff = self.param_diff(*one, &v);
                                if !diff.is_empty() {
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Apply parameters from part").color(c::TEXT_DIM).small());
                                    for (n, old, new) in &diff {
                                        ui.label(RichText::new(format!("{n}: {old} → {new}")).font(theme::mono(11.0)));
                                    }
                                    if ui.button("Apply").clicked() {
                                        acts.push(Act::Apply(*one, diff.into_iter().map(|(n, _, x)| (n, x)).collect()));
                                    }
                                }
                            }
                            if let Some(p) = v.provider {
                                ui.add_space(6.0);
                                ui.label(RichText::new(p.attribution()).color(c::TEXT_FAINT).small().italics());
                            }
                        });
                    });
                });
            });
        self.parts.open = open;
        for a in acts {
            match a {
                Act::Search => {
                    if let Some(msg) = self.parts.search(ctx) {
                        self.status = msg;
                    }
                }
                Act::Place(v) => self.place_catalog_part(&v),
                Act::Assign(v, ids) => self.assign_catalog_part(&v, &ids),
                Act::Apply(id, params) => self.apply_part_params(id, params),
                Act::Settings => self.dialog = Some("settings"),
            }
        }
    }

    fn apply_part_params(&mut self, id: Id, params: Vec<(String, String)>) {
        let Some(number) = self.doc.part(id).map(|p| p.part_number.clone()) else {
            return;
        };
        let n = params.len();
        self.edit(Command::SetPartNumber {
            part: id,
            number,
            params,
        });
        self.status = format!("Applied {n} parameter(s)");
    }

    /// The inspector's Sourcing section for one part.
    pub fn sourcing_section(&mut self, ui: &mut egui::Ui, id: Id) {
        let Some(part) = self.doc.part(id).cloned() else {
            return;
        };
        let ctx = ui.ctx().clone();
        let s = part.sourcing.clone();
        ui.add_space(6.0);
        ui.separator();
        ui.label(RichText::new("Sourcing").color(c::TEXT_DIM).small());
        let mut next = s.clone();
        egui::Grid::new(("sourcing", id))
            .num_columns(2)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                for (label, field, hint) in
                    [("MPN", 0, part.part_number.as_str()), ("Maker", 1, "")]
                {
                    ui.label(RichText::new(label).color(c::TEXT_DIM))
                        .on_hover_text(if field == 0 {
                            "Manufacturer part number"
                        } else {
                            "Manufacturer"
                        });
                    let key = format!("src-{field}-{id}");
                    let cur = if field == 0 {
                        s.mpn.clone()
                    } else {
                        s.manufacturer.clone()
                    };
                    let mut text = self
                        .inspector_edit
                        .get(&key)
                        .cloned()
                        .unwrap_or(cur.clone());
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text(hint)
                            .desired_width(150.0),
                    );
                    if r.changed() {
                        self.inspector_edit.insert(key.clone(), text.clone());
                    }
                    if r.lost_focus() {
                        self.inspector_edit.remove(&key);
                        if text.trim() != cur {
                            if field == 0 {
                                next.mpn = text.trim().to_string();
                            } else {
                                next.manufacturer = text.trim().to_string();
                            }
                        }
                    }
                    ui.end_row();
                }
                ui.label(RichText::new("Vendor").color(c::TEXT_DIM))
                    .on_hover_text("Preferred vendor (BOM)");
                let label = ProviderId::from_key(&s.preferred_vendor).map_or("none", |p| p.name());
                egui::ComboBox::from_id_salt(("pref", id))
                    .width(150.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut next.preferred_vendor, String::new(), "none");
                        for p in ProviderId::ALL {
                            ui.selectable_value(
                                &mut next.preferred_vendor,
                                p.key().to_string(),
                                p.name(),
                            );
                        }
                    });
                ui.end_row();
            });
        for v in &s.vendors {
            let name = ProviderId::from_key(&v.provider).map_or(v.provider.as_str(), |p| p.name());
            let star = if v.provider == s.preferred_vendor {
                "★ "
            } else {
                ""
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{star}{name}")).small().strong());
                if v.url.is_empty() {
                    ui.label(RichText::new(&v.vendor_pn).font(theme::mono(11.0)));
                } else {
                    ui.hyperlink_to(RichText::new(&v.vendor_pn).font(theme::mono(11.0)), &v.url);
                }
            });
            let mut facts = Vec::new();
            if let Some(pr) = v.unit_price {
                facts.push(format!(
                    "{} @{}",
                    money(pr, &v.currency),
                    v.price_qty.unwrap_or(1)
                ));
            }
            if let Some(st) = v.stock {
                facts.push(format!("stock {st}"));
            }
            if v.fetched_at > 0 {
                facts.push(age(v.fetched_at));
            }
            if !facts.is_empty() {
                ui.label(
                    RichText::new(facts.join(" · "))
                        .color(c::TEXT_FAINT)
                        .small(),
                );
            }
        }
        if generic_class(&part.def)
            && !s.mpn.is_empty()
            && known_model(&s.mpn).is_none()
            && ecs_lib::parts::find(&part.part_number)
                .is_none_or(|k| !k.number.eq_ignore_ascii_case(&s.mpn))
        {
            ui.label(
                RichText::new("generic model: the simulation uses typical values for this class")
                    .color(c::WARN)
                    .small(),
            );
        }
        if next != s {
            self.edit(Command::SetSourcing(vec![(id, next)]));
        }
        let enabled = !self.parts.allowed().is_empty();
        ui.horizontal_wrapped(|ui| {
            let mpn = if s.mpn.is_empty() {
                part.part_number.clone()
            } else {
                s.mpn.clone()
            };
            if ui
                .add_enabled(enabled, egui::Button::new("Search at vendor"))
                .on_hover_text("Open the Parts panel at the preferred vendor")
                .clicked()
            {
                self.parts.open = true;
                self.parts.provider = ProviderId::from_key(&s.preferred_vendor);
                self.parts.text = mpn.clone();
                if !mpn.is_empty() {
                    let _ = self.parts.search(&ctx);
                }
            }
            if ui
                .add_enabled(
                    enabled && !mpn.is_empty(),
                    egui::Button::new("Find at all vendors"),
                )
                .clicked()
            {
                self.find_at_vendors(&ctx, vec![id]);
            }
            if ui
                .add_enabled(
                    enabled && !s.vendors.is_empty(),
                    egui::Button::new("Refresh"),
                )
                .clicked()
            {
                self.refresh_sourcing(&ctx, id);
            }
            if ui
                .add_enabled(!s.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.edit(Command::SetSourcing(vec![(id, Sourcing::default())]));
                self.parts.found.remove(&id);
            }
        });
        if let Some(v) = self.parts.found.get(&id).cloned() {
            let diff = self.param_diff(id, &v);
            if !diff.is_empty() {
                ui.collapsing(
                    RichText::new(format!("Apply parameters from {}", v.mpn)).small(),
                    |ui| {
                        for (n, old, new) in &diff {
                            ui.label(
                                RichText::new(format!("{n}: {old} → {new}"))
                                    .font(theme::mono(11.0)),
                            );
                        }
                        if ui.button("Apply").clicked() {
                            self.apply_part_params(
                                id,
                                diff.into_iter().map(|(n, _, x)| (n, x)).collect(),
                            );
                        }
                    },
                );
            }
        }
        if !enabled {
            ui.label(
                RichText::new("Online catalogs are off (Settings).")
                    .color(c::TEXT_FAINT)
                    .small(),
            );
        }
    }

    /// The inspector for several selected parts: preferred vendor for all,
    /// and a lookup for those without vendor entries.
    pub fn sourcing_multi(&mut self, ui: &mut egui::Ui) {
        let ids = self.selected_parts();
        if ids.is_empty() {
            return;
        }
        let ctx = ui.ctx().clone();
        ui.add_space(6.0);
        ui.separator();
        ui.label(
            RichText::new(format!("Sourcing of {} parts", ids.len()))
                .color(c::TEXT_DIM)
                .small(),
        );
        let mut pick: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Preferred vendor").color(c::TEXT_DIM));
            egui::ComboBox::from_id_salt("pref-multi")
                .width(130.0)
                .selected_text("set…")
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, "none").clicked() {
                        pick = Some(String::new());
                    }
                    for p in ProviderId::ALL {
                        if ui.selectable_label(false, p.name()).clicked() {
                            pick = Some(p.key().into());
                        }
                    }
                });
        });
        if let Some(v) = pick {
            let changes: Vec<(Id, Sourcing)> = ids
                .iter()
                .filter_map(|id| {
                    let mut s = self.doc.part(*id)?.sourcing.clone();
                    s.preferred_vendor = v.clone();
                    Some((*id, s))
                })
                .collect();
            self.edit(Command::SetSourcing(changes));
        }
        let without: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|id| {
                self.doc
                    .part(*id)
                    .is_some_and(|p| p.sourcing.vendors.is_empty())
            })
            .collect();
        let enabled = !self.parts.allowed().is_empty();
        if ui
            .add_enabled(
                enabled && !without.is_empty(),
                egui::Button::new(format!("Find at all vendors ({} without)", without.len())),
            )
            .clicked()
        {
            self.find_at_vendors(&ctx, without);
        }
    }

    /// Write the BOM (and one file per preferred vendor) next to `path`.
    pub fn export_bom(&mut self, path: &Path) -> Result<Vec<PathBuf>, String> {
        let (all, per) = bom(self.root_doc(), &self.cat);
        std::fs::write(path, all).map_err(|e| e.to_string())?;
        let mut written = vec![path.to_path_buf()];
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "bom".into());
        for (vendor, text) in per {
            let p = path.with_file_name(format!("{stem}-{vendor}.csv"));
            std::fs::write(&p, text).map_err(|e| e.to_string())?;
            written.push(p);
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_from_catalog_numbers() {
        assert_eq!(
            known_model("BC547B,112").map(|k| k.number.as_str()),
            Some("BC547B")
        );
        assert_eq!(
            known_model("2N2222A").map(|k| k.def.as_str()),
            Some("core.npn")
        );
        assert_eq!(
            known_model("NE555P").map(|k| k.number.as_str()),
            Some("NE555")
        );
        assert!(known_model("SN74HC00N").is_some_and(|k| k.number.eq_ignore_ascii_case("74HC00")));
        assert!(known_model("RC0603FR-0710KL").is_none());
    }

    #[test]
    fn date_is_civil() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_794_268_800), "2026-11-10");
    }

    #[test]
    fn bom_groups_parts_and_writes_vendor_files() {
        let cat = crate::catalog::Catalog::builtin();
        let mut doc = Schematic::default();
        let r = cat.def("core.resistor").unwrap().clone();
        for (k, value) in [(0, "10k"), (1, "10k"), (2, "1k")] {
            let mut p = cat.new_part(&doc, &r, [k as f64 * 40.0, 0.0]);
            p.params.insert("R".into(), value.into());
            if value == "10k" {
                p.sourcing = Sourcing {
                    mpn: "RC0603FR-0710KL".into(),
                    manufacturer: "YAGEO".into(),
                    preferred_vendor: "jlcpcb".into(),
                    vendors: vec![VendorRef {
                        provider: "jlcpcb".into(),
                        vendor_pn: "C98220".into(),
                        unit_price: Some(0.0023),
                        price_qty: Some(1),
                        currency: "USD".into(),
                        ..Default::default()
                    }],
                };
            }
            doc.apply(Command::AddPart(Box::new(p)));
        }
        let g = cat.def("core.ground").unwrap().clone();
        doc.apply(Command::AddPart(Box::new(cat.new_part(
            &doc,
            &g,
            [0.0, 80.0],
        ))));
        let (all, per) = bom(&doc, &cat);
        let lines: Vec<&str> = all.lines().collect();
        assert_eq!(lines.len(), 3, "{all}");
        assert!(
            lines[1].starts_with(
                "2,R1 R2,Resistor,10kΩ,RC0603FR-0710KL,YAGEO,JLCPCB / LCSC,C98220,0.0023,USD,1"
            ),
            "{}",
            lines[1]
        );
        assert!(lines[2].starts_with("1,R3,Resistor,1kΩ,"), "{}", lines[2]);
        assert_eq!(per.len(), 1);
        assert_eq!(per[0].0, "jlcpcb");
        assert_eq!(
            per[0].1,
            "Comment,Designator,Footprint,LCSC Part #\n10kΩ,\"R1,R2\",,C98220\n"
        );
    }

    #[test]
    fn farnell_keeps_no_price() {
        let v = VendorPart {
            provider: Some(ProviderId::Farnell),
            vendor_pn: "1652438".into(),
            stock: Some(5),
            prices: vec![ecs_catalog::PriceBreak {
                quantity: 1,
                unit_price: 0.17,
                currency: "GBP".into(),
            }],
            fetched_at: 99,
            ..Default::default()
        };
        let r = vendor_ref(&v);
        assert_eq!((r.unit_price, r.stock, r.fetched_at), (None, None, 0));
        assert_eq!(r.vendor_pn, "1652438");
    }

    #[test]
    fn catalogs_are_off_by_default() {
        let s = crate::settings::Settings::default();
        assert!(!s.catalogs.master && s.catalogs.enabled.is_empty());
        let mut ui = PartsUi {
            text: "10k".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        assert!(ui.search(&ctx).is_some());
        assert!(ui.service.is_none());
    }
}
