//! Online part catalogs (differential spec ADD-01): JLCPCB/LCSC, DigiKey,
//! Mouser, TME and Farnell behind one trait, a consent gate, a request
//! budget and a cache. Nothing is sent to any provider unless the user has
//! allowed online catalogs **and** that provider in Settings; a call to a
//! provider that is not approved fails with [`CatalogError::NotApproved`]
//! without touching the network.
//!
//! The app ships no credentials and no catalog data: keys come from the
//! user and live in the operating system keychain ([`credentials`]).

pub mod cache;
pub mod credentials;
pub mod http;
pub mod mapping;
pub mod normalize;
pub mod providers;
pub mod service;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The five providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Jlcpcb,
    Digikey,
    Mouser,
    Tme,
    Farnell,
}

impl ProviderId {
    pub const ALL: [ProviderId; 5] = [
        ProviderId::Jlcpcb,
        ProviderId::Digikey,
        ProviderId::Mouser,
        ProviderId::Tme,
        ProviderId::Farnell,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ProviderId::Jlcpcb => "jlcpcb",
            ProviderId::Digikey => "digikey",
            ProviderId::Mouser => "mouser",
            ProviderId::Tme => "tme",
            ProviderId::Farnell => "farnell",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Jlcpcb => "JLCPCB / LCSC",
            ProviderId::Digikey => "DigiKey",
            ProviderId::Mouser => "Mouser",
            ProviderId::Tme => "TME",
            ProviderId::Farnell => "Farnell / Newark / element14",
        }
    }
    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
    }
    /// What a request to this provider sends (shown before approval).
    pub fn sends(self) -> &'static str {
        match self {
            ProviderId::Jlcpcb => {
                "Search text and part numbers, to jlcsearch.tscircuit.com (a community service, unofficial, may change)."
            }
            ProviderId::Digikey => {
                "Search text and part numbers, your client ID and secret (for a token), to api.digikey.com."
            }
            ProviderId::Mouser => {
                "Search text and part numbers, and your API key, to api.mouser.com."
            }
            ProviderId::Tme => {
                "Search text and part symbols, your token and application secret (for an access token), to api.tme.eu."
            }
            ProviderId::Farnell => {
                "Search text and part numbers, your API key and store, to api.element14.com."
            }
        }
    }
    pub fn terms_url(self) -> &'static str {
        match self {
            ProviderId::Jlcpcb => "https://github.com/tscircuit/jlcsearch",
            ProviderId::Digikey => "https://developer.digikey.com/api-user-agreement",
            ProviderId::Mouser => "https://www.mouser.com/api-hub/",
            ProviderId::Tme => "https://developers.tme.eu/en/terms",
            ProviderId::Farnell => "https://partner.element14.com/terms",
        }
    }
    /// Attribution the provider's terms require next to its data.
    pub fn attribution(self) -> &'static str {
        match self {
            ProviderId::Jlcpcb => "Data from JLCPCB/LCSC via jlcsearch (community service)",
            ProviderId::Digikey => "Data from DigiKey",
            ProviderId::Mouser => "Data from Mouser Electronics",
            ProviderId::Tme => {
                "Data powered by TME.eu Data – no guarantee of data accuracy. Rights to the content belong to TME, the manufacturers or their owners."
            }
            ProviderId::Farnell => "Content from Farnell (element14)",
        }
    }
    /// Credential fields the provider needs (names in the keychain).
    pub fn credential_fields(self) -> &'static [(&'static str, &'static str)] {
        match self {
            ProviderId::Jlcpcb => &[],
            ProviderId::Digikey => &[
                ("client_id", "Client ID"),
                ("client_secret", "Client secret"),
            ],
            ProviderId::Mouser => &[("api_key", "API key")],
            ProviderId::Tme => &[
                ("token", "Token (50 characters)"),
                ("secret", "Application secret"),
            ],
            ProviderId::Farnell => &[("api_key", "API key")],
        }
    }
    /// Whether price and stock may be kept (cache, schematic). Farnell's
    /// terms forbid storing any of its content.
    pub fn may_store_prices(self) -> bool {
        self != ProviderId::Farnell
    }
}

/// What a provider supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub keyword: bool,
    pub mpn: bool,
    pub parametric: bool,
    pub price: bool,
    pub stock: bool,
}

/// A search.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CatalogQuery {
    pub text: String,
    pub limit: u32,
    pub offset: u32,
    pub in_stock: bool,
    /// Case-insensitive substring filters applied to results.
    pub category: String,
    pub package: String,
    /// Canonical parameter ranges (key, min, max).
    pub ranges: Vec<(String, Option<f64>, Option<f64>)>,
    /// Hide parts that are not active.
    pub active_only: bool,
}

impl CatalogQuery {
    pub fn text(s: &str) -> Self {
        Self {
            text: s.into(),
            limit: 20,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Unknown,
    Active,
    NotRecommended,
    Obsolete,
}

impl Lifecycle {
    pub fn from_text(s: &str) -> Self {
        let l = s.to_ascii_lowercase();
        if l.is_empty() {
            Lifecycle::Unknown
        } else if l.contains("obsolete")
            || l.contains("discontinu")
            || l.contains("end of life")
            || l.contains("last time")
        {
            Lifecycle::Obsolete
        } else if l.contains("not recommended") || l.contains("nrnd") {
            Lifecycle::NotRecommended
        } else if l.contains("active")
            || l.contains("new product")
            || l.contains("production")
            || l.contains("defaultstatus")
            || l.contains("available")
        {
            Lifecycle::Active
        } else {
            Lifecycle::Unknown
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Lifecycle::Unknown => "unknown",
            Lifecycle::Active => "active",
            Lifecycle::NotRecommended => "not for new designs",
            Lifecycle::Obsolete => "obsolete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceBreak {
    pub quantity: u32,
    pub unit_price: f64,
    pub currency: String,
}

/// A parameter value: canonical (SI number) or raw text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Number(f64),
    Text(String),
}

/// A part as one provider offers it, normalized.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct VendorPart {
    pub provider: Option<ProviderId>,
    pub vendor_pn: String,
    pub mpn: String,
    pub manufacturer: String,
    pub description: String,
    pub category: String,
    pub package: String,
    /// Canonical keys (resistance, capacitance...) with SI values, plus raw attributes.
    pub params: BTreeMap<String, ParamValue>,
    pub prices: Vec<PriceBreak>,
    pub stock: Option<i64>,
    pub lead_time: String,
    pub lifecycle: Lifecycle,
    pub rohs: String,
    pub datasheet_url: String,
    pub product_url: String,
    /// JLCPCB assembly class: Basic or Extended.
    pub jlc_basic: Option<bool>,
    /// Seconds since the Unix epoch.
    pub fetched_at: u64,
}

impl VendorPart {
    /// Unit price at a quantity (the break that applies).
    pub fn price_at(&self, qty: u32) -> Option<&PriceBreak> {
        self.prices
            .iter()
            .filter(|p| p.quantity <= qty.max(1))
            .max_by_key(|p| p.quantity)
            .or_else(|| self.prices.iter().min_by_key(|p| p.quantity))
    }
    pub fn number(&self, key: &str) -> Option<f64> {
        match self.params.get(key) {
            Some(ParamValue::Number(v)) => Some(*v),
            _ => None,
        }
    }
}

/// One page of results.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SearchPage {
    pub parts: Vec<VendorPart>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CatalogError {
    /// Online catalogs or this provider are not approved in Settings.
    NotApproved(ProviderId),
    MissingCredentials(ProviderId),
    BadCredentials(String),
    Quota(String),
    Offline(String),
    Http(u16, String),
    Parse(String),
    NotFound,
    Unsupported(&'static str),
    Cancelled,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::NotApproved(p) => write!(
                f,
                "{} is not enabled in Settings › Online catalogs",
                p.name()
            ),
            CatalogError::MissingCredentials(p) => write!(
                f,
                "{} needs credentials in Settings › Online catalogs",
                p.name()
            ),
            CatalogError::BadCredentials(m) => write!(f, "credentials rejected: {m}"),
            CatalogError::Quota(m) => write!(f, "request quota reached: {m}"),
            CatalogError::Offline(m) => write!(f, "network error: {m}"),
            CatalogError::Http(c, m) => write!(f, "HTTP {c}: {m}"),
            CatalogError::Parse(m) => write!(f, "unexpected response: {m}"),
            CatalogError::NotFound => write!(f, "no such part"),
            CatalogError::Unsupported(m) => write!(f, "not supported: {m}"),
            CatalogError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// A catalog provider.
#[async_trait::async_trait]
pub trait CatalogProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError>;
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError>;
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError>;
}

/// Consent and provider settings, kept in the user's settings (never in
/// schematics). Credentials are not here: they live in the keychain.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Consent {
    /// "Allow online part catalogs" (off by default).
    pub master: bool,
    /// Providers the user enabled (and confirmed).
    pub enabled: Vec<ProviderId>,
    /// Store or region per provider (Farnell store id, DigiKey site/currency...).
    pub region: BTreeMap<String, String>,
    /// Path of a downloaded jlcparts snapshot (SQLite), if any.
    pub jlc_snapshot: String,
}

impl Consent {
    pub fn allows(&self, p: ProviderId) -> bool {
        self.master && self.enabled.contains(&p)
    }
    pub fn region(&self, p: ProviderId, default: &str) -> String {
        self.region
            .get(p.key())
            .cloned()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    }
}

/// Shared, live consent: the gate reads it before every call.
pub type SharedConsent = Arc<std::sync::RwLock<Consent>>;

/// Wraps a provider and refuses calls the user has not approved.
pub struct ConsentGate {
    inner: Box<dyn CatalogProvider>,
    consent: SharedConsent,
}

impl ConsentGate {
    pub fn new(inner: Box<dyn CatalogProvider>, consent: SharedConsent) -> Self {
        Self { inner, consent }
    }
    fn check(&self) -> Result<(), CatalogError> {
        let ok = self
            .consent
            .read()
            .map(|c| c.allows(self.inner.id()))
            .unwrap_or(false);
        if ok {
            Ok(())
        } else {
            Err(CatalogError::NotApproved(self.inner.id()))
        }
    }
}

#[async_trait::async_trait]
impl CatalogProvider for ConsentGate {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError> {
        self.check()?;
        self.inner.search(q).await
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        self.check()?;
        self.inner.by_vendor_pn(pn).await
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        self.check()?;
        self.inner.by_mpn(mpn, mfr).await
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Apply the query's local filters (in stock, category, package, ranges, lifecycle).
pub fn filter(parts: Vec<VendorPart>, q: &CatalogQuery) -> Vec<VendorPart> {
    parts
        .into_iter()
        .filter(|p| !q.in_stock || p.stock.unwrap_or(0) > 0)
        .filter(|p| {
            q.category.is_empty()
                || p.category
                    .to_lowercase()
                    .contains(&q.category.to_lowercase())
        })
        .filter(|p| {
            q.package.is_empty() || p.package.to_lowercase().contains(&q.package.to_lowercase())
        })
        .filter(|p| !q.active_only || matches!(p.lifecycle, Lifecycle::Active | Lifecycle::Unknown))
        .filter(|p| {
            q.ranges.iter().all(|(k, lo, hi)| match p.number(k) {
                Some(v) => lo.is_none_or(|l| v >= l) && hi.is_none_or(|h| v <= h),
                None => lo.is_none() && hi.is_none(),
            })
        })
        .collect()
}

/// Group parts from several providers by manufacturer part number (spec 4,
/// "All enabled"); each part keeps its provider, so sources stay distinct.
pub fn group_by_mpn(parts: &[VendorPart]) -> Vec<(String, Vec<&VendorPart>)> {
    let mut groups: Vec<(String, Vec<&VendorPart>)> = Vec::new();
    for p in parts {
        let key = p.mpn.trim().to_uppercase();
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.1.push(p),
            None => groups.push((key, vec![p])),
        }
    }
    groups
}
