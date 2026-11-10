//! End-to-end tests against a real server on loopback (test-only escape hatch).

use futures_util::{SinkExt, StreamExt};
use remote::ca::MemStore;
use remote::server::{start, RemoteEvent, RemoteHandle, ServerConfig, StopReason};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[derive(Debug)]
struct AcceptAny;
impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn server(idle: Duration) -> (RemoteHandle, u16, u16) {
    let dir = std::env::temp_dir().join(format!("iw-remote-{}-{}", std::process::id(), free_port()));
    let (port, bport) = (free_port(), free_port());
    let mut cfg = ServerConfig::new(dir);
    cfg.port = port;
    cfg.bootstrap_port = bport;
    cfg.idle_timeout = idle;
    cfg.allow_loopback_for_tests = true;
    (start(cfg, Arc::new(MemStore::default())).expect("server starts"), port, bport)
}

fn client_cfg(versions: &[&'static rustls::SupportedProtocolVersion]) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider).with_protocol_versions(versions).unwrap().dangerous().with_custom_certificate_verifier(Arc::new(AcceptAny)).with_no_client_auth(),
    )
}

async fn tls(port: u16, versions: &[&'static rustls::SupportedProtocolVersion]) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
    TlsConnector::from(client_cfg(versions)).connect(ServerName::try_from("localhost").unwrap(), tcp).await
}

async fn ws(port: u16, origin: Option<&str>) -> Result<Ws, String> {
    let s = tls(port, &[&rustls::version::TLS13]).await.map_err(|e| e.to_string())?;
    let mut req = format!("wss://127.0.0.1:{port}/ws").into_client_request().unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("Origin", o.parse().unwrap());
    }
    tokio_tungstenite::client_async(req, s).await.map(|(w, _)| w).map_err(|e| e.to_string())
}

async fn recv_json(w: &mut Ws) -> serde_json::Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), w.next()).await.expect("timeout").expect("closed").expect("ws error") {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            _ => continue,
        }
    }
}

fn pair_event(h: &RemoteHandle) -> (String, String) {
    loop {
        match h.events.recv_timeout(Duration::from_secs(5)).expect("event") {
            RemoteEvent::PendingApproval { id, code, .. } => return (id, code),
            _ => continue,
        }
    }
}

/// Full pairing; returns (device, secret).
async fn do_pairing(h: &RemoteHandle, port: u16) -> (String, String) {
    let info = h.begin_pairing();
    assert!(info.url.contains('#') && !info.url.contains('?'), "token lives in the fragment only");
    assert!(!info.qr.is_empty());
    let token = info.url.split('#').nth(1).unwrap().to_string();
    let mut w = ws(port, Some(&format!("https://127.0.0.1:{port}"))).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"pair","token":"{token}","nonce":"abc123","name":"Test iPad"}}"#))).await.unwrap();
    let code_msg = recv_json(&mut w).await;
    assert_eq!(code_msg["t"], "pair_code");
    let (pid, pc_code) = pair_event(h);
    assert_eq!(code_msg["code"], pc_code, "device and PC show the same 6-digit code");
    assert_eq!(pc_code, remote::pairing::confirmation_code(&token, "abc123"));
    assert!(h.devices().is_empty(), "not trusted before Approve");
    assert!(h.approve(&pid));
    let paired = recv_json(&mut w).await;
    assert_eq!(paired["t"], "paired");
    (paired["device"].as_str().unwrap().into(), paired["secret"].as_str().unwrap().into())
}

#[tokio::test]
async fn https_serves_ui_with_strict_headers_and_tls13_only() {
    let (h, port, _) = server(Duration::from_secs(60));
    let mut s = tls(port, &[&rustls::version::TLS13]).await.unwrap();
    assert_eq!(s.get_ref().1.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut body = String::new();
    s.read_to_string(&mut body).await.ok();
    let lower = body.to_lowercase();
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    for hdr in ["content-security-policy: default-src 'none'", "strict-transport-security", "x-content-type-options: nosniff", "referrer-policy: no-referrer"] {
        assert!(lower.contains(hdr), "missing {hdr}");
    }
    assert!(!lower.contains("cdn.") && !body.contains("https://fonts"), "no third-party requests");
    assert!(body.contains("/app.js"));
    assert!(tls(port, &[&rustls::version::TLS12]).await.is_err(), "TLS 1.2 must be rejected");
    h.stop();
}

#[tokio::test]
async fn websocket_origin_is_checked() {
    let (h, port, _) = server(Duration::from_secs(60));
    assert!(ws(port, None).await.is_err(), "missing Origin");
    assert!(ws(port, Some("https://evil.example")).await.is_err(), "foreign Origin");
    assert!(ws(port, Some(&format!("http://127.0.0.1:{port}"))).await.is_err(), "non-https Origin");
    assert!(ws(port, Some(&format!("https://127.0.0.1:{port}"))).await.is_ok());
    h.stop();
}

#[tokio::test]
async fn pairing_reconnect_rotation_and_exactly_once_strokes() {
    let (h, port, _) = server(Duration::from_secs(60));
    let origin = format!("https://127.0.0.1:{port}");
    let (dev, secret) = do_pairing(&h, port).await;
    assert_eq!(h.devices().len(), 1);

    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"auth","device":"{dev}","secret":"{secret}"}}"#))).await.unwrap();
    let ok = recv_json(&mut w).await;
    assert_eq!(ok["t"], "auth_ok");
    let new_secret = ok["secret"].as_str().unwrap().to_string();
    assert_ne!(new_secret, secret);
    let stroke = r#"{"t":"strokes","seq":1,"id":7,"pen":"pen","width":3.0,"pts":[[1,2,0.5,0,0,0],[3,4,0.6,0,0,8]],"done":true}"#;
    w.send(Message::text(stroke)).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "ack");
    w.send(Message::text(stroke)).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "ack", "duplicates are acked again");
    w.send(Message::text(r#"{"t":"text","seq":2,"s":"alpha"}"#)).await.unwrap();
    w.send(Message::text(r#"{"t":"cmd","seq":3,"cmd":{"c":"undo"}}"#)).await.unwrap();
    w.send(Message::text("garbage that must be dropped")).await.unwrap();
    w.send(Message::text(r#"{"t":"list_notes"}"#)).await.unwrap();
    let mut got = Vec::new();
    while got.len() < 3 {
        if let e @ (RemoteEvent::Strokes { .. } | RemoteEvent::Text(_) | RemoteEvent::Cmd(_)) = h.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            got.push(e);
        }
    }
    assert!(matches!(&got[0], RemoteEvent::Strokes { id: 7, done: true, .. }));
    assert_eq!(got[1], RemoteEvent::Text("alpha".into()));
    assert!(matches!(got[2], RemoteEvent::Cmd(remote::protocol::Command::Undo {})));
    assert!(h.events.recv_timeout(Duration::from_millis(300)).map_or(true, |e| !matches!(e, RemoteEvent::Strokes { .. })), "the duplicate stroke was not applied twice");
    h.send(remote::protocol::ServerMsg::Candidates { labels: vec!["x".into(), "×".into()] });
    loop {
        let m = recv_json(&mut w).await;
        if m["t"] == "candidates" {
            break;
        }
        assert_eq!(m["t"], "ack");
    }
    drop(w);

    // the old secret no longer works; the new one does
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"auth","device":"{dev}","secret":"{secret}"}}"#))).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "err");
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"auth","device":"{dev}","secret":"{new_secret}"}}"#))).await.unwrap();
    let ok2 = recv_json(&mut w).await;
    assert_eq!(ok2["t"], "auth_ok");
    // revoke from the PC
    assert!(h.revoke(&dev));
    let latest = ok2["secret"].as_str().unwrap().to_string();
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"auth","device":"{dev}","secret":"{latest}"}}"#))).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "err", "revoked devices are refused");
    h.stop();
}

#[tokio::test]
async fn unauthenticated_ink_is_refused_and_pairing_locks_after_failures() {
    let (h, port, _) = server(Duration::from_secs(60));
    let origin = format!("https://127.0.0.1:{port}");
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(r#"{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3,"pts":[[1,1,0.5,0,0,0]],"done":true}"#)).await.unwrap();
    let r = tokio::time::timeout(Duration::from_secs(3), w.next()).await.unwrap();
    assert!(matches!(r, None | Some(Err(_)) | Some(Ok(Message::Close(_)))), "connection dropped, no ink accepted");
    assert!(h.events.recv_timeout(Duration::from_millis(300)).is_err(), "no event reached the app");
    for _ in 0..5 {
        h.begin_pairing();
        let mut w = ws(port, Some(&origin)).await.unwrap();
        w.send(Message::text(format!(r#"{{"t":"pair","token":"{}","nonce":"n1","name":"x"}}"#, "0".repeat(32)))).await.unwrap();
        assert_eq!(recv_json(&mut w).await["t"], "err");
    }
    let info = h.begin_pairing();
    let token = info.url.split('#').nth(1).unwrap();
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"pair","token":"{token}","nonce":"n1","name":"x"}}"#))).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "err", "locked out");
    h.stop();
}

#[tokio::test]
async fn bootstrap_serves_only_the_ca_once_and_closes() {
    let (h, _port, bport) = server(Duration::from_secs(60));
    assert!(!h.bootstrap_open());
    assert!(TcpStream::connect(("127.0.0.1", bport)).await.is_err(), "nothing listens until the setup screen opens it");
    h.open_bootstrap().unwrap();
    assert!(h.bootstrap_open());
    let get = |path: &'static str| async move {
        let mut s = TcpStream::connect(("127.0.0.1", bport)).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut b = Vec::new();
        s.read_to_end(&mut b).await.unwrap();
        b
    };
    let nf = get("/notes").await;
    assert!(String::from_utf8_lossy(&nf).starts_with("HTTP/1.1 404"), "only the CA is served");
    let crt = get("/ca.crt").await;
    let split = crt.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(crt[split], 0x30, "DER certificate");
    assert!(!String::from_utf8_lossy(&crt[split..]).contains("PRIVATE KEY"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(TcpStream::connect(("127.0.0.1", bport)).await.is_err(), "closed after one download");
    h.stop();
}

#[tokio::test]
async fn idle_timeout_stops_the_server_and_stop_closes_ports() {
    let (h, port, _) = server(Duration::from_secs(2));
    loop {
        match h.events.recv_timeout(Duration::from_secs(10)).expect("idle stop event") {
            RemoteEvent::Stopped(StopReason::Idle) => break,
            _ => continue,
        }
    }
    h.stop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err(), "port closed after stop");
}

async fn peer_cert(port: u16) -> Vec<u8> {
    let s = tls(port, &[&rustls::version::TLS13]).await.unwrap();
    s.get_ref().1.peer_certificates().unwrap()[0].to_vec()
}

#[tokio::test]
async fn revoking_a_device_cuts_off_its_live_session() {
    let (h, port, _) = server(Duration::from_secs(60));
    let origin = format!("https://127.0.0.1:{port}");
    let (dev, secret) = do_pairing(&h, port).await;
    let mut w = ws(port, Some(&origin)).await.unwrap();
    w.send(Message::text(format!(r#"{{"t":"auth","device":"{dev}","secret":"{secret}"}}"#))).await.unwrap();
    assert_eq!(recv_json(&mut w).await["t"], "auth_ok");
    assert!(h.revoke(&dev));
    let m = recv_json(&mut w).await;
    assert_eq!(m["t"], "err", "the live session is told it was revoked");
    // and then the socket is closed: further ink never arrives
    let _ = w.send(Message::text(r#"{"t":"strokes","seq":9,"id":1,"pen":"pen","width":3.0,"pts":[[1,2,0.5,0,0,0]],"done":true}"#)).await;
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match w.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return true,
                _ => continue,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(closed, "revoked session is closed");
    assert!(h.events.recv_timeout(Duration::from_millis(300)).map_or(true, |e| !matches!(e, RemoteEvent::Strokes { .. })));
    h.stop();
}

#[tokio::test]
async fn leaf_certificate_is_renewed_while_running() {
    let (h, port, _) = server(Duration::from_secs(60));
    let before = peer_cert(port).await;
    assert!(!h.renew_if_due(remote::server::now_ms()), "fresh certificate is not renewed");
    let later = remote::server::now_ms() + (remote::ca::LEAF_DAYS as u64 - 1) * 86_400_000;
    assert!(h.renew_if_due(later), "renewed close to expiry");
    assert_eq!(h.cert_renewals(), 1);
    let after = peer_cert(port).await;
    assert_ne!(before, after, "new handshakes get the new certificate without a restart");
    h.stop();
}

async fn http_get(port: u16, path: &str) -> String {
    let mut s = tls(port, &[&rustls::version::TLS13]).await.unwrap();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut body = String::new();
    s.read_to_string(&mut body).await.ok();
    body
}

#[tokio::test]
async fn static_routes_resist_traversal_and_hostile_frames_are_dropped() {
    let (h, port, _) = server(Duration::from_secs(60));
    for p in ["/../../etc/passwd", "/%2e%2e/%2e%2e/etc/passwd", "/..%2fCargo.toml", "/app.js/../../ca.pem", "//etc/passwd", "/devices.json", "/ca.pem", "/notes"] {
        let r = http_get(port, p).await;
        assert!(!r.starts_with("HTTP/1.1 200"), "{p} must not be served: {r}");
        assert!(!r.contains("BEGIN CERTIFICATE") && !r.contains("secret_hash") && !r.contains("root:"), "{p} leaked data");
    }
    // unauthenticated junk, an oversized frame and a huge point count never reach the app
    let origin = format!("https://127.0.0.1:{port}");
    let mut w = ws(port, Some(&origin)).await.unwrap();
    let big: String = format!(r#"{{"t":"text","seq":1,"s":"{}"}}"#, "x".repeat(remote::protocol::MAX_MESSAGE_BYTES * 2));
    let _ = w.send(Message::text(big)).await;
    let pts: Vec<String> = (0..5000).map(|i| format!("[{i},1,0.5,0,0,1]")).collect();
    let mut w2 = ws(port, Some(&origin)).await.unwrap();
    let _ = w2.send(Message::text(format!(r#"{{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3.0,"pts":[{}],"done":true}}"#, pts.join(",")))).await;
    let mut w3 = ws(port, Some(&origin)).await.unwrap();
    for i in 0..2000 {
        if w3.send(Message::text(format!(r#"{{"t":"text","seq":{i},"s":"flood"}}"#))).await.is_err() {
            break;
        }
    }
    let leaked = h.events.recv_timeout(Duration::from_millis(500));
    assert!(leaked.map_or(true, |e| !matches!(e, RemoteEvent::Strokes { .. } | RemoteEvent::Text(_))), "nothing from unpaired sockets is applied");
    // the server is still healthy afterwards
    assert!(http_get(port, "/").await.starts_with("HTTP/1.1 200"));
    h.stop();
}
