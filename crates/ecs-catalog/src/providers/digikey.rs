//! DigiKey Product Information API v4 (<https://developer.digikey.com/products/product-information-v4>).
//!
//! * Token: `POST https://api.digikey.com/v1/oauth2/token`, form
//!   `client_id`, `client_secret`, `grant_type=client_credentials`; the
//!   response has `access_token` and `expires_in` (about 10 minutes).
//! * KeywordSearch: `POST /products/v4/search/keyword`, body `Keywords`
//!   (≤ 250 characters), `Limit` (1-50), `Offset`; headers
//!   `X-DIGIKEY-Client-Id`, `Authorization: Bearer …`,
//!   `X-DIGIKEY-Locale-Site/Language/Currency`. Response `Products[]`,
//!   `ProductsCount`.
//! * ProductDetails: `GET /products/v4/search/{productNumber}/productdetails`,
//!   response `{"Product": {…}}`.
//! * Limits: 120 requests per minute and 1000 per day per application
//!   (429 with `Retry-After` beyond); the API User Agreement also mentions a
//!   ten-minute suspension for exceeding a per-second threshold.
//! * Terms (<https://developer.digikey.com/api-user-agreement>): attribute
//!   DigiKey data; no bulk download; no use "to update or create your own
//!   database of information" (5.1(e)); no cache lifetime is stated, so the
//!   cache keeps results for 24 h at most.
//!
//! Region setting: `SITE,CURRENCY` (default `US,USD`).

use super::{Env, arr, n, s};
use crate::http::{Budget, Request};
use crate::{
    Capabilities, CatalogError, CatalogProvider, CatalogQuery, Lifecycle, PriceBreak, ProviderId,
    SearchPage, VendorPart, normalize,
};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://api.digikey.com";

pub struct Digikey {
    env: Env,
    budget: Budget,
    token: Mutex<Option<(String, Instant)>>,
}

impl Digikey {
    pub fn new(env: Env) -> Self {
        Self {
            env,
            budget: Budget::new(120, 1000),
            token: Mutex::new(None),
        }
    }

    fn locale(&self) -> (String, String) {
        let r = self.env.region(ProviderId::Digikey, "US,USD");
        let mut it = r.split(',').map(|x| x.trim().to_string());
        (
            it.next().unwrap_or("US".into()),
            it.next().unwrap_or("USD".into()),
        )
    }

    async fn token(&self) -> Result<String, CatalogError> {
        if let Ok(t) = self.token.lock()
            && let Some((tok, until)) = t.as_ref()
            && Instant::now() < *until
        {
            return Ok(tok.clone());
        }
        let (id, secret) = (
            self.env.cred(ProviderId::Digikey, "client_id")?,
            self.env.cred(ProviderId::Digikey, "client_secret")?,
        );
        let req = Request::post(&format!("{BASE}/v1/oauth2/token"))
            .form("client_id", id)
            .form("client_secret", secret)
            .form("grant_type", "client_credentials");
        let v = self
            .budget
            .send(self.env.transport.as_ref(), ProviderId::Digikey, req)
            .await?
            .json()?;
        let tok = s(&v, &["access_token"]);
        if tok.is_empty() {
            return Err(CatalogError::BadCredentials(
                "no access_token in the token response".into(),
            ));
        }
        let life = n(&v, &["expires_in"]).unwrap_or(600.0).max(60.0) - 30.0;
        if let Ok(mut t) = self.token.lock() {
            *t = Some((tok.clone(), Instant::now() + Duration::from_secs_f64(life)));
        }
        Ok(tok)
    }

    async fn call(&self, req: Request) -> Result<Value, CatalogError> {
        let tok = self.token().await?;
        let (site, currency) = self.locale();
        let req = req
            .header(
                "X-DIGIKEY-Client-Id",
                self.env.cred(ProviderId::Digikey, "client_id")?,
            )
            .header("Authorization", format!("Bearer {tok}"))
            .header("X-DIGIKEY-Locale-Site", site)
            .header("X-DIGIKEY-Locale-Language", "en")
            .header("X-DIGIKEY-Locale-Currency", currency)
            .header("X-DIGIKEY-Customer-Id", "0");
        self.budget
            .send(self.env.transport.as_ref(), ProviderId::Digikey, req)
            .await?
            .json()
    }

    fn currency(&self) -> String {
        self.locale().1
    }
}

/// A DigiKey `Product`.
pub fn part(p: &Value, currency: &str) -> VendorPart {
    let variation = arr(p, &["ProductVariations"])
        .first()
        .cloned()
        .unwrap_or(Value::Null);
    let mut out = VendorPart {
        provider: Some(ProviderId::Digikey),
        vendor_pn: s(&variation, &["DigiKeyProductNumber"]),
        mpn: s(p, &["ManufacturerProductNumber"]),
        manufacturer: s(p, &["Manufacturer", "Name"]),
        description: {
            let d = s(p, &["Description", "DetailedDescription"]);
            if d.is_empty() {
                s(p, &["Description", "ProductDescription"])
            } else {
                d
            }
        },
        category: s(p, &["Category", "Name"]),
        stock: n(p, &["QuantityAvailable"]).map(|x| x as i64),
        lead_time: {
            let w = s(p, &["ManufacturerLeadWeeks"]);
            if w.is_empty() {
                w
            } else {
                format!("{w} weeks")
            }
        },
        lifecycle: Lifecycle::from_text(&s(p, &["ProductStatus", "Status"])),
        rohs: s(p, &["Classifications", "RohsStatus"]),
        datasheet_url: s(p, &["DatasheetUrl"]),
        product_url: s(p, &["ProductUrl"]),
        fetched_at: crate::now(),
        ..Default::default()
    };
    out.prices = arr(&variation, &["StandardPricing"])
        .iter()
        .filter_map(|b| {
            Some(PriceBreak {
                quantity: n(b, &["BreakQuantity"])? as u32,
                unit_price: n(b, &["UnitPrice"])?,
                currency: currency.into(),
            })
        })
        .collect();
    if out.prices.is_empty()
        && let Some(u) = n(p, &["UnitPrice"])
    {
        out.prices.push(PriceBreak {
            quantity: 1,
            unit_price: u,
            currency: currency.into(),
        });
    }
    for prm in arr(p, &["Parameters"]) {
        normalize::add(
            &mut out,
            &s(prm, &["ParameterText"]),
            &s(prm, &["ValueText"]),
        );
    }
    out
}

#[async_trait::async_trait]
impl CatalogProvider for Digikey {
    fn id(&self) -> ProviderId {
        ProviderId::Digikey
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
        let text: String = q.text.chars().take(250).collect();
        let body = json!({"Keywords": text, "Limit": q.limit.clamp(1, 50), "Offset": q.offset});
        let v = self
            .call(Request::post(&format!("{BASE}/products/v4/search/keyword")).json(body))
            .await?;
        let cur = self.currency();
        Ok(SearchPage {
            total: n(&v, &["ProductsCount"]).map(|x| x as u64),
            parts: arr(&v, &["Products"])
                .iter()
                .map(|p| part(p, &cur))
                .collect(),
        })
    }
    async fn by_vendor_pn(&self, pn: &str) -> Result<VendorPart, CatalogError> {
        let v = self
            .call(Request::get(&format!(
                "{BASE}/products/v4/search/{}/productdetails",
                pn.trim()
            )))
            .await?;
        if v["Product"].is_null() {
            return Err(CatalogError::NotFound);
        }
        Ok(part(&v["Product"], &self.currency()))
    }
    async fn by_mpn(&self, mpn: &str, mfr: Option<&str>) -> Result<Vec<VendorPart>, CatalogError> {
        let page = self.search(&CatalogQuery::text(mpn)).await?;
        Ok(page
            .parts
            .into_iter()
            .filter(|p| {
                p.mpn.eq_ignore_ascii_case(mpn.trim())
                    && mfr.is_none_or(|m| p.manufacturer.to_lowercase().contains(&m.to_lowercase()))
            })
            .collect())
    }
}
