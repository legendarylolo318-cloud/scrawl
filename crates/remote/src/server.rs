//! HTTPS + WSS server (axum, tokio-rustls, TLS 1.3 only). Nothing exists until `start` is called;
//! `stop` (or idle timeout, or dropping the handle) tears down the runtime, threads and sockets.

use crate::ca::{self, is_private_ip, KeyStore};
use crate::pairing::{Device, Pairing};
use crate::protocol::{self, ClientMsg, Command, Pen, RateLimiter, ServerMsg};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch, Notify};
use tokio_rustls::TlsAcceptor;

const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
pub const BOOTSTRAP_WINDOW: Duration = Duration::from_secs(5 * 60);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub bootstrap_port: u16,
    pub idle_timeout: Duration,
    pub config_dir: PathBuf,
    /// Tests only: also bind 127.0.0.1 (never set in the app).
    pub allow_loopback_for_tests: bool,
}

impl ServerConfig {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { port: 8443, bootstrap_port: 8080, idle_timeout: Duration::from_secs(15 * 60), config_dir, allow_loopback_for_tests: false }
    }
}

/// What the desktop app receives.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteEvent {
    Strokes { id: u32, pen: Pen, width: f32, pts: Vec<[f32; 6]>, done: bool },
    Text(String),
    Cmd(Command),
    /// A device submitted a valid pairing token and waits for Approve on the PC.
    PendingApproval { id: String, name: String, code: String },
    DeviceConnected(String),
    DeviceDisconnected,
    Stopped(StopReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    User,
    Idle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Approved,
    Rejected,
}

/// Result of the PC's Approve/Reject click: (outcome, (device id, secret) when approved).
type OutcomeEntry = (Outcome, Option<(String, String)>);

struct Shared {
    pairing: Mutex<Pairing>,
    cfg: ServerConfig,
    events: mpsc::Sender<RemoteEvent>,
    outbound: broadcast::Sender<ServerMsg>,
    last_seq: Mutex<HashMap<String, u64>>,
    outcomes: Mutex<HashMap<String, OutcomeEntry>>,
    outcome_notify: Notify,
    last_activity: AtomicU64,
    connections: AtomicU64,
    bootstrap_open: AtomicBool,
    bootstrap_cert: Mutex<Option<ca::CaMaterial>>,
    hosts: Vec<String>,
    /// Bumped on every revoke so live sessions re-check that their device is still trusted.
    revoked: watch::Sender<u64>,
    /// Server certificate, swappable while running (renewal).
    cert: Arc<CertSlot>,
    leaf: Mutex<Option<ca::Leaf>>,
    keys: Arc<dyn KeyStore + Send + Sync>,
    ca_pem: String,
    renewals: AtomicU64,
}

/// Holds the current server certificate; rustls asks it on every handshake.
#[derive(Debug)]
struct CertSlot(RwLock<Arc<CertifiedKey>>);

impl ResolvesServerCert for CertSlot {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.0.read().ok().map(|k| k.clone())
    }
}

fn certified_key(cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Arc<CertifiedKey>, String> {
    let key = rustls_pki_types::PrivateKeyDer::try_from(key_der).map_err(|e| e.to_string())?;
    let signer = rustls::crypto::ring::sign::any_supported_type(&key).map_err(|e| e.to_string())?;
    Ok(Arc::new(CertifiedKey::new(vec![rustls_pki_types::CertificateDer::from(cert_der)], signer)))
}

impl Shared {
    /// Re-issue the leaf certificate when it is close to expiry. New handshakes use it at once.
    fn renew_if_due(&self, now: u64) -> bool {
        let mut leaf = self.leaf.lock().unwrap();
        let Some(cur) = leaf.as_ref() else { return false };
        if !cur.expires_soon(now) {
            return false;
        }
        let Ok(new) = ca::issue_leaf_inner(&*self.keys, &self.ca_pem, &cur.ips, now, self.cfg.allow_loopback_for_tests) else { return false };
        let Ok(ck) = certified_key(new.cert_der.clone(), new.key_der.clone()) else { return false };
        *self.cert.0.write().unwrap() = ck;
        *leaf = Some(new);
        self.renewals.fetch_add(1, Ordering::Relaxed);
        true
    }
    fn is_trusted(&self, device: &str) -> bool {
        self.pairing.lock().unwrap().devices().iter().any(|d| d.id == device)
    }
    fn touch(&self) {
        self.last_activity.store(now_ms(), Ordering::Relaxed);
    }
    fn persist_devices(&self) {
        let devices: Vec<Device> = self.pairing.lock().unwrap().devices().to_vec();
        let path = self.cfg.config_dir.join("devices.json");
        let _ = std::fs::create_dir_all(&self.cfg.config_dir);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&devices).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

pub struct PairingInfo {
    /// `https://<ip>:<port>/#<token>`: the token is in the fragment, which browsers never send.
    pub url: String,
    pub qr: Vec<Vec<bool>>,
    pub expires_in: Duration,
}

pub struct RemoteHandle {
    shared: Arc<Shared>,
    pub events: mpsc::Receiver<RemoteEvent>,
    stop_tx: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub addrs: Vec<SocketAddr>,
    pub ca_fingerprint: String,
    pub bootstrap_urls: Vec<String>,
}

fn load_devices(dir: &std::path::Path) -> Vec<Device> {
    std::fs::read(dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn private_ips(allow_loopback: bool) -> Vec<IpAddr> {
    let mut v: Vec<IpAddr> = if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| i.ip()).filter(|i| is_private_ip(*i)).collect();
    if allow_loopback {
        v.push(IpAddr::from([127, 0, 0, 1]));
    }
    v.sort();
    v.dedup();
    v
}

/// Start the server. Fails (without leaving anything running) if there is no private interface.
pub fn start(cfg: ServerConfig, keys: Arc<dyn KeyStore + Send + Sync>) -> Result<RemoteHandle, String> {
    let ips = private_ips(cfg.allow_loopback_for_tests);
    if ips.is_empty() {
        return Err("No private-network interface found (LAN or Tailscale). The tablet server never listens on public interfaces.".into());
    }
    std::fs::create_dir_all(&cfg.config_dir).map_err(|e| e.to_string())?;
    let ca_path = cfg.config_dir.join("ca.pem");
    let existing = std::fs::read_to_string(&ca_path).ok();
    let ca = ca::ensure_ca(&*keys, existing.as_deref(), now_ms())?;
    std::fs::write(&ca_path, &ca.cert_pem).map_err(|e| e.to_string())?;
    // Loopback is certified only in test mode (never in the app).
    let leaf = ca::issue_leaf_inner(&*keys, &ca.cert_pem, &ips, now_ms(), cfg.allow_loopback_for_tests)?;
    let fingerprint = ca.fingerprint();
    let (cert_der, key_der) = (leaf.cert_der.clone(), leaf.key_der.clone());
    let leaf = Some(leaf);
    let cert = Arc::new(CertSlot(RwLock::new(certified_key(cert_der, key_der)?)));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_cert_resolver(cert.clone());
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let ca_pem = ca.cert_pem.clone();

    let (events_tx, events_rx) = mpsc::channel();
    let (out_tx, _) = broadcast::channel(256);
    let hosts: Vec<String> = ips.iter().map(|i| format!("{i}:{}", cfg.port)).chain([format!("scrawl.local:{}", cfg.port)]).collect();
    let shared = Arc::new(Shared {
        pairing: Mutex::new(Pairing::with_devices(load_devices(&cfg.config_dir))),
        cfg: cfg.clone(),
        events: events_tx,
        outbound: out_tx,
        last_seq: Mutex::new(HashMap::new()),
        outcomes: Mutex::new(HashMap::new()),
        outcome_notify: Notify::new(),
        last_activity: AtomicU64::new(now_ms()),
        connections: AtomicU64::new(0),
        bootstrap_open: AtomicBool::new(false),
        bootstrap_cert: Mutex::new(Some(ca)),
        hosts,
        revoked: watch::channel(0).0,
        cert,
        leaf: Mutex::new(leaf),
        keys,
        ca_pem,
        renewals: AtomicU64::new(0),
    });
    let (stop_tx, stop_rx) = watch::channel(false);
    let (ready_tx, ready_rx) = mpsc::channel::<Result<Vec<SocketAddr>, String>>();
    let sh = shared.clone();
    let thread = std::thread::Builder::new()
        .name("scrawl-tablet-server".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(r) => r,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            rt.block_on(run(sh, acceptor, ips, stop_rx, ready_tx));
        })
        .map_err(|e| e.to_string())?;
    let addrs = ready_rx.recv().map_err(|e| e.to_string())??;
    let bootstrap_urls = addrs.iter().map(|a| format!("http://{}:{}/", a.ip(), cfg.bootstrap_port)).collect();
    Ok(RemoteHandle { shared, events: events_rx, stop_tx, thread: Some(thread), addrs, ca_fingerprint: fingerprint, bootstrap_urls })
}

async fn run(shared: Arc<Shared>, acceptor: TlsAcceptor, ips: Vec<IpAddr>, mut stop: watch::Receiver<bool>, ready: mpsc::Sender<Result<Vec<SocketAddr>, String>>) {
    let app = Router::new()
        .route("/", get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }))
        .route("/app.js", get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }))
        .route("/style.css", get(|| async { asset("text/css; charset=utf-8", STYLE_CSS) }))
        .route("/ws", get(ws_handler))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(axum::middleware::map_response(|r: Response| async move { secure(r) }))
        .with_state(shared.clone());
    let mut addrs = Vec::new();
    let mut listeners = Vec::new();
    for ip in &ips {
        match TcpListener::bind(SocketAddr::new(*ip, shared.cfg.port)).await {
            Ok(l) => {
                addrs.push(l.local_addr().unwrap());
                listeners.push(l);
            }
            Err(e) => {
                let _ = ready.send(Err(format!("cannot listen on {ip}:{}: {e}", shared.cfg.port)));
                return;
            }
        }
    }
    let _ = ready.send(Ok(addrs));
    shared.touch();
    for l in listeners {
        let (app, acc, mut stop2) = (app.clone(), acceptor.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop2.changed() => break,
                    r = l.accept() => {
                        let Ok((tcp, _)) = r else { continue };
                        let (app, acc) = (app.clone(), acc.clone());
                        tokio::spawn(async move {
                            let Ok(Ok(tls)) = tokio::time::timeout(Duration::from_secs(10), acc.accept(tcp)).await else { return };
                            let io = TokioIo::new(tls);
                            let _ = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                                .serve_connection_with_upgrades(io, TowerToHyperService::new(app))
                                .await;
                        });
                    }
                }
            }
        });
    }
    // Idle watchdog.
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            _ = tick.tick() => {
                if shared.connections.load(Ordering::Relaxed) > 0 { shared.touch(); }
                shared.renew_if_due(now_ms());
                let idle = now_ms().saturating_sub(shared.last_activity.load(Ordering::Relaxed));
                if idle > shared.cfg.idle_timeout.as_millis() as u64 {
                    let _ = shared.events.send(RemoteEvent::Stopped(StopReason::Idle));
                    break;
                }
            }
        }
    }
    // Returning drops the runtime, aborting every task and closing every socket.
}

fn asset(ct: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, ct)], body).into_response()
}

/// Apply security headers to every response.
fn secure(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    r
}

/// The Origin must be exactly `https://<one of our hosts>`.
fn origin_ok(headers: &HeaderMap, hosts: &[String]) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return false };
    let Some(host) = origin.strip_prefix("https://") else { return false };
    hosts.iter().any(|h| h == host)
}

async fn ws_handler(State(sh): State<Arc<Shared>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !origin_ok(&headers, &sh.hosts) {
        return secure((StatusCode::FORBIDDEN, "bad origin").into_response());
    }
    secure(ws.max_message_size(protocol::MAX_MESSAGE_BYTES).max_frame_size(protocol::MAX_MESSAGE_BYTES).on_upgrade(move |s| socket(sh, s)))
}

async fn send(ws: &mut WebSocket, m: &ServerMsg) -> bool {
    ws.send(Message::Text(serde_json::to_string(m).unwrap_or_default().into())).await.is_ok()
}

async fn socket(sh: Arc<Shared>, mut ws: WebSocket) {
    let start = std::time::Instant::now();
    let mut limiter = RateLimiter::new(400.0, 800.0, 0);
    let mut device: Option<String> = None;
    let mut out_rx = sh.outbound.subscribe();
    let mut rev_rx = sh.revoked.subscribe();
    sh.connections.fetch_add(1, Ordering::Relaxed);
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            // Unauthenticated sockets get 10 seconds to pair or authenticate.
            _ = &mut deadline, if device.is_none() => break,
            // A revoke on the PC cuts off live sessions of that device immediately.
            _ = rev_rx.changed(), if device.is_some() => {
                if !device.as_deref().is_some_and(|d| sh.is_trusted(d)) {
                    let _ = send(&mut ws, &ServerMsg::Err { msg: "device revoked: pair this device again".into() }).await;
                    break;
                }
            }
            fwd = out_rx.recv(), if device.is_some() => {
                match fwd { Ok(m) => { if !send(&mut ws, &m).await { break; } } Err(broadcast::error::RecvError::Lagged(_)) => {} Err(_) => break }
            }
            msg = ws.recv() => {
                let Some(Ok(msg)) = msg else { break };
                let frame = match msg { Message::Text(t) => t.as_bytes().to_vec(), Message::Close(_) => break, Message::Ping(_) | Message::Pong(_) => continue, Message::Binary(_) => break };
                if !limiter.allow(start.elapsed().as_millis() as u64) { continue; } // drop floods silently
                let Ok(m) = protocol::validate(&frame) else { continue };            // drop malformed input
                sh.touch();
                match (&device, m) {
                    (_, ClientMsg::Ping {}) => { if !send(&mut ws, &ServerMsg::Pong).await { break; } }
                    (None, ClientMsg::Pair { token, nonce, name }) => {
                        let r = sh.pairing.lock().unwrap().submit(now_ms(), &token, &nonce, &name);
                        match r {
                            Ok(p) => {
                                let _ = sh.events.send(RemoteEvent::PendingApproval { id: p.id.clone(), name: p.name.clone(), code: p.code.clone() });
                                if !send(&mut ws, &ServerMsg::PairCode { code: p.code.clone() }).await { break; }
                                // wait up to 2 minutes for Approve / Reject on the PC
                                let end = tokio::time::Instant::now() + Duration::from_secs(120);
                                let result = loop {
                                    if let Some(o) = sh.outcomes.lock().unwrap().remove(&p.id) { break Some(o); }
                                    if tokio::time::timeout_at(end, sh.outcome_notify.notified()).await.is_err() { break None; }
                                };
                                match result {
                                    Some((Outcome::Approved, Some((id, secret)))) => {
                                        sh.persist_devices();
                                        if !send(&mut ws, &ServerMsg::Paired { device: id, secret }).await { break; }
                                        // the client now authenticates on this same socket
                                    }
                                    _ => { sh.pairing.lock().unwrap().reject(&p.id); let _ = send(&mut ws, &ServerMsg::Err { msg: "pairing rejected".into() }).await; break; }
                                }
                            }
                            Err(_) => { let _ = send(&mut ws, &ServerMsg::Err { msg: "pairing failed".into() }).await; break; }
                        }
                    }
                    (None, ClientMsg::Auth { device: id, secret }) => {
                        let r = sh.pairing.lock().unwrap().authenticate(now_ms(), &id, &secret);
                        match r {
                            Ok(new_secret) => {
                                sh.persist_devices();
                                let last = sh.last_seq.lock().unwrap().get(&id).copied().unwrap_or(0);
                                if !send(&mut ws, &ServerMsg::AuthOk { secret: new_secret, last_seq: last }).await { break; }
                                let name = sh.pairing.lock().unwrap().list().into_iter().find(|d| d.id == id).map(|d| d.name).unwrap_or_default();
                                let _ = sh.events.send(RemoteEvent::DeviceConnected(name));
                                device = Some(id);
                            }
                            Err(_) => { let _ = send(&mut ws, &ServerMsg::Err { msg: "auth failed: pair this device again".into() }).await; break; }
                        }
                    }
                    (Some(id), ClientMsg::Auth { .. } | ClientMsg::Pair { .. }) => { let _ = id; }
                    (Some(id), ClientMsg::Resume { .. }) => { let _ = id; }
                    (Some(id), ClientMsg::Strokes { seq, id: sid, pen, width, pts, done }) => {
                        if accept_seq(&sh, id, seq) { let _ = sh.events.send(RemoteEvent::Strokes { id: sid, pen, width, pts, done }); }
                        if !send(&mut ws, &ServerMsg::Ack { seq }).await { break; }
                    }
                    (Some(id), ClientMsg::Text { seq, s }) => {
                        if accept_seq(&sh, id, seq) { let _ = sh.events.send(RemoteEvent::Text(s)); }
                        if !send(&mut ws, &ServerMsg::Ack { seq }).await { break; }
                    }
                    (Some(id), ClientMsg::Cmd { seq, cmd }) => {
                        if accept_seq(&sh, id, seq) { let _ = sh.events.send(RemoteEvent::Cmd(cmd)); }
                        if !send(&mut ws, &ServerMsg::Ack { seq }).await { break; }
                    }
                    // Anything else before authentication: drop the connection.
                    (None, _) => break,
                }
            }
        }
    }
    sh.connections.fetch_sub(1, Ordering::Relaxed);
    if device.is_some() {
        let _ = sh.events.send(RemoteEvent::DeviceDisconnected);
    }
}

/// Exactly-once delivery across reconnects: only a seq above the last processed one is applied.
fn accept_seq(sh: &Shared, device: &str, seq: u64) -> bool {
    let mut m = sh.last_seq.lock().unwrap();
    let last = m.entry(device.to_string()).or_insert(0);
    if seq > *last {
        *last = seq;
        true
    } else {
        false
    }
}

// ---- one-time CA bootstrap over plain HTTP -------------------------------------------------

async fn serve_bootstrap(sh: Arc<Shared>, listener: TcpListener, done: Arc<Notify>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let Ok((mut s, _)) = listener.accept().await else { return };
        let mut buf = [0u8; 1024];
        let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        let ca = sh.bootstrap_cert.lock().unwrap().as_ref().map(|c| (c.cert_der.clone(), c.cert_pem.clone()));
        let (status, ctype, body, final_download) = match (path.as_str(), ca) {
            ("/ca.crt", Some((der, _))) => ("200 OK", "application/x-x509-ca-cert", der, true),
            ("/ca.mobileconfig", Some((der, _))) => ("200 OK", "application/x-apple-aspen-config", ca::mobileconfig(&der, &crate::pairing::random_hex(8), &crate::pairing::random_hex(8)).into_bytes(), true),
            ("/", _) => ("200 OK", "text/html; charset=utf-8", b"<!doctype html><meta name=viewport content='width=device-width'><h1>Scrawl</h1><p><a href='/ca.mobileconfig'>iPhone / iPad profile</a></p><p><a href='/ca.crt'>Android certificate (.crt)</a></p>".to_vec(), false),
            _ => ("404 Not Found", "text/plain", b"not found".to_vec(), false),
        };
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nContent-Disposition: inline\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
        let _ = s.write_all(head.as_bytes()).await;
        let _ = s.write_all(&body).await;
        let _ = s.shutdown().await;
        if final_download {
            done.notify_one(); // one download closes the bootstrap endpoint
            return;
        }
    }
}

impl RemoteHandle {
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// New single-use pairing token as a URL + QR modules.
    pub fn begin_pairing(&self) -> PairingInfo {
        let token = self.shared.pairing.lock().unwrap().begin(now_ms());
        let addr = self.addrs.iter().find(|a| !a.ip().is_loopback()).or(self.addrs.first()).copied().unwrap();
        let url = format!("https://{}:{}/#{token}", addr.ip(), addr.port());
        let qr = qrcode::QrCode::new(url.as_bytes()).map(|c| {
            let w = c.width();
            let cols = c.to_colors();
            (0..w).map(|y| (0..w).map(|x| cols[y * w + x] == qrcode::Color::Dark).collect()).collect()
        });
        PairingInfo { url, qr: qr.unwrap_or_default(), expires_in: Duration::from_millis(crate::pairing::TOKEN_TTL_MS) }
    }

    pub fn approve(&self, pending_id: &str) -> bool {
        let r = self.shared.pairing.lock().unwrap().approve(now_ms(), pending_id);
        let ok = r.is_some();
        self.shared.outcomes.lock().unwrap().insert(pending_id.to_string(), if ok { (Outcome::Approved, r) } else { (Outcome::Rejected, None) });
        self.shared.outcome_notify.notify_waiters();
        ok
    }

    pub fn reject(&self, pending_id: &str) {
        self.shared.pairing.lock().unwrap().reject(pending_id);
        self.shared.outcomes.lock().unwrap().insert(pending_id.to_string(), (Outcome::Rejected, None));
        self.shared.outcome_notify.notify_waiters();
    }

    pub fn devices(&self) -> Vec<crate::pairing::DeviceInfo> {
        self.shared.pairing.lock().unwrap().list()
    }

    pub fn revoke(&self, id: &str) -> bool {
        let r = self.shared.pairing.lock().unwrap().revoke(id);
        self.shared.revoked.send_modify(|g| *g += 1);
        self.shared.persist_devices();
        r
    }

    /// Reset all pairing (SECURITY.md "reset").
    pub fn revoke_all(&self) {
        self.shared.pairing.lock().unwrap().revoke_all();
        self.shared.revoked.send_modify(|g| *g += 1);
        self.shared.persist_devices();
    }

    /// Push a state / candidate / frame update to connected devices.
    pub fn send(&self, m: ServerMsg) {
        let _ = self.shared.outbound.send(m);
    }

    /// Re-issue the server certificate if it is due at `now` (the watchdog calls this every 2 s).
    pub fn renew_if_due(&self, now: u64) -> bool {
        self.shared.renew_if_due(now)
    }

    /// How many times the server certificate was renewed while running.
    pub fn cert_renewals(&self) -> u64 {
        self.shared.renewals.load(Ordering::Relaxed)
    }

    pub fn connected_devices(&self) -> u64 {
        self.shared.connections.load(Ordering::Relaxed)
    }

    /// Open the plain-HTTP bootstrap endpoint that serves only the CA certificate. It closes after
    /// one download or 5 minutes. Returns false if it is already open.
    pub fn open_bootstrap(&self) -> Result<(), String> {
        if self.shared.bootstrap_open.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let sh = self.shared.clone();
        let ips: Vec<IpAddr> = self.addrs.iter().map(|a| a.ip()).collect();
        let port = sh.cfg.bootstrap_port;
        let (tx, rx) = mpsc::channel();
        let mut stop = self.stop_tx.subscribe();
        // run on its own short-lived thread + runtime, owned by the server lifetime
        std::thread::Builder::new()
            .name("scrawl-ca-bootstrap".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
                rt.block_on(async {
                    let done = Arc::new(Notify::new());
                    let mut ok = false;
                    let mut tasks = Vec::new();
                    for ip in ips {
                        if let Ok(l) = TcpListener::bind(SocketAddr::new(ip, port)).await {
                            ok = true;
                            tasks.push(tokio::spawn(serve_bootstrap(sh.clone(), l, done.clone())));
                        }
                    }
                    let _ = tx.send(ok);
                    tokio::select! {
                        _ = done.notified() => {}
                        _ = tokio::time::sleep(BOOTSTRAP_WINDOW) => {}
                        _ = stop.changed() => {}
                    }
                    for t in tasks { t.abort(); }
                });
                sh.bootstrap_open.store(false, Ordering::SeqCst);
            })
            .map_err(|e| e.to_string())?;
        if rx.recv().unwrap_or(false) {
            Ok(())
        } else {
            self.shared.bootstrap_open.store(false, Ordering::SeqCst);
            Err("could not open the bootstrap port".into())
        }
    }

    pub fn bootstrap_open(&self) -> bool {
        self.shared.bootstrap_open.load(Ordering::SeqCst)
    }

    /// Stop everything: sockets, threads and the runtime.
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for RemoteHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
