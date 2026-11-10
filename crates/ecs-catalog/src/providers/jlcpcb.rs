//! JLCPCB / LCSC parts.
//!
//! * **Live search**: the community jlcsearch service by tscircuit
//!   (<https://github.com/tscircuit/jlcsearch>, MIT). `GET
//!   https://jlcsearch.tscircuit.com/api/search?q=…&limit=…` returns
//!   `{"components": [{"lcsc", "mfr", "package", "is_basic", "is_preferred",
//!   "description", "stock", "price"}]}` with `price` the unit price. No key,
//!   no stated rate limit; its README asks clients to revalidate within 24 h
//!   (nightly stock sync), so results are cached for 24 h. Unofficial: it may
//!   change or disappear.
//! * **Offline snapshot**: a jlcparts `cache.sqlite3` the user downloads from
//!   <https://yaqwsx.github.io/jlcparts/> (MIT) and selects in Settings; read
//!   locally, no network. Tables `components`, `manufacturers`, `categories`.
//! * The official JLCPCB Open API (<https://api.jlcpcb.com/>) documents its
//!   authentication only behind a developer sign-in, so it is not implemented.
//!
//! Budget: 30 requests per minute, 2000 per day (self-imposed; none stated).

use super::{Env, arr, n, s};
use crate::http::{Budget, Request};
use crate::{
    Capabilities, CatalogError, CatalogProvider, CatalogQuery, Lifecycle, PriceBreak, ProviderId,
    SearchPage, VendorPart, normalize,
};
use serde_json::Value;

const BASE: &str = "https://jlcsearch.tscircuit.com";

pub struct Jlcpcb {
    env: Env,
    budget: Budget,
}

impl Jlcpcb {
    pub fn new(env: Env) -> Self {
        Self {
            env,
            budget: Budget::new(30, 2000),
        }
    }

    fn snapshot(&self) -> Option<std::path::PathBuf> {
        let p = self.env.consent.read().ok()?.jlc_snapshot.clone();
        let path = std::path::PathBuf::from(p.trim());
        (!p.trim().is_empty() && path.is_file()).then_some(path)
    }

    async fn api(&self, q: &str, limit: u32) -> Result<Vec<VendorPart>, CatalogError> {
        let req = Request::get(&format!("{BASE}/api/search"))
            .query("q", q)
            .query("limit", limit.clamp(1, 100).to_string());
        let r = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Jlcpcb, req)
            .await?;
        let v = r.json()?;
        Ok(arr(&v, &["components"]).iter().map(part).collect())
    }
}

/// A jlcsearch component.
pub fn part(c: &Value) -> VendorPart {
    let lcsc = s(c, &["lcsc"]);
    let lcsc = if lcsc.starts_with('C') {
        lcsc
    } else {
        format!("C{lcsc}")
    };
    let mut p = VendorPart {
        provider: Some(ProviderId::Jlcpcb),
        vendor_pn: lcsc.clone(),
        mpn: s(c, &["mfr"]),
        manufacturer: s(c, &["manufacturer"]),
        description: s(c, &["description"]),
        category: [s(c, &["category"]), s(c, &["subcategory"])]
            .iter()
            .filter(|x| !x.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" / "),
        package: s(c, &["package"]),
        stock: n(c, &["stock"]).map(|x| x as i64).or(Some(0)),
        lifecycle: Lifecycle::Unknown,
        product_url: format!("https://www.lcsc.com/product-detail/{lcsc}.html"),
        jlc_basic: c["is_basic"].as_bool(),
        fetched_at: crate::now(),
        ..Default::default()
    };
    match &c["price"] {
        Value::Number(x) => p.prices.push(PriceBreak {
            quantity: 1,
            unit_price: x.as_f64().unwrap_or(0.0),
            currency: "USD".into(),
        }),
        // "1-49:0.3039,50-149:0.2371,…"
        Value::String(tiers) => {
            for t in tiers.split(',') {
                if let Some((range, price)) = t.split_once(':') {
                    let q = range
                        .split('-')
                        .next()
                        .and_then(|x| x.trim().parse::<u32>().ok())
                        .unwrap_or(1);
                    if let Ok(v) = price.trim().parse::<f64>() {
                        p.prices.push(PriceBreak {
                            quantity: q,
                            unit_price: v,
                            currency: "USD".into(),
                        });
                    }
                }
            }
        }
        _ => {}
    }
    if !p.package.is_empty() {
        p.params
            .insert("package".into(), crate::ParamValue::Text(p.package.clone()));
    }
    normalize::from_description(&mut p);
    p
}

/// Search a jlcparts snapshot (SQLite) for every word of `text`.
pub fn snapshot_search(
    path: &std::path::Path,
    text: &str,
    limit: u32,
) -> Result<Vec<VendorPart>, CatalogError> {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| CatalogError::Parse(format!("snapshot: {e}")))?;
    let words: Vec<String> = text
        .split_whitespace()
        .take(6)
        .map(|w| format!("%{w}%"))
        .collect();
    let mut sql = String::from(
        "SELECT c.lcsc, c.mfr, c.package, c.basic, c.description, c.datasheet, c.stock, c.price, m.name, cat.category, cat.subcategory \
         FROM components c LEFT JOIN manufacturers m ON c.manufacturer_id = m.id LEFT JOIN categories cat ON c.category_id = cat.id WHERE 1",
    );
    for k in 0..words.len() {
        sql.push_str(&format!(
            " AND (c.mfr LIKE ?{0} OR c.description LIKE ?{0} OR ('C' || c.lcsc) LIKE ?{0})",
            k + 1
        ));
    }
    sql.push_str(&format!(
        " ORDER BY c.stock DESC LIMIT {}",
        limit.clamp(1, 200)
    ));
    let mut stmt = db.prepare(&sql).map_err(|e| {
        CatalogError::Parse(format!(
            "snapshot schema not recognised (jlcparts cache.sqlite3 expected): {e}"
        ))
    })?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(words.iter()), |r| {
            Ok(serde_json::json!({
                "lcsc": r.get::<_, i64>(0)?,
                "mfr": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "package": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "is_basic": r.get::<_, Option<i64>>(3)?.unwrap_or(0) == 1,
                "description": r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                "datasheet": r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                "stock": r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                "price_json": r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                "manufacturer": r.get::<_, Option<String>>(8)?.unwrap_or_default(),
                "category": r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                "subcategory": r.get::<_, Option<String>>(10)?.unwrap_or_default(),
            }))
        })
        .map_err(|e| CatalogError::Parse(e.to_string()))?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let mut p = part(&row);
        p.datasheet_url = s(&row, &["datasheet"]);
        // jlcparts prices: [{"qFrom": 1, "qTo": 9, "price": 0.0123}, …]
        if let Ok(Value::Array(tiers)) = serde_json::from_str::<Value>(&s(&row, &["price_json"])) {
            p.prices = tiers
                .iter()
                .filter_map(|t| {
                    Some(PriceBreak {
                        quantity: n(t, &["qFrom"])? as u32,
                        unit_price: n(t, &["price"])?,
                        currency: "USD".into(),
                    })
                })
                .collect();
        }
        out.push(p);
    }
    Ok(out)
}

#[async_trait::async_trait]
impl CatalogProvider for Jlcpcb {
    fn id(&self) -> ProviderId {
        ProviderId::Jlcpcb
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            keyword: true,
            mpn: true,
            parametric: false,
            price: true,
            stock: true,
        }
    }
    async fn search(&self, q: &CatalogQuery) -> Result<SearchPage, CatalogError> {
        let parts = match self.snapshot() {
            Some(path) => snapshot_search(&path, &q.text, q.limit.max(1))?,
            None => self.api(&q.text, q.limit.max(1)).await?,
        };
        Ok(SearchPage { total: None, parts })
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let want = pn.trim().to_uppercase();
        let want = if want.starts_with('C') {
            want
        } else {
            format!("C{want}")
        };
        let parts = match self.snapshot() {
            Some(path) => snapshot_search(&path, &want, 10)?,
            None => self.api(&want, 10).await?,
        };
        parts
            .into_iter()
            .find(|p| p.vendor_pn.eq_ignore_ascii_case(&want))
            .ok_or(CatalogError::NotFound)
    }
    async fn by_mpn(&self, mpn: &str, _mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let parts = match self.snapshot() {
            Some(path) => snapshot_search(&path, mpn, 20)?,
            None => self.api(mpn, 20).await?,
        };
        Ok(parts
            .into_iter()
            .filter(|p| p.mpn.eq_ignore_ascii_case(mpn.trim()))
            .collect())
    }
}
