//! Streamable HTTP transport for the MCP tools of the current window.

use crate::app::{App, Cmd};
use crate::mcp::{AppBackend, Job};
use crate::settings::McpSettings;
use crate::theme::{self, c};
use egui::{RichText, Ui};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde_json::Value;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Write;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

const MAX_CONNECTIONS: usize = 16;
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_LOG_ENTRIES: usize = 200;

type HttpResponse = Response<Full<Bytes>>;

#[derive(Default)]
pub struct NetworkMcp {
    listener: Option<Listener>,
    error: Option<String>,
    log: Arc<AccessLog>,
}

struct Listener {
    endpoint: String,
    token: String,
    jobs: Option<Receiver<Job>>,
    stop: Option<oneshot::Sender<()>>,
    thread: JoinHandle<Result<(), String>>,
}

#[derive(Default)]
struct AccessLog {
    enabled: AtomicBool,
    entries: Mutex<VecDeque<String>>,
}

impl AccessLog {
    fn record(&self, entry: impl FnOnce() -> String) {
        if self.enabled.load(Ordering::Relaxed)
            && let Ok(mut entries) = self.entries.lock()
        {
            if entries.len() >= MAX_LOG_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(entry());
        }
    }
}

impl NetworkMcp {
    pub fn start(&mut self, config: &McpSettings, ctx: &egui::Context) {
        if self.listener.is_some() {
            return;
        }
        self.error = None;
        match Listener::start(config, ctx, self.log.clone()) {
            Ok(listener) => self.listener = Some(listener),
            Err(error) => self.error = Some(error),
        }
    }

    pub fn stop(&mut self) {
        if let Some(listener) = &mut self.listener {
            // Drop queued calls and their reply senders before stopping the runtime.
            listener.jobs = None;
            if let Some(stop) = listener.stop.take() {
                let _ = stop.send(());
            }
        }
    }

    pub fn poll(&mut self) {
        if self
            .listener
            .as_ref()
            .is_some_and(|s| s.thread.is_finished())
            && let Some(listener) = self.listener.take()
        {
            self.error = match listener.thread.join() {
                Ok(result) => result.err(),
                Err(_) => Some("MCP listener stopped unexpectedly".into()),
            };
        }
    }

    pub fn try_job(&self) -> Option<Job> {
        self.listener.as_ref()?.jobs.as_ref()?.try_recv().ok()
    }
}

impl Listener {
    fn start(
        config: &McpSettings,
        ctx: &egui::Context,
        log: Arc<AccessLog>,
    ) -> Result<Self, String> {
        let ip: IpAddr = config.bind.trim().parse().map_err(|_| {
            "Enter an IPv4 or IPv6 bind address, such as 127.0.0.1 or ::1".to_owned()
        })?;
        if config.port == 0 {
            return Err("Enter a port between 1 and 65535".into());
        }
        let address = SocketAddr::new(ip, config.port);
        let tls = if config.https {
            Some(tls_acceptor(config)?)
        } else {
            None
        };
        let socket =
            TcpListener::bind(address).map_err(|e| format!("Cannot listen on {address}: {e}"))?;
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(MAX_CONNECTIONS)
            .build()
            .map_err(|e| format!("Cannot create MCP runtime: {e}"))?;
        let listener = {
            let _guard = runtime.enter();
            tokio::net::TcpListener::from_std(socket)
                .map_err(|e| format!("Cannot configure MCP listener: {e}"))?
        };

        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|e| format!("Cannot generate access token: {e}"))?;
        let mut token = String::with_capacity(64);
        for byte in random {
            let _ = write!(token, "{byte:02x}");
        }
        let scheme = if config.https { "https" } else { "http" };
        let endpoint = format!("{scheme}://{address}/mcp");
        let (tx, jobs) = sync_channel(MAX_CONNECTIONS);
        let (stop, shutdown) = oneshot::channel();
        log.enabled.store(config.access_log, Ordering::Relaxed);
        let shared = Arc::new(HttpState {
            backend: AppBackend::new(tx, ctx.clone()),
            authorization: format!("Bearer {token}"),
            log,
            started: Instant::now(),
            ctx: ctx.clone(),
        });
        let ctx = ctx.clone();
        let thread = std::thread::Builder::new()
            .name("mcp-http".into())
            .spawn(move || {
                let result = runtime.block_on(serve(listener, tls, shared, shutdown));
                // The UI drops its queue when stopping. If the listener fails, allow
                // the UI to collect the error without waiting on a tool request.
                runtime.shutdown_background();
                ctx.request_repaint();
                result
            })
            .map_err(|e| format!("Cannot start MCP listener: {e}"))?;
        Ok(Self {
            endpoint,
            token,
            jobs: Some(jobs),
            stop: Some(stop),
            thread,
        })
    }
}

fn tls_acceptor(config: &McpSettings) -> Result<TlsAcceptor, String> {
    let certificates = CertificateDer::pem_file_iter(config.certificate.trim())
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| format!("Cannot read PEM certificate: {e}"))?;
    let key = PrivateKeyDer::from_pem_file(config.private_key.trim())
        .map_err(|e| format!("Cannot read PEM private key: {e}"))?;
    let tls = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .and_then(|builder| {
        builder
            .with_no_client_auth()
            .with_single_cert(certificates, key)
    })
    .map_err(|e| format!("Invalid TLS certificate or key: {e}"))?;
    Ok(TlsAcceptor::from(Arc::new(tls)))
}

struct HttpState {
    backend: AppBackend,
    authorization: String,
    log: Arc<AccessLog>,
    started: Instant,
    ctx: egui::Context,
}

async fn serve(
    listener: tokio::net::TcpListener,
    tls: Option<TlsAcceptor>,
    state: Arc<HttpState>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return Ok(()),
            Some(_) = connections.join_next() => {},
            result = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                let (stream, peer) = result.map_err(|e| format!("MCP listener failed: {e}"))?;
                let tls = tls.clone();
                let state = state.clone();
                connections.spawn(async move {
                    if let Some(tls) = tls {
                        match timeout(Duration::from_secs(10), tls.accept(stream)).await {
                            Ok(Ok(stream)) => serve_connection(stream, peer, state).await,
                            _ => {
                                state.log.record(|| format!("{peer}  TLS handshake failed"));
                                state.ctx.request_repaint();
                            }
                        }
                    } else {
                        serve_connection(stream, peer, state).await;
                    }
                });
            }
        }
    }
}

async fn serve_connection(
    stream: impl AsyncRead + AsyncWrite + Unpin,
    peer: SocketAddr,
    state: Arc<HttpState>,
) {
    let service = service_fn(move |request: Request<Incoming>| {
        let state = state.clone();
        async move {
            let started = Instant::now();
            let method = request.method().clone();
            let response = respond(request, &state).await;
            state.log.record(|| {
                format!(
                    "+{:04}s  {}  {:.12}  {}  {} ms",
                    state.started.elapsed().as_secs(),
                    peer,
                    method.as_str(),
                    response.status().as_u16(),
                    started.elapsed().as_millis()
                )
            });
            state.ctx.request_repaint();
            Ok::<_, Infallible>(response)
        }
    });
    let connection = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10))
        .keep_alive(false)
        .serve_connection(TokioIo::new(stream), service);
    let _ = timeout(Duration::from_secs(150), connection).await;
}

async fn respond(request: Request<Incoming>, state: &HttpState) -> HttpResponse {
    // Browser origins are not allowed. Native clients send no Origin header.
    if request.headers().contains_key(header::ORIGIN) {
        return text_response(StatusCode::FORBIDDEN, "Browser origins are not allowed");
    }
    if request.uri().path() != "/mcp" {
        return text_response(StatusCode::NOT_FOUND, "MCP endpoint is /mcp");
    }
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .is_some_and(|value| bool::from(value.as_bytes().ct_eq(state.authorization.as_bytes())));
    if !authorized {
        let mut response = text_response(StatusCode::UNAUTHORIZED, "MCP bearer token required");
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            header::HeaderValue::from_static("Bearer"),
        );
        return response;
    }
    if request.method() != Method::POST {
        let mut response = text_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "MCP uses JSON responses to POST",
        );
        response
            .headers_mut()
            .insert(header::ALLOW, header::HeaderValue::from_static("POST"));
        return response;
    }
    if let Some(version) = request.headers().get("mcp-protocol-version")
        && !version
            .to_str()
            .is_ok_and(|v| ecs_mcp::PROTOCOL_VERSIONS.contains(&v))
    {
        return text_response(StatusCode::BAD_REQUEST, "Unsupported MCP protocol version");
    }
    if !request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return text_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send application/json");
    }

    let body = match timeout(
        Duration::from_secs(15),
        Limited::new(request.into_body(), MAX_BODY).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(error)) if error.is::<http_body_util::LengthLimitError>() => {
            return text_response(StatusCode::PAYLOAD_TOO_LARGE, "MCP request is too large");
        }
        Ok(Err(_)) => return text_response(StatusCode::BAD_REQUEST, "Cannot read MCP request"),
        Err(_) => return text_response(StatusCode::REQUEST_TIMEOUT, "MCP request body timed out"),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return text_response(StatusCode::BAD_REQUEST, "Invalid JSON");
    };
    if !message.is_object() || message["jsonrpc"] != "2.0" {
        return text_response(StatusCode::BAD_REQUEST, "Send one JSON-RPC 2.0 message");
    }
    let backend = state.backend.clone();
    match tokio::task::spawn_blocking(move || {
        ecs_mcp::Server::new(backend).handle_message(&message)
    })
    .await
    {
        Ok(Some(reply)) => {
            let mut response = Response::new(Full::new(Bytes::from(reply.to_string())));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            );
            response
        }
        Ok(None) => text_response(StatusCode::ACCEPTED, ""),
        Err(_) => text_response(StatusCode::INTERNAL_SERVER_ERROR, "MCP request failed"),
    }
}

fn text_response(status: StatusCode, message: &'static str) -> HttpResponse {
    let mut response = Response::new(Full::new(Bytes::from_static(message.as_bytes())));
    *response.status_mut() = status;
    if !message.is_empty() {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/plain; charset=utf-8"),
        );
    }
    response
}

fn pem_field(ui: &mut Ui, label: &str, path: &mut String) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(path).desired_width(280.0));
        if ui.button("Browse…").clicked()
            && let Some(file) = rfd::FileDialog::new().pick_file()
        {
            *path = file.to_string_lossy().into_owned();
        }
    });
    ui.end_row();
}

impl App {
    pub fn mcp_settings(&mut self, ui: &mut Ui) {
        let server = &mut self.mcp_network;
        let config = &mut self.settings.mcp;
        let before = config.clone();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Network MCP server").font(theme::bold(13.0)));
            match &server.listener {
                Some(listener) if listener.stop.is_some() => {
                    ui.label(RichText::new("Running").color(c::OK));
                }
                Some(_) => {
                    ui.label("Stopping…");
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
                None => {
                    ui.label(RichText::new("Stopped").color(c::TEXT_DIM));
                }
            }
        });
        ui.add_enabled_ui(server.listener.is_none(), |ui| {
            egui::Grid::new("mcp-network-settings")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Bind address");
                    ui.add(egui::TextEdit::singleline(&mut config.bind).desired_width(160.0))
                        .on_hover_text("IPv4 or IPv6 address. Use 127.0.0.1 for local clients.");
                    ui.end_row();
                    ui.label("Port");
                    ui.add(egui::DragValue::new(&mut config.port).range(1..=65535));
                    ui.end_row();
                    ui.label("Transport");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut config.https, false, "HTTP");
                        ui.selectable_value(&mut config.https, true, "HTTPS");
                    });
                    ui.end_row();
                    if config.https {
                        pem_field(ui, "PEM certificate", &mut config.certificate);
                        pem_field(ui, "PEM private key", &mut config.private_key);
                    }
                });
        });
        if !config.https
            && config
                .bind
                .trim()
                .parse::<IpAddr>()
                .is_ok_and(|ip| !ip.is_loopback())
        {
            ui.label(
                RichText::new("HTTP sends the access token and requests in clear text.")
                    .color(c::WARN),
            );
        }
        let mut command = None;
        ui.horizontal(|ui| match &server.listener {
            Some(listener) if listener.stop.is_some() => {
                if ui.button("Stop server").clicked() {
                    command = Some(Cmd::McpStop);
                }
            }
            Some(_) => {}
            None => {
                if ui.button("Start server").clicked() {
                    command = Some(Cmd::McpStart);
                }
            }
        });
        if let Some(error) = &server.error {
            ui.label(RichText::new(error).color(c::WARN));
        }
        if let Some(listener) = &server.listener
            && listener.stop.is_some()
        {
            ui.horizontal(|ui| {
                ui.monospace(&listener.endpoint);
                if ui.button("Copy URL").clicked() {
                    ui.ctx().copy_text(listener.endpoint.clone());
                }
            });
            ui.horizontal(|ui| {
                ui.label("Client authorization: Bearer token");
                if ui.button("Copy token").clicked() {
                    ui.ctx().copy_text(listener.token.clone());
                }
            });
            if config
                .bind
                .trim()
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_unspecified())
            {
                ui.label(
                    "For a wildcard bind, connect using this machine’s IP address or hostname.",
                );
            }
            if config.https {
                ui.label("Use an IP address or hostname covered by the certificate.");
            }
        }
        if self.mcp.is_some() {
            ui.label(
                RichText::new("Stdio MCP is active for the client that launched this window.")
                    .color(c::TEXT_DIM),
            );
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Access log").font(theme::bold(13.0)));
            ui.checkbox(&mut config.access_log, "Record requests");
            if ui.button("Clear").clicked()
                && let Ok(mut entries) = server.log.entries.lock()
            {
                entries.clear();
            }
        });
        server
            .log
            .enabled
            .store(config.access_log, Ordering::Relaxed);
        let entries = server
            .log
            .entries
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default();
        if entries.is_empty() {
            ui.label(RichText::new("No requests recorded.").color(c::TEXT_DIM));
        } else {
            ui.label(
                RichText::new("Elapsed · Client · Method · Status · Duration").color(c::TEXT_DIM),
            );
            for entry in entries.iter().rev() {
                ui.monospace(entry);
            }
        }
        if *config != before {
            self.settings.save();
        }
        if let Some(command) = command {
            self.run(command, ui.ctx());
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write as _};
    use std::net::TcpStream;

    fn ping(address: SocketAddr, headers: &str) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = r#"{"jsonrpc":"2.0","method":"ping","id":1}"#;
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn listener_checks_access_and_releases_its_port_on_stop() {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = probe.local_addr().unwrap();
        drop(probe);
        let config = McpSettings {
            port: address.port(),
            ..McpSettings::default()
        };
        let mut server = NetworkMcp::default();
        server.start(&config, &egui::Context::default());
        assert!(server.error.is_none(), "{:?}", server.error);
        let token = &server.listener.as_ref().unwrap().token;
        let auth = format!("Authorization: Bearer {token}\r\n");

        assert!(ping(address, "").starts_with("HTTP/1.1 401 "));
        let origin = format!("{auth}Origin: https://example.com\r\n");
        assert!(ping(address, &origin).starts_with("HTTP/1.1 403 "));
        let response = ping(address, &auth);
        assert!(response.starts_with("HTTP/1.1 200 "));
        assert!(response.contains(r#""result":{}"#));
        let entries = server.log.entries.lock().unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|entry| !entry.contains(token)));
        drop(entries);

        server.stop();
        server
            .listener
            .take()
            .unwrap()
            .thread
            .join()
            .unwrap()
            .unwrap();
        assert!(TcpStream::connect(address).is_err());
    }
}
