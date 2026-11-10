//! element14 Product Search API (Farnell, Newark, element14;
//! <https://partner.element14.com/docs/Product_Search_API_REST__Description>).
//!
//! * `GET https://api.element14.com/catalog/products?term=…&storeInfo.id=…`
//!   `&resultsSettings.offset=…&resultsSettings.numberOfResults=…`
//!   `&resultsSettings.responseGroup=large&callInfo.responseDataFormat=json`
//!   `&callInfo.apiKey=…`; terms `any:` (keyword), `id:` (element14 part
//!   number), `manuPartNum:` (manufacturer part number); optional
//!   `resultsSettings.refinements.filters=inStock`.
//! * Keyword responses: `keywordSearchReturn.numberOfResults`, `.products[]`
//!   with `sku`, `displayName`, `translatedManufacturerPartNumber`,
//!   `brandName`, `vendorName`, `stock.level`, `stock.leastLeadTime`,
//!   `prices[]` (`from`, `to`, `cost`), `datasheets[].url`, `attributes[]`
//!   (`attributeLabel`, `attributeValue`, `attributeUnit`), `productStatus`,
//!   `rohsStatusCode`. The wrappers of `id:` and `manuPartNum:` responses are
//!   not documented, so any top-level object holding `products` is read.
//! * Store ids: `uk.farnell.com`, `de.farnell.com`, …, `www.newark.com`,
//!   `canada.newark.com`, `sg.element14.com`, …. Neither rate limits nor a
//!   maximum page size are stated; the client keeps to 20 requests per
//!   minute, 1000 per day and 25 results per page.
//! * Terms (API Terms of Use v1.1, <https://partner.element14.com/terms>):
//!   4A forbids caching, recording or storing any Farnell Content, so these
//!   results are never cached and schematics keep only the order code and
//!   link (no price or stock); content must be attributed to Farnell (5) and
//!   not merged with third-party content without distinction (1).
//!
//! Region setting: the store id, optionally `,CURRENCY` (default
//! `uk.farnell.com`, currency from the store).

use super::{Env, arr, n, s};
use crate::http::{Budget, Request};
use crate::{
    Capabilities, CatalogError, CatalogProvider, CatalogQuery, Lifecycle, PriceBreak, ProviderId,
    SearchPage, VendorPart, normalize,
};
use serde_json::Value;

const BASE: &str = "https://api.element14.com/catalog/products";

pub struct Farnell {
    env: Env,
    budget: Budget,
}

/// Currency of a store, unless the region names one.
pub fn currency_of(store: &str) -> &'static str {
    let s = store.to_lowercase();
    match s.split('.').next().unwrap_or("") {
        "uk" | "cpc" | "onecall" => "GBP",
        "www" | "mexico" if s.contains("newark") => "USD",
        "canada" => "CAD",
        "ch" => "CHF",
        "dk" => "DKK",
        "se" => "SEK",
        "no" => "NOK",
        "pl" => "PLN",
        "cz" => "CZK",
        "hu" => "HUF",
        "ro" => "RON",
        "bg" => "BGN",
        "il" => "ILS",
        "tr" => "TRY",
        "au" => "AUD",
        "nz" => "NZD",
        "sg" => "SGD",
        "hk" => "HKD",
        "in" => "INR",
        "cn" => "CNY",
        "my" => "MYR",
        "th" => "THB",
        "ph" => "PHP",
        "tw" => "TWD",
        "kr" => "KRW",
        "vn" => "VND",
        _ => "EUR",
    }
}

impl Farnell {
    pub fn new(env: Env) -> Self {
        Self {
            env,
            budget: Budget::new(20, 1000),
        }
    }
    fn store(&self) -> (String, String) {
        let r = self.env.region(ProviderId::Farnell, "uk.farnell.com");
        let mut it = r.split(',').map(|x| x.trim().to_string());
        let store = it
            .next()
            .filter(|x| !x.is_empty())
            .unwrap_or("uk.farnell.com".into());
        let cur = it
            .next()
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| currency_of(&store).into());
        (store, cur)
    }
    async fn call(
        &self,
        term: &str,
        offset: u32,
        count: u32,
        in_stock: bool,
    ) -> Result<(Vec<VendorPart>, Option<u64>), CatalogError> {
        let key = self.env.cred(ProviderId::Farnell, "api_key")?;
        let (store, cur) = self.store();
        let mut req = Request::get(BASE)
            .query("term", term)
            .query("storeInfo.id", store.clone())
            .query("resultsSettings.offset", offset.to_string())
            .query(
                "resultsSettings.numberOfResults",
                count.clamp(1, 25).to_string(),
            )
            .query("resultsSettings.responseGroup", "large")
            .query("callInfo.responseDataFormat", "json")
            .query("callInfo.apiKey", key);
        if in_stock {
            req = req.query("resultsSettings.refinements.filters", "inStock");
        }
        let v = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Farnell, req)
            .await?
            .json()?;
        if let Some(f) = v.get("Fault").or_else(|| v.get("fault")) {
            return Err(CatalogError::Http(200, f.to_string()));
        }
        let wrapper = v
            .as_object()
            .and_then(|o| {
                o.values()
                    .find(|x| x.get("products").is_some() || x.get("numberOfResults").is_some())
            })
            .cloned()
            .unwrap_or(Value::Null);
        let parts = arr(&wrapper, &["products"])
            .iter()
            .map(|p| part(p, &store, &cur))
            .collect();
        Ok((parts, n(&wrapper, &["numberOfResults"]).map(|x| x as u64)))
    }
}

/// An element14 product.
pub fn part(p: &Value, store: &str, currency: &str) -> VendorPart {
    let sku = s(p, &["sku"]);
    let brand = s(p, &["brandName"]);
    let mut out = VendorPart {
        provider: Some(ProviderId::Farnell),
        vendor_pn: sku.clone(),
        mpn: s(p, &["translatedManufacturerPartNumber"]),
        manufacturer: if brand.is_empty() {
            s(p, &["vendorName"])
        } else {
            brand
        },
        description: s(p, &["displayName"]),
        stock: n(p, &["stock", "level"]).map(|x| x as i64),
        lead_time: n(p, &["stock", "leastLeadTime"])
            .map(|d| format!("{d} days"))
            .unwrap_or_default(),
        lifecycle: {
            let st = s(p, &["productStatus"]);
            if st.is_empty() || st.eq_ignore_ascii_case("defaultStatus") {
                Lifecycle::Active
            } else {
                Lifecycle::from_text(&st)
            }
        },
        rohs: s(p, &["rohsStatusCode"]),
        datasheet_url: arr(p, &["datasheets"])
            .first()
            .map(|d| s(d, &["url"]))
            .unwrap_or_default(),
        product_url: format!("https://{store}/search?st={sku}"),
        fetched_at: crate::now(),
        ..Default::default()
    };
    out.prices = arr(p, &["prices"])
        .iter()
        .filter_map(|b| {
            Some(PriceBreak {
                quantity: n(b, &["from"])? as u32,
                unit_price: n(b, &["cost"])?,
                currency: currency.into(),
            })
        })
        .collect();
    for a in arr(p, &["attributes"]) {
        let unit = s(a, &["attributeUnit"]);
        let value = format!("{}{}", s(a, &["attributeValue"]), unit);
        normalize::add(&mut out, &s(a, &["attributeLabel"]), &value);
    }
    normalize::from_description(&mut out);
    out
}

#[async_trait::async_trait]
impl CatalogProvider for Farnell {
    fn id(&self) -> ProviderId {
        ProviderId::Farnell
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
        let (parts, total) = self
            .call(
                &format!("any:{}", q.text.trim()),
                q.offset,
                q.limit,
                q.in_stock,
            )
            .await?;
        Ok(SearchPage { parts, total })
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let (parts, _) = self.call(&format!("id:{}", pn.trim()), 0, 1, false).await?;
        parts.into_iter().next().ok_or(CatalogError::NotFound)
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let (parts, _) = self
            .call(&format!("manuPartNum:{}", mpn.trim()), 0, 10, false)
            .await?;
        Ok(parts
            .into_iter()
            .filter(|p| {
                mfr.is_none_or(|m| p.manufacturer.to_lowercase().contains(&m.to_lowercase()))
            })
            .collect())
    }
}
