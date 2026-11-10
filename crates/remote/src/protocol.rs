//! Wire protocol (JSON text frames over WSS) and strict validation of everything a device sends.
//!
//! The device may only: pair/authenticate, send strokes, send text, and send UI commands for the
//! active block. There is deliberately no message for reading, listing, exporting or deleting notes.

use serde::{Deserialize, Serialize};

pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
pub const MAX_POINTS_PER_BATCH: usize = 256;
pub const MAX_TEXT_CHARS: usize = 512;
const COORD_LIMIT: f32 = 100_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pen {
    Pen,
    Highlighter,
    Eraser,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "k", rename_all = "snake_case", deny_unknown_fields)]
pub enum Template {
    Fraction {},
    Sqrt {},
    Matrix { rows: u8, cols: u8 },
    Integral {},
    Sum {},
    Limit {},
    Cases {},
    Abs {},
    Norm {},
    Deriv {},
    Vector {},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavKey {
    Backspace,
    Left,
    Right,
    Tab,
    ShiftTab,
    Enter,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "c", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Undo {},
    Redo {},
    RecognizeNow {},
    Mode { drawing: bool },
    Tool { pen: Pen },
    Color { rgb: u32 },
    Choose { i: u8 },
    Template { t: Template },
    Nav { key: NavKey },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMsg {
    /// First contact: single-use pairing token from the QR code plus a client nonce.
    Pair { token: String, nonce: String, name: String },
    /// Reconnect with the per-device secret.
    Auth { device: String, secret: String },
    /// Ink points: [x, y, pressure, tilt_x, tilt_y, dt_ms].
    Strokes { seq: u64, id: u32, pen: Pen, width: f32, pts: Vec<[f32; 6]>, done: bool },
    Text { seq: u64, s: String },
    Cmd { seq: u64, cmd: Command },
    /// After reconnecting: "I have sent up to `seq`".
    Resume { seq: u64 },
    Ping {},
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    PairCode { code: String },
    Paired { device: String, secret: String },
    AuthOk { secret: String, last_seq: u64 },
    Ack { seq: u64 },
    Candidates { labels: Vec<String> },
    State { drawing: bool, can_undo: bool, can_redo: bool, expr: String },
    /// Render of the active block only (PNG, base64).
    Frame { png: String },
    Err { msg: String },
    Pong,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reject {
    TooLarge,
    Malformed,
    OutOfRange(&'static str),
}

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

/// Parse and validate one text frame. Never panics; every failure is a `Reject`.
pub fn validate(frame: &[u8]) -> Result<ClientMsg, Reject> {
    if frame.len() > MAX_MESSAGE_BYTES {
        return Err(Reject::TooLarge);
    }
    let msg: ClientMsg = serde_json::from_slice(frame).map_err(|_| Reject::Malformed)?;
    match &msg {
        ClientMsg::Pair { token, nonce, name } => {
            if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Reject::OutOfRange("token"));
            }
            if nonce.is_empty() || nonce.len() > 64 || !nonce.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(Reject::OutOfRange("nonce"));
            }
            if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
                return Err(Reject::OutOfRange("name"));
            }
        }
        ClientMsg::Auth { device, secret } => {
            if device.len() != 32 || !device.bytes().all(|b| b.is_ascii_hexdigit()) || secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Reject::OutOfRange("credentials"));
            }
        }
        ClientMsg::Strokes { pts, width, id, .. } => {
            if pts.is_empty() || pts.len() > MAX_POINTS_PER_BATCH {
                return Err(Reject::OutOfRange("points"));
            }
            if !finite_in(*width, 0.1, 100.0) {
                return Err(Reject::OutOfRange("width"));
            }
            if *id > 1_000_000 {
                return Err(Reject::OutOfRange("id"));
            }
            for p in pts {
                if !finite_in(p[0], -COORD_LIMIT, COORD_LIMIT) || !finite_in(p[1], -COORD_LIMIT, COORD_LIMIT) {
                    return Err(Reject::OutOfRange("coordinate"));
                }
                if !finite_in(p[2], 0.0, 1.0) || !finite_in(p[3], -90.0, 90.0) || !finite_in(p[4], -90.0, 90.0) || !finite_in(p[5], 0.0, 10_000.0) {
                    return Err(Reject::OutOfRange("pressure/tilt/time"));
                }
            }
        }
        ClientMsg::Text { s, .. } => {
            if s.chars().count() > MAX_TEXT_CHARS || s.chars().any(|c| c.is_control() && c != '\n') {
                return Err(Reject::OutOfRange("text"));
            }
        }
        ClientMsg::Cmd { cmd, .. } => match cmd {
            Command::Choose { i } if *i > 4 => return Err(Reject::OutOfRange("candidate")),
            Command::Color { rgb } if *rgb > 0xFF_FFFF => return Err(Reject::OutOfRange("color")),
            Command::Template { t: Template::Matrix { rows, cols } } if !(1..=6).contains(rows) || !(1..=6).contains(cols) => {
                return Err(Reject::OutOfRange("matrix size"))
            }
            _ => {}
        },
        ClientMsg::Resume { .. } | ClientMsg::Ping {} => {}
    }
    Ok(msg)
}

/// Token bucket: `rate` messages per second with `burst` capacity.
pub struct RateLimiter {
    rate: f64,
    burst: f64,
    tokens: f64,
    last_ms: u64,
}

impl RateLimiter {
    pub fn new(rate_per_sec: f64, burst: f64, now_ms: u64) -> Self {
        Self { rate: rate_per_sec, burst, tokens: burst, last_ms: now_ms }
    }
    pub fn allow(&mut self, now_ms: u64) -> bool {
        let dt = now_ms.saturating_sub(self.last_ms) as f64 / 1000.0;
        self.last_ms = now_ms;
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl ServerMsg {
    /// The active block's render (PNG bytes) for the device; an empty slice hides it.
    pub fn frame(png: &[u8]) -> Self {
        use base64::Engine;
        ServerMsg::Frame { png: base64::engine::general_purpose::STANDARD.encode(png) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Result<ClientMsg, Reject> {
        validate(s.as_bytes())
    }

    #[test]
    fn accepts_valid_messages() {
        assert!(matches!(v(r#"{"t":"strokes","seq":1,"id":3,"pen":"pen","width":3.0,"pts":[[1,2,0.5,0,0,8],[3,4,0.6,1,-1,16]],"done":false}"#), Ok(ClientMsg::Strokes { .. })));
        assert!(matches!(v(r#"{"t":"text","seq":2,"s":"alpha"}"#), Ok(ClientMsg::Text { .. })));
        assert!(matches!(v(r#"{"t":"cmd","seq":3,"cmd":{"c":"undo"}}"#), Ok(ClientMsg::Cmd { .. })));
        assert!(v(r#"{"t":"cmd","seq":4,"cmd":{"c":"template","t":{"k":"matrix","rows":3,"cols":3}}}"#).is_ok());
        assert!(matches!(v(r#"{"t":"ping"}"#), Ok(ClientMsg::Ping {})));
    }

    #[test]
    fn rejects_malformed_oversized_and_out_of_range() {
        assert_eq!(v("not json"), Err(Reject::Malformed));
        assert_eq!(v(r#"{"t":"unknown"}"#), Err(Reject::Malformed));
        assert_eq!(v(r#"{"t":"ping","extra":1}"#), Err(Reject::Malformed), "unknown fields are refused");
        assert_eq!(validate(&vec![b' '; MAX_MESSAGE_BYTES + 1]), Err(Reject::TooLarge));
        let many = format!(r#"{{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3,"pts":[{}],"done":true}}"#, vec!["[1,1,0.5,0,0,1]"; MAX_POINTS_PER_BATCH + 1].join(","));
        assert!(matches!(v(&many), Err(Reject::OutOfRange("points"))));
        assert!(matches!(v(r#"{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3,"pts":[[1e9,1,0.5,0,0,1]],"done":true}"#), Err(Reject::OutOfRange("coordinate"))));
        assert!(matches!(v(r#"{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3,"pts":[[1,1,7,0,0,1]],"done":true}"#), Err(Reject::OutOfRange(_))));
        assert!(matches!(v(r#"{"t":"strokes","seq":1,"id":1,"pen":"pen","width":3,"pts":[],"done":true}"#), Err(Reject::OutOfRange("points"))));
        assert!(matches!(v(r#"{"t":"text","seq":1,"s":"a\u0000b"}"#), Err(Reject::OutOfRange("text"))));
        assert!(matches!(v(r#"{"t":"cmd","seq":1,"cmd":{"c":"choose","i":9}}"#), Err(Reject::OutOfRange("candidate"))));
        assert!(matches!(v(r#"{"t":"cmd","seq":1,"cmd":{"c":"template","t":{"k":"matrix","rows":9,"cols":2}}}"#), Err(Reject::OutOfRange("matrix size"))));
        assert!(matches!(v(r#"{"t":"pair","token":"zz","nonce":"abc","name":"iPad"}"#), Err(Reject::OutOfRange("token"))));
    }

    #[test]
    fn no_message_can_read_or_delete_notes() {
        for forbidden in ["list", "read", "export", "delete", "open", "fs", "get_page"] {
            assert_eq!(v(&format!(r#"{{"t":"{forbidden}"}}"#)), Err(Reject::Malformed), "{forbidden}");
        }
    }

    #[test]
    fn fuzz_garbage_never_panics() {
        let mut x: u64 = 0x1234_5678;
        for _ in 0..5000 {
            let len = (x % 300) as usize;
            let buf: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    b"{}[]\":,0123456789tseqpts.-eE ntrufalse"[(x % 38) as usize]
                })
                .collect();
            let _ = validate(&buf);
        }
    }

    #[test]
    fn rate_limiter_blocks_floods_and_recovers() {
        let mut r = RateLimiter::new(100.0, 10.0, 0);
        assert_eq!((0..50).filter(|_| r.allow(0)).count(), 10);
        assert!(!r.allow(0));
        assert!(r.allow(100), "tokens refill over time");
    }
}
