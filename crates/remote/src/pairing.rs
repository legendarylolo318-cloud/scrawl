//! Pairing and device sessions.
//!
//! * A pairing token is 128 random bits, single use, and expires after 2 minutes.
//! * After the device submits it, both screens show the same 6-digit code; the user must press
//!   Approve on the PC.
//! * Approved devices get a random 256-bit session secret (only its SHA-256 is stored here).
//!   The secret is rotated on every successful connection and devices unused for 30 days expire.
//! * 5 failed pairing attempts lock pairing for 5 minutes.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TOKEN_TTL_MS: u64 = 2 * 60_000;
pub const MAX_FAILURES: u32 = 5;
pub const LOCKOUT_MS: u64 = 5 * 60_000;
pub const DEVICE_EXPIRY_MS: u64 = 30 * 86_400_000;
const MAX_PENDING: usize = 4;

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("OS random source");
    hex(&b)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha_hex(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

/// Constant-time equality for secrets.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 6-digit confirmation code both sides can compute from the token and the client's nonce.
pub fn confirmation_code(token_hex: &str, client_nonce: &str) -> String {
    let h = Sha256::digest(format!("scrawl-pair|{token_hex}|{client_nonce}").as_bytes());
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{n:06}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairError {
    Locked { until_ms: u64 },
    Expired,
    Invalid,
    TooManyPending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthError {
    Unknown,
    BadSecret,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub secret_hash: String,
    pub created_ms: u64,
    pub last_seen_ms: u64,
}

/// What the UI lists in "Paired devices".
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub last_seen_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub name: String,
    pub code: String,
}

struct ActiveToken {
    token: String,
    created_ms: u64,
}

#[derive(Default)]
pub struct Pairing {
    token: Option<ActiveToken>,
    failures: u32,
    locked_until_ms: u64,
    pending: Vec<Pending>,
    devices: Vec<Device>,
}

impl Pairing {
    pub fn with_devices(devices: Vec<Device>) -> Self {
        Self { devices, ..Default::default() }
    }

    /// Generate a fresh token for the QR code; replaces any previous one.
    pub fn begin(&mut self, now: u64) -> String {
        let token = random_hex(16);
        self.token = Some(ActiveToken { token: token.clone(), created_ms: now });
        token
    }

    pub fn is_locked(&self, now: u64) -> bool {
        now < self.locked_until_ms
    }

    fn fail(&mut self, now: u64, e: PairError) -> PairError {
        self.failures += 1;
        if self.failures >= MAX_FAILURES {
            self.locked_until_ms = now + LOCKOUT_MS;
            self.failures = 0;
        }
        e
    }

    /// The device presented a token. Consumes it (single use) and creates a pending approval.
    pub fn submit(&mut self, now: u64, token: &str, client_nonce: &str, name: &str) -> Result<Pending, PairError> {
        if self.is_locked(now) {
            return Err(PairError::Locked { until_ms: self.locked_until_ms });
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(PairError::TooManyPending);
        }
        let Some(active) = self.token.take() else {
            return Err(self.fail(now, PairError::Invalid));
        };
        if now.saturating_sub(active.created_ms) > TOKEN_TTL_MS {
            return Err(self.fail(now, PairError::Expired));
        }
        if !ct_eq(active.token.as_bytes(), token.as_bytes()) {
            // A wrong guess burns the token as well, so it cannot be brute-forced.
            return Err(self.fail(now, PairError::Invalid));
        }
        let p = Pending { id: random_hex(8), name: name.to_string(), code: confirmation_code(&active.token, client_nonce) };
        self.pending.push(p.clone());
        self.failures = 0;
        Ok(p)
    }

    pub fn pending(&self) -> &[Pending] {
        &self.pending
    }

    pub fn reject(&mut self, pending_id: &str) {
        self.pending.retain(|p| p.id != pending_id);
    }

    /// The user pressed Approve on the PC. Returns (device id, secret to hand to the device).
    pub fn approve(&mut self, now: u64, pending_id: &str) -> Option<(String, String)> {
        let i = self.pending.iter().position(|p| p.id == pending_id)?;
        let p = self.pending.remove(i);
        let secret = random_hex(32);
        let dev = Device { id: random_hex(16), name: p.name, secret_hash: sha_hex(&secret), created_ms: now, last_seen_ms: now };
        let id = dev.id.clone();
        self.devices.push(dev);
        Some((id, secret))
    }

    /// Validate a reconnecting device. On success the secret is rotated and returned.
    pub fn authenticate(&mut self, now: u64, device: &str, secret: &str) -> Result<String, AuthError> {
        let Some(d) = self.devices.iter_mut().find(|d| ct_eq(d.id.as_bytes(), device.as_bytes())) else {
            return Err(AuthError::Unknown);
        };
        if now.saturating_sub(d.last_seen_ms) > DEVICE_EXPIRY_MS {
            let id = d.id.clone();
            self.devices.retain(|d| d.id != id);
            return Err(AuthError::Expired);
        }
        if !ct_eq(d.secret_hash.as_bytes(), sha_hex(secret).as_bytes()) {
            return Err(AuthError::BadSecret);
        }
        let new_secret = random_hex(32);
        d.secret_hash = sha_hex(&new_secret);
        d.last_seen_ms = now;
        Ok(new_secret)
    }

    pub fn revoke(&mut self, device_id: &str) -> bool {
        let n = self.devices.len();
        self.devices.retain(|d| d.id != device_id);
        self.devices.len() != n
    }

    pub fn revoke_all(&mut self) {
        self.devices.clear();
        self.pending.clear();
        self.token = None;
    }

    /// Drop devices unused for 30 days (call periodically).
    pub fn expire_old(&mut self, now: u64) -> usize {
        let n = self.devices.len();
        self.devices.retain(|d| now.saturating_sub(d.last_seen_ms) <= DEVICE_EXPIRY_MS);
        n - self.devices.len()
    }

    pub fn list(&self) -> Vec<DeviceInfo> {
        self.devices.iter().map(|d| DeviceInfo { id: d.id.clone(), name: d.name.clone(), last_seen_ms: d.last_seen_ms }).collect()
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000;

    fn paired() -> (Pairing, String, String) {
        let mut p = Pairing::default();
        let tok = p.begin(T0);
        let pend = p.submit(T0 + 1000, &tok, "nonce1", "iPad").unwrap();
        let (id, secret) = p.approve(T0 + 2000, &pend.id).unwrap();
        (p, id, secret)
    }

    #[test]
    fn token_is_128_bit_single_use_and_expires() {
        let mut p = Pairing::default();
        let tok = p.begin(T0);
        assert_eq!(tok.len(), 32);
        assert!(p.submit(T0 + 1000, &tok, "n", "x").is_ok());
        assert_eq!(p.submit(T0 + 2000, &tok, "n", "x"), Err(PairError::Invalid), "single use");
        let tok = p.begin(T0);
        assert_eq!(p.submit(T0 + TOKEN_TTL_MS + 1, &tok, "n", "x"), Err(PairError::Expired));
        let tok = p.begin(T0);
        assert!(p.submit(T0 + TOKEN_TTL_MS, &tok, "n", "x").is_ok(), "valid right up to 2 minutes");
    }

    #[test]
    fn five_failures_lock_pairing_for_five_minutes() {
        let mut p = Pairing::default();
        for i in 0..5 {
            p.begin(T0);
            assert_eq!(p.submit(T0 + i, &"0".repeat(32), "n", "x"), Err(PairError::Invalid));
        }
        let real = p.begin(T0);
        assert!(matches!(p.submit(T0 + 10, &real, "n", "x"), Err(PairError::Locked { .. })), "even the right token is refused while locked");
        assert!(p.is_locked(T0 + LOCKOUT_MS - 1));
        let real = p.begin(T0 + LOCKOUT_MS + 10);
        assert!(p.submit(T0 + LOCKOUT_MS + 11, &real, "n", "x").is_ok());
    }

    #[test]
    fn wrong_guess_burns_the_token() {
        let mut p = Pairing::default();
        let tok = p.begin(T0);
        assert!(p.submit(T0, &"f".repeat(32), "n", "x").is_err());
        assert!(p.submit(T0, &tok, "n", "x").is_err(), "token cannot be brute-forced by retrying");
    }

    #[test]
    fn both_sides_compute_the_same_six_digit_code() {
        let a = confirmation_code("ab".repeat(16).as_str(), "nonce");
        assert_eq!(a.len(), 6);
        assert!(a.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(a, confirmation_code(&"ab".repeat(16), "nonce"));
        assert_ne!(a, confirmation_code(&"ab".repeat(16), "other"));
    }

    #[test]
    fn approval_required_and_secret_rotates_each_connection() {
        let mut p = Pairing::default();
        let tok = p.begin(T0);
        let pend = p.submit(T0, &tok, "n", "Pixel").unwrap();
        assert!(p.list().is_empty(), "nothing is trusted before Approve");
        assert!(p.approve(T0, "nonexistent").is_none());
        let (id, s1) = p.approve(T0 + 1, &pend.id).unwrap();
        let s2 = p.authenticate(T0 + 10, &id, &s1).unwrap();
        assert_ne!(s1, s2);
        assert_eq!(p.authenticate(T0 + 20, &id, &s1), Err(AuthError::BadSecret), "old secret is dead after rotation");
        assert!(p.authenticate(T0 + 30, &id, &s2).is_ok());
        assert_eq!(p.authenticate(T0, "deadbeef", &s2), Err(AuthError::Unknown));
    }

    #[test]
    fn revoke_and_30_day_expiry() {
        let (mut p, id, secret) = paired();
        assert_eq!(p.list()[0].name, "iPad");
        assert!(p.revoke(&id));
        assert_eq!(p.authenticate(T0 + 5000, &id, &secret), Err(AuthError::Unknown));
        let (mut p, id, secret) = paired();
        assert_eq!(p.authenticate(T0 + DEVICE_EXPIRY_MS + 10_000, &id, &secret), Err(AuthError::Expired));
        assert!(p.list().is_empty());
        let (mut p, ..) = paired();
        assert_eq!(p.expire_old(T0 + DEVICE_EXPIRY_MS + 10_000), 1);
    }

    #[test]
    fn secrets_are_stored_hashed_only() {
        let (p, _, secret) = paired();
        let json = serde_json::to_string(p.devices()).unwrap();
        assert!(!json.contains(&secret));
    }

    #[test]
    fn ct_eq_basic() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
