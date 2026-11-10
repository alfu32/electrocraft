//! Mouser Search API v1 (Swagger at <https://api.mouser.com/api/docs/v1>).
//!
//! * Keyword: `POST https://api.mouser.com/api/v1/search/keyword?apiKey=…`,
//!   body `{"SearchByKeywordRequest": {"keyword", "records", "startingRecord",
//!   "searchOptions" (None|Rohs|InStock|RohsAndInStock), "searchWithYourSignUpLanguage"}}`;
//!   at most 50 parts per request.
//! * Part number: `POST /api/v1/search/partnumber`, body
//!   `{"SearchByPartRequest": {"mouserPartNumber", "partSearchOptions" (None|Exact)}}`;
//!   matches Mouser and manufacturer numbers, up to 10 separated by `|`.
//! * Response: `Errors[]` (`Code`, `Message`) and `SearchResults` with
//!   `NumberOfResult` and `Parts[]` (`MouserPartNumber`,
//!   `ManufacturerPartNumber`, `Manufacturer`, `Description`, `Category`,
//!   `DataSheetUrl`, `ProductDetailUrl`, `AvailabilityInStock` (text),
//!   `LifecycleStatus`, `LeadTime`, `ROHSStatus`, `PriceBreaks[]` with
//!   `Quantity`, `Price` (text), `RawPrice`, `Currency`, `ProductAttributes[]`).
//! * Rate limits and the terms of use (<https://www.mouser.com/api-hub/>)
//!   could not be read (the pages timed out on Nov 10, 2026): the client
//!   keeps to 30 requests per minute and 1000 per day, and caches 24 h.

use super::{Env, arr, n, parse_number, s};
use crate::http::{Budget, Request};
use crate::{
    Capabilities, CatalogError, CatalogProvider, CatalogQuery, Lifecycle, PriceBreak, ProviderId,
    SearchPage, VendorPart, normalize,
};
use serde_json::{Value, json};

const BASE: &str = "https://api.mouser.com/api/v1/search";

pub struct Mouser {
    env: Env,
    budget: Budget,
}

impl Mouser {
    pub fn new(env: Env) -> Self {
        Self {
            env,
            budget: Budget::new(30, 1000),
        }
    }
    async fn call(&self, path: &str, body: Value) -> Result<Vec<VendorPart>, CatalogError> {
        let key = self.env.cred(ProviderId::Mouser, "api_key")?;
        let req = Request::post(&format!("{BASE}/{path}"))
            .query("apiKey", key)
            .json(body);
        let v = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Mouser, req)
            .await?
            .json()?;
        if let Some(e) = arr(&v, &["Errors"]).first() {
            let (code, msg, prop) = (
                s(e, &["Code"]),
                s(e, &["Message"]),
                s(e, &["PropertyName"]).to_lowercase(),
            );
            let key_problem = prop.contains("api key")
                || prop.contains("apikey")
                || code.to_lowercase().contains("auth")
                || msg.to_lowercase().contains("api key");
            return Err(if key_problem {
                CatalogError::BadCredentials(format!("{code} {msg}"))
            } else {
                CatalogError::Http(200, format!("{code} {msg}"))
            });
        }
        Ok(arr(&v, &["SearchResults", "Parts"])
            .iter()
            .map(part)
            .collect())
    }
}

/// A Mouser `MouserPart`.
pub fn part(p: &Value) -> VendorPart {
    let mut out = VendorPart {
        provider: Some(ProviderId::Mouser),
        vendor_pn: s(p, &["MouserPartNumber"]),
        mpn: s(p, &["ManufacturerPartNumber"]),
        manufacturer: s(p, &["Manufacturer"]),
        description: s(p, &["Description"]),
        category: s(p, &["Category"]),
        stock: n(p, &["AvailabilityInStock"])
            .map(|x| x as i64)
            .or_else(|| parse_number(&s(p, &["Availability"])).map(|x| x as i64))
            .or(Some(0)),
        lead_time: s(p, &["LeadTime"]),
        lifecycle: {
            let l = s(p, &["LifecycleStatus"]);
            if l.is_empty() && p["IsDiscontinued"].as_str() != Some("true") {
                Lifecycle::Active
            } else if p["IsDiscontinued"].as_str() == Some("true") {
                Lifecycle::Obsolete
            } else {
                Lifecycle::from_text(&l)
            }
        },
        rohs: s(p, &["ROHSStatus"]),
        datasheet_url: s(p, &["DataSheetUrl"]),
        product_url: s(p, &["ProductDetailUrl"]),
        fetched_at: crate::now(),
        ..Default::default()
    };
    out.prices = arr(p, &["PriceBreaks"])
        .iter()
        .filter_map(|b| {
            let price = n(b, &["RawPrice"]).or_else(|| parse_number(&s(b, &["Price"])))?;
            Some(PriceBreak {
                quantity: n(b, &["Quantity"])? as u32,
                unit_price: price,
                currency: s(b, &["Currency"]),
            })
        })
        .collect();
    for a in arr(p, &["ProductAttributes"]) {
        normalize::add(
            &mut out,
            &s(a, &["AttributeName"]),
            &s(a, &["AttributeValue"]),
        );
    }
    normalize::from_description(&mut out);
    out
}

#[async_trait::async_trait]
impl CatalogProvider for Mouser {
    fn id(&self) -> ProviderId {
        ProviderId::Mouser
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
        let body = json!({"SearchByKeywordRequest": {
            "keyword": q.text,
            "records": q.limit.clamp(1, 50),
            "startingRecord": q.offset,
            "searchOptions": if q.in_stock { "InStock" } else { "None" },
            "searchWithYourSignUpLanguage": "false",
        }});
        Ok(SearchPage {
            total: None,
            parts: self.call("keyword", body).await?,
        })
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let body = json!({"SearchByPartRequest": {"mouserPartNumber": pn.trim(), "partSearchOptions": "Exact"}});
        self.call("partnumber", body)
            .await?
            .into_iter()
            .find(|p| p.vendor_pn.eq_ignore_ascii_case(pn.trim()))
            .ok_or(CatalogError::NotFound)
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let body = json!({"SearchByPartRequest": {"mouserPartNumber": mpn.trim(), "partSearchOptions": "Exact"}});
        Ok(self
            .call("partnumber", body)
            .await?
            .into_iter()
            .filter(|p| {
                p.mpn.eq_ignore_ascii_case(mpn.trim())
                    && mfr.is_none_or(|m| p.manufacturer.to_lowercase().contains(&m.to_lowercase()))
            })
            .collect())
    }
}
