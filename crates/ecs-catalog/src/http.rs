//! HTTP transport. Providers build plain requests; the transport sends them.
//! Tests use [`MockTransport`], which replays recorded responses and counts
//! requests (CI never calls live services).

use crate::{CatalogError, ProviderId};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    /// JSON body, or form fields.
    pub json: Option<serde_json::Value>,
    pub form: Vec<(String, String)>,
    /// HTTP basic authentication (user, password).
    pub basic: Option<(String, String)>,
}

impl Request {
    pub fn get(url: &str) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            query: Vec::new(),
            headers: Vec::new(),
            json: None,
            form: Vec::new(),
            basic: None,
        }
    }
    pub fn post(url: &str) -> Self {
        Self {
            method: Method::Post,
            ..Self::get(url)
        }
    }
    pub fn query(mut self, k: &str, v: impl Into<String>) -> Self {
        self.query.push((k.into(), v.into()));
        self
    }
    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
    pub fn json(mut self, v: serde_json::Value) -> Self {
        self.json = Some(v);
        self
    }
    pub fn form(mut self, k: &str, v: impl Into<String>) -> Self {
        self.form.push((k.into(), v.into()));
        self
    }
    pub fn basic(mut self, user: &str, password: &str) -> Self {
        self.basic = Some((user.into(), password.into()));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn json(&self) -> Result<serde_json::Value, CatalogError> {
        serde_json::from_str(&self.body)
            .map_err(|e| CatalogError::Parse(format!("{e}: {}", snippet(&self.body))))
    }
}

fn snippet(s: &str) -> String {
    s.chars().take(160).collect()
}

/// Sends requests.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, req: Request) -> Result<Response, CatalogError>;
}

/// The real network, via reqwest with rustls.
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("Electrocraft/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Self { client }
    }
}

#[async_trait::async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, req: Request) -> Result<Response, CatalogError> {
        let mut b = match req.method {
            Method::Get => self.client.get(&req.url),
            Method::Post => self.client.post(&req.url),
        };
        if !req.query.is_empty() {
            b = b.query(&req.query);
        }
        for (k, v) in &req.headers {
            b = b.header(k, v);
        }
        if let Some((u, p)) = &req.basic {
            b = b.basic_auth(u, Some(p));
        }
        if let Some(j) = &req.json {
            b = b.json(j);
        } else if !req.form.is_empty() {
            b = b.form(&req.form);
        }
        let r = b
            .send()
            .await
            .map_err(|e| CatalogError::Offline(e.to_string()))?;
        let status = r.status().as_u16();
        let headers = r
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = r
            .text()
            .await
            .map_err(|e| CatalogError::Offline(e.to_string()))?;
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

type Routes = Vec<(String, VecDeque<Response>)>;

/// Recorded responses, matched by URL substring, with a request counter.
#[derive(Clone, Default)]
pub struct MockTransport {
    routes: Arc<Mutex<Routes>>,
    count: Arc<AtomicUsize>,
    log: Arc<Mutex<Vec<Request>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }
    /// Reply to requests whose URL contains `pattern` with `body` (repeatable).
    pub fn route(&self, pattern: &str, status: u16, body: &str) -> &Self {
        if let Ok(mut r) = self.routes.lock() {
            let resp = Response {
                status,
                headers: Vec::new(),
                body: body.into(),
            };
            match r.iter_mut().find(|(p, _)| p == pattern) {
                Some((_, q)) => q.push_back(resp),
                None => r.push((pattern.into(), VecDeque::from([resp]))),
            }
        }
        self
    }
    pub fn requests(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
    pub fn log(&self) -> Vec<Request> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&self, req: Request) -> Result<Response, CatalogError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut l) = self.log.lock() {
            l.push(req.clone());
        }
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| CatalogError::Offline("mock".into()))?;
        let Some((_, q)) = routes
            .iter_mut()
            .find(|(p, _)| req.url.contains(p.as_str()))
        else {
            return Err(CatalogError::Offline(format!(
                "no recorded response for {}",
                req.url
            )));
        };
        // The last response repeats.
        if q.len() > 1 {
            Ok(q.pop_front().unwrap())
        } else {
            q.front()
                .cloned()
                .ok_or(CatalogError::Offline("empty".into()))
        }
    }
}

/// A request budget per provider (requests per minute and per day) with
/// waiting, and retry with backoff on HTTP 429.
pub struct Budget {
    per_minute: u32,
    per_day: u32,
    state: Mutex<(VecDeque<std::time::Instant>, u32, u64)>,
}

impl Budget {
    pub fn new(per_minute: u32, per_day: u32) -> Self {
        Self {
            per_minute,
            per_day,
            state: Mutex::new((VecDeque::new(), 0, 0)),
        }
    }
    /// How long to wait before the next request, or a quota error for today.
    fn reserve(&self) -> Result<Duration, CatalogError> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| CatalogError::Offline("budget".into()))?;
        let today = crate::now() / 86_400;
        if s.2 != today {
            s.1 = 0;
            s.2 = today;
        }
        if s.1 >= self.per_day {
            return Err(CatalogError::Quota(format!(
                "{} requests today (the provider's daily limit)",
                self.per_day
            )));
        }
        let now = std::time::Instant::now();
        while s
            .0
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            s.0.pop_front();
        }
        let wait = if s.0.len() as u32 >= self.per_minute {
            Duration::from_secs(60).saturating_sub(now.duration_since(*s.0.front().unwrap()))
        } else {
            Duration::ZERO
        };
        s.0.push_back(now + wait);
        s.1 += 1;
        Ok(wait)
    }

    /// Send through the budget; retries twice on 429 with backoff.
    pub async fn send(
        &self,
        t: &dyn Transport,
        p: ProviderId,
        req: Request,
    ) -> Result<Response, CatalogError> {
        let mut backoff = Duration::from_secs(2);
        for attempt in 0..3 {
            let wait = self.reserve()?;
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            let r = t.send(req.clone()).await?;
            if r.status == 429 && attempt < 2 {
                let after = r
                    .header("Retry-After")
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs);
                tokio::time::sleep(after.unwrap_or(backoff).min(Duration::from_secs(30))).await;
                backoff *= 2;
                continue;
            }
            return check(p, r);
        }
        Err(CatalogError::Quota(
            "still rate limited after retries".into(),
        ))
    }
}

/// Map HTTP failures to errors.
pub fn check(p: ProviderId, r: Response) -> Result<Response, CatalogError> {
    match r.status {
        200..=299 => Ok(r),
        401 | 403 => Err(CatalogError::BadCredentials(format!(
            "{} answered {}: {}",
            p.name(),
            r.status,
            snippet(&r.body)
        ))),
        404 => Err(CatalogError::NotFound),
        429 => Err(CatalogError::Quota(snippet(&r.body))),
        s => Err(CatalogError::Http(s, snippet(&r.body))),
    }
}
