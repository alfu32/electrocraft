//! The five providers. Each module's documentation records the endpoints,
//! limits and terms read on Nov 10, 2026.

pub mod digikey;
pub mod farnell;
pub mod jlcpcb;
pub mod mouser;
pub mod tme;

use crate::cache::Cache;
use crate::credentials::Store;
use crate::http::Transport;
use crate::{
    CatalogError, CatalogProvider, CatalogQuery, ConsentGate, ProviderId, SearchPage,
    SharedConsent, VendorPart,
};
use serde_json::Value;
use std::sync::Arc;

/// What providers share.
#[derive(Clone)]
pub struct Env {
    pub transport: Arc<dyn Transport>,
    pub creds: Arc<dyn Store>,
    pub consent: SharedConsent,
    pub cache: Arc<Cache>,
}

impl Env {
    pub fn cred(&self, p: ProviderId, field: &str) -> Result<String, CatalogError> {
        self.creds
            .get(p, field)
            .filter(|v| !v.trim().is_empty())
            .ok_or(CatalogError::MissingCredentials(p))
    }
    pub fn region(&self, p: ProviderId, default: &str) -> String {
        self.consent
            .read()
            .map(|c| c.region(p, default))
            .unwrap_or_else(|_| default.to_string())
    }
}

/// Every provider, cached and behind the consent gate (outermost).
pub fn all(env: &Env) -> Vec<Arc<ConsentGate>> {
    let raw: Vec<Box<dyn CatalogProvider>> = vec![
        Box::new(jlcpcb::Jlcpcb::new(env.clone())),
        Box::new(digikey::Digikey::new(env.clone())),
        Box::new(mouser::Mouser::new(env.clone())),
        Box::new(tme::Tme::new(env.clone())),
        Box::new(farnell::Farnell::new(env.clone())),
    ];
    raw.into_iter()
        .map(|p| {
            Arc::new(ConsentGate::new(
                Box::new(Cached {
                    inner: p,
                    cache: env.cache.clone(),
                }),
                env.consent.clone(),
            ))
        })
        .collect()
}

/// Cache in front of a provider (lifetimes in [`crate::cache`]).
pub struct Cached {
    inner: Box<dyn CatalogProvider>,
    cache: Arc<Cache>,
}

#[async_trait::async_trait]
impl CatalogProvider for Cached {
    fn id(&self) -> ProviderId {
        self.inner.id()
    }
    fn capabilities(&self) -> crate::Capabilities {
        self.inner.capabilities()
    }
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError> {
        let key = format!(
            "{}|{}|{}|{}",
            q.text.trim().to_lowercase(),
            q.limit,
            q.offset,
            q.in_stock
        );
        if let Some(hit) = self
            .cache
            .get(self.id(), "search", &key)
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            return Ok(hit);
        }
        let page = self.inner.search(q).await?;
        if let Ok(s) = serde_json::to_string(&page) {
            self.cache.put(self.id(), "search", &key, &s);
        }
        Ok(page)
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let key = format!("pn|{}", pn.trim().to_uppercase());
        if let Some(hit) = self
            .cache
            .get(self.id(), "part", &key)
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            return Ok(hit);
        }
        let part = self.inner.by_vendor_pn(pn).await?;
        if let Ok(s) = serde_json::to_string(&part) {
            self.cache.put(self.id(), "part", &key, &s);
        }
        Ok(part)
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let key = format!(
            "mpn|{}|{}",
            mpn.trim().to_uppercase(),
            mfr.unwrap_or("").to_uppercase()
        );
        if let Some(hit) = self
            .cache
            .get(self.id(), "part", &key)
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            return Ok(hit);
        }
        let parts = self.inner.by_mpn(mpn, mfr).await?;
        if let Ok(s) = serde_json::to_string(&parts) {
            self.cache.put(self.id(), "part", &key, &s);
        }
        Ok(parts)
    }
}

// JSON helpers.
pub(crate) fn s(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for k in path {
        cur = &cur[*k];
    }
    match cur {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}
pub(crate) fn n(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for k in path {
        cur = &cur[*k];
    }
    match cur {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_number(s),
        _ => None,
    }
}
/// A number inside text: "$0.10", "1,234 In Stock", "0,647 €".
pub(crate) fn parse_number(s: &str) -> Option<f64> {
    let t: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    if t.is_empty() {
        return None;
    }
    // "1,234.5" → thousands; "0,647" → decimal comma.
    let t = if t.contains('.') {
        t.replace(',', "")
    } else if t.matches(',').count() == 1 && t.split(',').nth(1).is_some_and(|d| d.len() != 3) {
        t.replace(',', ".")
    } else {
        t.replace(',', "")
    };
    t.parse().ok()
}
pub(crate) fn arr<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    let mut cur = v;
    for k in path {
        cur = &cur[*k];
    }
    cur.as_array().map(Vec::as_slice).unwrap_or(&[])
}
