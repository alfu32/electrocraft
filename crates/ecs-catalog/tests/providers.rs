//! Providers against recorded responses (no live calls), the consent gate,
//! normalization per provider and mapping to components.

use ecs_catalog::cache::Cache;
use ecs_catalog::credentials::{Memory, Store};
use ecs_catalog::http::{Method, MockTransport};
use ecs_catalog::providers::{self, Env};
use ecs_catalog::service::{Outcome, Service};
use ecs_catalog::{
    CatalogError, CatalogProvider, CatalogQuery, Consent, ConsentGate, Lifecycle, ProviderId,
    mapping,
};
use std::sync::{Arc, RwLock};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn env(mock: &MockTransport, allowed: &[ProviderId]) -> Env {
    let creds = Memory::default();
    for p in ProviderId::ALL {
        for (f, _) in p.credential_fields() {
            creds.set(p, f, "secret-value").unwrap();
        }
    }
    let consent = Consent {
        master: true,
        enabled: allowed.to_vec(),
        ..Default::default()
    };
    Env {
        transport: Arc::new(mock.clone()),
        creds: Arc::new(creds),
        consent: Arc::new(RwLock::new(consent)),
        cache: Arc::new(Cache::memory()),
    }
}

fn gate(env: &Env, p: ProviderId) -> Arc<ConsentGate> {
    providers::all(env)
        .into_iter()
        .find(|g| g.id() == p)
        .unwrap()
}

fn block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn consent_off_sends_nothing() {
    let mock = MockTransport::new();
    mock.route("", 200, "{}");
    // Master off, every provider listed.
    let e = env(&mock, &ProviderId::ALL);
    e.consent.write().unwrap().master = false;
    for p in ProviderId::ALL {
        let g = gate(&e, p);
        assert_eq!(
            block(g.search(&CatalogQuery::text("10k"))),
            Err(CatalogError::NotApproved(p))
        );
        assert!(block(g.by_mpn("NE555P", None)).is_err());
        assert!(block(g.by_vendor_pn("X")).is_err());
    }
    // Master on, no provider enabled.
    e.consent.write().unwrap().master = true;
    e.consent.write().unwrap().enabled.clear();
    for p in ProviderId::ALL {
        assert_eq!(
            block(gate(&e, p).search(&CatalogQuery::text("10k"))),
            Err(CatalogError::NotApproved(p))
        );
    }
    assert_eq!(mock.requests(), 0);
}

#[test]
fn service_honours_consent_and_reports_per_provider() {
    let mock = MockTransport::new();
    mock.route("jlcsearch", 200, &fixture("jlcsearch_10k_0603.json"));
    let e = env(&mock, &[ProviderId::Jlcpcb]);
    let mut svc = Service::start(e, Arc::new(|| {}));
    let job = svc.search(&ProviderId::ALL, CatalogQuery::text("10k 0603"));
    let mut replies = Vec::new();
    let t0 = std::time::Instant::now();
    while replies.len() < 5 && t0.elapsed() < std::time::Duration::from_secs(10) {
        replies.extend(svc.poll());
        std::thread::yield_now();
    }
    assert_eq!(replies.len(), 5);
    for r in &replies {
        assert_eq!(r.job, job);
        let Outcome::Page(page) = &r.outcome else {
            panic!()
        };
        match r.provider {
            ProviderId::Jlcpcb => assert_eq!(page.as_ref().unwrap().parts.len(), 3),
            p => assert_eq!(page.as_ref().unwrap_err(), &CatalogError::NotApproved(p)),
        }
    }
    assert_eq!(mock.requests(), 1);
}

#[test]
fn jlcsearch_normalizes_and_maps() {
    let mock = MockTransport::new();
    mock.route(
        "jlcsearch.tscircuit.com/api/search",
        200,
        &fixture("jlcsearch_10k_0603.json"),
    );
    let e = env(&mock, &[ProviderId::Jlcpcb]);
    let g = gate(&e, ProviderId::Jlcpcb);
    let page = block(g.search(&CatalogQuery::text("10k 0603"))).unwrap();
    let r = &page.parts[0];
    assert_eq!(
        (r.vendor_pn.as_str(), r.mpn.as_str(), r.package.as_str()),
        ("C98220", "RC0603FR-0710KL", "0603")
    );
    assert_eq!(r.number("resistance"), Some(10e3));
    assert_eq!(r.number("tolerance"), Some(0.01));
    assert_eq!(r.number("power_rating"), Some(0.1));
    assert_eq!(r.stock, Some(11792132));
    assert_eq!(r.price_at(1).unwrap().unit_price, 0.0023);
    assert_eq!(r.jlc_basic, Some(false));
    assert_eq!(page.parts[2].jlc_basic, Some(true));
    let place = mapping::placement(r).unwrap();
    assert_eq!(
        (place.def, place.params[0].1.as_str()),
        ("core.resistor", "10k")
    );
    // The request carried only the search text.
    let req = &mock.log()[0];
    assert_eq!(
        req.query,
        vec![
            ("q".to_string(), "10k 0603".to_string()),
            ("limit".to_string(), "20".to_string())
        ]
    );
    // A repeat is served from the cache.
    block(g.search(&CatalogQuery::text("10k 0603"))).unwrap();
    assert_eq!(mock.requests(), 1);
}

#[test]
fn digikey_token_then_search() {
    let mock = MockTransport::new();
    mock.route("/v1/oauth2/token", 200, &fixture("digikey_token.json"));
    mock.route(
        "/products/v4/search/keyword",
        200,
        &fixture("digikey_keyword.json"),
    );
    let e = env(&mock, &[ProviderId::Digikey]);
    let page = block(gate(&e, ProviderId::Digikey).search(&CatalogQuery::text("RC0603FR-0710KL")))
        .unwrap();
    let p = &page.parts[0];
    assert_eq!(
        (p.vendor_pn.as_str(), p.manufacturer.as_str()),
        ("311-10.0KHRCT-ND", "YAGEO")
    );
    assert_eq!(p.number("resistance"), Some(10e3));
    assert_eq!(p.number("power_rating"), Some(0.1));
    assert_eq!(p.package, "0603 (1608 Metric)");
    assert_eq!(p.lifecycle, Lifecycle::Active);
    assert_eq!(p.price_at(50).unwrap().unit_price, 0.016);
    assert_eq!(p.price_at(50).unwrap().currency, "USD");
    let log = mock.log();
    assert_eq!(log.len(), 2);
    assert!(
        log[0]
            .form
            .contains(&("grant_type".into(), "client_credentials".into()))
    );
    assert!(
        log[1]
            .headers
            .contains(&("Authorization".into(), "Bearer test-token".into()))
    );
    assert_eq!(log[1].json.as_ref().unwrap()["Limit"], 20);
}

#[test]
fn mouser_search_and_key_errors() {
    let mock = MockTransport::new();
    mock.route("/search/keyword", 200, &fixture("mouser_keyword.json"));
    let e = env(&mock, &[ProviderId::Mouser]);
    let page =
        block(gate(&e, ProviderId::Mouser).search(&CatalogQuery::text("100uF 25V"))).unwrap();
    let p = &page.parts[0];
    assert_eq!(
        (p.vendor_pn.as_str(), p.mpn.as_str()),
        ("647-UVR1E101MED", "UVR1E101MED")
    );
    assert_eq!(p.stock, Some(1240));
    assert_eq!(p.price_at(10).unwrap().unit_price, 0.137);
    assert_eq!(p.number("capacitance"), Some(100e-6));
    assert_eq!(p.number("voltage_rating"), Some(25.0));
    assert_eq!(mapping::placement(p).unwrap().def, "core.capacitor");
    assert_eq!(mock.log()[0].method, Method::Post);
    assert_eq!(
        mock.log()[0].json.as_ref().unwrap()["SearchByKeywordRequest"]["records"],
        20
    );

    let bad = MockTransport::new();
    bad.route("/search/keyword", 200, &fixture("mouser_bad_key.json"));
    let e = env(&bad, &[ProviderId::Mouser]);
    assert!(matches!(
        block(gate(&e, ProviderId::Mouser).search(&CatalogQuery::text("x1"))),
        Err(CatalogError::BadCredentials(_))
    ));
}

#[test]
fn tme_search_fills_prices_stock_and_parameters() {
    let mock = MockTransport::new();
    mock.route("/auth/token", 200, &fixture("tme_token.json"));
    mock.route("/products/search", 200, &fixture("tme_search.json"));
    mock.route("/products/data", 200, &fixture("tme_data.json"));
    mock.route("/products/parameters", 200, &fixture("tme_parameters.json"));
    let e = env(&mock, &[ProviderId::Tme]);
    let page = block(gate(&e, ProviderId::Tme).search(&CatalogQuery::text("1N4733A"))).unwrap();
    let p = &page.parts[0];
    assert_eq!(
        (p.vendor_pn.as_str(), p.mpn.as_str()),
        ("1N4733A-DIO", "1N4733A")
    );
    assert_eq!(p.stock, Some(21450));
    assert_eq!(p.price_at(100).unwrap().unit_price, 0.0391);
    assert_eq!(p.price_at(100).unwrap().currency, "EUR");
    assert_eq!(p.number("zener_voltage"), Some(5.1));
    assert_eq!(p.package, "DO41");
    let place = mapping::placement(p).unwrap();
    assert_eq!(
        (place.def, place.params[0].clone()),
        ("core.zener", ("BV".to_string(), "5.1".to_string()))
    );
    let log = mock.log();
    assert!(log[0].basic.is_some());
    assert!(log[1].query.contains(&("phrase".into(), "1N4733A".into())));
}

#[test]
fn farnell_search_is_never_cached() {
    let mock = MockTransport::new();
    mock.route("api.element14.com", 200, &fixture("farnell_keyword.json"));
    let e = env(&mock, &[ProviderId::Farnell]);
    e.consent
        .write()
        .unwrap()
        .region
        .insert("farnell".into(), "de.farnell.com".into());
    let g = gate(&e, ProviderId::Farnell);
    let page = block(g.search(&CatalogQuery::text("BC547B"))).unwrap();
    let p = &page.parts[0];
    assert_eq!(
        (
            p.vendor_pn.as_str(),
            p.mpn.as_str(),
            p.manufacturer.as_str()
        ),
        ("1652438", "BC547B,112", "NEXPERIA")
    );
    assert_eq!(p.stock, Some(3561));
    assert_eq!(p.price_at(10).unwrap().currency, "EUR");
    assert_eq!(p.package, "TO-92");
    let place = mapping::placement(p).unwrap();
    assert_eq!((place.def, place.generic), ("core.npn", true));
    assert!(
        mock.log()[0]
            .query
            .contains(&("storeInfo.id".into(), "de.farnell.com".into()))
    );
    assert!(
        mock.log()[0]
            .query
            .contains(&("term".into(), "any:BC547B".into()))
    );
    block(g.search(&CatalogQuery::text("BC547B"))).unwrap();
    assert_eq!(mock.requests(), 2);
    assert_eq!(e.cache.entries(ProviderId::Farnell), 0);
}

#[test]
fn missing_credentials_send_nothing() {
    let mock = MockTransport::new();
    mock.route("", 200, "{}");
    let e = env(&mock, &ProviderId::ALL);
    let empty = Env {
        creds: Arc::new(Memory::default()),
        ..e
    };
    for p in [
        ProviderId::Digikey,
        ProviderId::Mouser,
        ProviderId::Tme,
        ProviderId::Farnell,
    ] {
        assert_eq!(
            block(gate(&empty, p).search(&CatalogQuery::text("x1"))),
            Err(CatalogError::MissingCredentials(p))
        );
    }
    assert_eq!(mock.requests(), 0);
}

#[test]
fn rate_limited_requests_retry() {
    let mock = MockTransport::new();
    mock.route("jlcsearch", 429, "slow down");
    mock.route("jlcsearch", 200, &fixture("jlcsearch_10k_0603.json"));
    let e = env(&mock, &[ProviderId::Jlcpcb]);
    let page = block(gate(&e, ProviderId::Jlcpcb).search(&CatalogQuery::text("10k"))).unwrap();
    assert_eq!(page.parts.len(), 3);
    assert_eq!(mock.requests(), 2);
}
