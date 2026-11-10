//! TME API v2 (OpenAPI at <https://developers.tme.eu/openapi.json>, "V2.0
//! Current Version"). Applications created after May 14, 2026 only work with
//! this version; the older signed API (`ApiSignature`) is not used.
//!
//! * Token: `POST https://api.tme.eu/auth/token`, HTTP Basic with the
//!   50-character token as user and the application secret as password,
//!   form `grant_type=client_credentials`; `access_token`, `expires_in` 300 s.
//! * `GET /products/search?phrase=…&scope[]=products&limit=…&page=…&country=…`
//!   → `data.products.elements[]` (`symbol`, `manufacturer_symbols`,
//!   `manufacturer.name`, `description`, `category.name`, `product_status`).
//! * `GET /products?symbols[]=…` or `?mpns[]=…` (≤ 50) → `data.elements[]`.
//! * `GET /products/data?symbols[]=…&scope[]=prices&scope[]=stock&currency=…`
//!   → `data.elements[]` (`symbol`, `stock_quantity`, `prices.elements[]`
//!   with `amount`, `price`; `prices.currency`).
//! * `GET /products/parameters?symbols[]=…` → `data.elements[]` with
//!   `parameters.elements[]` (`name`, `values[].value`).
//! * Limits: 50 symbols per request; request rates are not stated, so the
//!   client keeps to 30 per minute and 2000 per day.
//! * Terms (2026-07-01, <https://developers.tme.eu/en/terms>): content may be
//!   used in native desktop applications for personal use (8.5); shown data
//!   must carry "Data powered by TME.eu Data – no guarantee of data
//!   accuracy" (8.7) and a notice that the rights belong to TME, the
//!   manufacturers or their owners (8.8); no caching lifetime is stated
//!   (cache: 24 h for prices and stock, 30 days for parameters).
//!
//! Region setting: `COUNTRY,CURRENCY,LANGUAGE` (default `GB,EUR,en`).

use super::{Env, arr, n, s};
use crate::http::{Budget, Request};
use crate::{
    Capabilities, CatalogError, CatalogProvider, CatalogQuery, Lifecycle, PriceBreak, ProviderId,
    SearchPage, VendorPart, normalize,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://api.tme.eu";

pub struct Tme {
    env: Env,
    budget: Budget,
    token: Mutex<Option<(String, Instant)>>,
}

impl Tme {
    pub fn new(env: Env) -> Self {
        Self {
            env,
            budget: Budget::new(30, 2000),
            token: Mutex::new(None),
        }
    }

    fn region(&self) -> (String, String, String) {
        let r = self.env.region(ProviderId::Tme, "GB,EUR,en");
        let mut it = r.split(',').map(|x| x.trim().to_string());
        (
            it.next().unwrap_or("GB".into()),
            it.next().unwrap_or("EUR".into()),
            it.next().unwrap_or("en".into()),
        )
    }

    async fn token(&self) -> Result<String, CatalogError> {
        if let Ok(t) = self.token.lock()
            && let Some((tok, until)) = t.as_ref()
            && Instant::now() < *until
        {
            return Ok(tok.clone());
        }
        let (token, secret) = (
            self.env.cred(ProviderId::Tme, "token")?,
            self.env.cred(ProviderId::Tme, "secret")?,
        );
        let req = Request::post(&format!("{BASE}/auth/token"))
            .basic(&token, &secret)
            .form("grant_type", "client_credentials");
        let v = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Tme, req)
            .await?
            .json()?;
        let tok = s(&v, &["access_token"]);
        if tok.is_empty() {
            return Err(CatalogError::BadCredentials(
                "no access_token in the token response".into(),
            ));
        }
        let life = n(&v, &["expires_in"]).unwrap_or(300.0).max(60.0) - 20.0;
        if let Ok(mut t) = self.token.lock() {
            *t = Some((tok.clone(), Instant::now() + Duration::from_secs_f64(life)));
        }
        Ok(tok)
    }

    async fn get(&self, path: &str, query: Vec<(&str, String)>) -> Result<Value, CatalogError> {
        let tok = self.token().await?;
        let (country, _, lang) = self.region();
        let mut req = Request::get(&format!("{BASE}{path}"))
            .header("Authorization", format!("Bearer {tok}"))
            .header("Accept-Language", lang)
            .query("country", country);
        for (k, v) in query {
            req = req.query(k, v);
        }
        let v = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Tme, req)
            .await?
            .json()?;
        if !s(&v, &["status"]).eq_ignore_ascii_case("OK") && !v["status"].is_null() {
            return Err(CatalogError::Http(
                200,
                format!("TME status {}", s(&v, &["status"])),
            ));
        }
        Ok(v)
    }

    /// Prices, stock and parameters for a set of products.
    async fn complete(&self, mut parts: Vec<VendorPart>) -> Result<Vec<VendorPart>, CatalogError> {
        if parts.is_empty() {
            return Ok(parts);
        }
        let symbols: Vec<String> = parts.iter().take(50).map(|p| p.vendor_pn.clone()).collect();
        let (_, currency, _) = self.region();
        let mut q: Vec<(&str, String)> = symbols.iter().map(|x| ("symbols[]", x.clone())).collect();
        q.push(("scope[]", "prices".into()));
        q.push(("scope[]", "stock".into()));
        q.push(("currency", currency));
        let data = self.get("/products/data", q).await?;
        let mut by_symbol: BTreeMap<String, &Value> = BTreeMap::new();
        for e in arr(&data, &["data", "elements"]) {
            by_symbol.insert(s(e, &["symbol"]), e);
        }
        let pq: Vec<(&str, String)> = symbols.iter().map(|x| ("symbols[]", x.clone())).collect();
        let params = self
            .env
            .cache
            .get(ProviderId::Tme, "params", &symbols.join(","))
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let params = match params {
            Some(p) => p,
            None => {
                let p = self.get("/products/parameters", pq).await?;
                self.env.cache.put(
                    ProviderId::Tme,
                    "params",
                    &symbols.join(","),
                    &p.to_string(),
                );
                p
            }
        };
        let mut params_by: BTreeMap<String, &Value> = BTreeMap::new();
        for e in arr(&params, &["data", "elements"]) {
            params_by.insert(s(e, &["symbol"]), e);
        }
        for p in &mut parts {
            if let Some(e) = by_symbol.get(&p.vendor_pn) {
                p.stock = n(e, &["stock_quantity"]).map(|x| x as i64);
                let cur = s(e, &["prices", "currency"]);
                p.prices = arr(e, &["prices", "elements"])
                    .iter()
                    .filter_map(|b| {
                        Some(PriceBreak {
                            quantity: n(b, &["amount"])? as u32,
                            unit_price: n(b, &["price"])?,
                            currency: cur.clone(),
                        })
                    })
                    .collect();
            }
            if let Some(e) = params_by.get(&p.vendor_pn) {
                for prm in arr(e, &["parameters", "elements"]) {
                    let values: Vec<String> = arr(prm, &["values"])
                        .iter()
                        .map(|v| s(v, &["value"]))
                        .collect();
                    normalize::add(p, &s(prm, &["name"]), &values.join(", "));
                }
            }
        }
        Ok(parts)
    }
}

/// A TME product element (search or /products).
pub fn part(e: &Value) -> VendorPart {
    let symbol = s(e, &["symbol"]);
    let mpn = match &e["manufacturer_symbols"] {
        Value::Array(a) => a.first().map(|x| match x {
            Value::String(s) => s.clone(),
            other => s(other, &["symbol"]),
        }),
        Value::String(x) => Some(x.clone()),
        _ => None,
    }
    .unwrap_or_default();
    let status: Vec<String> = arr(e, &["product_status"])
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let lifecycle = if status.iter().any(|x| {
        x.contains("CANNOT_BE_ORDERED") || x.contains("DISCONTINUED") || x.contains("WITHDRAWN")
    }) {
        Lifecycle::Obsolete
    } else if status
        .iter()
        .any(|x| x.contains("AVAILABLE_WHILE_STOCKS_LAST") || x.contains("NOT_RECOMMENDED"))
    {
        Lifecycle::NotRecommended
    } else {
        Lifecycle::Active
    };
    let mut p = VendorPart {
        provider: Some(ProviderId::Tme),
        vendor_pn: symbol.clone(),
        mpn,
        manufacturer: s(e, &["manufacturer", "name"]),
        description: s(e, &["description"]),
        category: s(e, &["category", "name"]),
        lifecycle,
        product_url: format!("https://www.tme.eu/en/details/{}/", symbol.to_lowercase()),
        fetched_at: crate::now(),
        ..Default::default()
    };
    normalize::from_description(&mut p);
    p
}

#[async_trait::async_trait]
impl CatalogProvider for Tme {
    fn id(&self) -> ProviderId {
        ProviderId::Tme
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            keyword: true,
            mpn: true,
            parametric: true,
            price: true,
            stock: true,
        }
    }
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError> {
        let phrase: String = q.text.chars().take(40).collect();
        if phrase.trim().chars().count() < 2 {
            return Ok(SearchPage::default());
        }
        let limit = q.limit.clamp(1, 50);
        let page = q.offset / limit + 1;
        let mut query = vec![
            ("phrase", phrase),
            ("scope[]", "products".into()),
            ("scope[]", "counters".into()),
            ("limit", limit.to_string()),
            ("page", page.to_string()),
        ];
        if q.in_stock {
            query.push(("filter", "in_stock".into()));
        }
        let v = self.get("/products/search", query).await?;
        let parts: Vec<VendorPart> = arr(&v, &["data", "products", "elements"])
            .iter()
            .map(part)
            .collect();
        let total = n(&v, &["data", "counters", "count"]).map(|x| x as u64);
        Ok(SearchPage {
            total,
            parts: self.complete(parts).await?,
        })
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let v = self
            .get("/products", vec![("symbols[]", pn.trim().to_string())])
            .await?;
        let parts: Vec<VendorPart> = arr(&v, &["data", "elements"]).iter().map(part).collect();
        self.complete(parts)
            .await?
            .into_iter()
            .next()
            .ok_or(CatalogError::NotFound)
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let v = self
            .get("/products", vec![("mpns[]", mpn.trim().to_string())])
            .await?;
        let parts: Vec<VendorPart> = arr(&v, &["data", "elements"])
            .iter()
            .map(part)
            .filter(|p| {
                mfr.is_none_or(|m| p.manufacturer.to_lowercase().contains(&m.to_lowercase()))
            })
            .collect();
        self.complete(parts).await
    }
}
