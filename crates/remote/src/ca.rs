//! Private certificate authority for the tablet server.
//!
//! * The root CA carries X.509 Name Constraints limiting it to private IP ranges (10/8, 172.16/12,
//!   192.168/16, 100.64/10 for Tailscale) and `.local` names, so even a leaked CA key cannot be used
//!   to impersonate a real website.
//! * The CA private key lives in the OS keyring (Secret Service / Windows Credential Manager), never
//!   as a plaintext file. Only the public certificate is stored on disk.
//! * Leaf certificates are valid for 30 days and are re-issued when they near expiry or when the
//!   machine's LAN IPs change.

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

pub const LEAF_DAYS: i64 = 30;
pub const RENEW_BEFORE_DAYS: i64 = 7;
const CA_YEARS: i32 = 10;

/// Where the CA private key (PEM) is kept.
pub trait KeyStore {
    fn get(&self) -> Option<String>;
    fn set(&self, pem: &str) -> Result<(), String>;
    fn delete(&self);
}

/// OS keyring: Secret Service on Linux, Credential Manager (DPAPI) on Windows.
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("inkwell-tablet-server", "root-ca-key").map_err(|e| e.to_string())
    }
}

impl KeyStore for KeyringStore {
    fn get(&self) -> Option<String> {
        Self::entry().ok()?.get_password().ok()
    }
    fn set(&self, pem: &str) -> Result<(), String> {
        Self::entry()?.set_password(pem).map_err(|e| format!("could not store the CA key in the OS keyring: {e}"))
    }
    fn delete(&self) {
        if let Ok(e) = Self::entry() {
            let _ = e.delete_credential();
        }
    }
}

/// In-memory store for tests.
#[derive(Default)]
pub struct MemStore(pub std::sync::Mutex<Option<String>>);

impl KeyStore for MemStore {
    fn get(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
    fn set(&self, pem: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(pem.to_string());
        Ok(())
    }
    fn delete(&self) {
        *self.0.lock().unwrap() = None;
    }
}

/// True for the ranges the server may bind to and the CA may certify.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168) || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        IpAddr::V6(_) => false,
    }
}

fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn ymd(now_ms: u64, plus_days: i64) -> time::OffsetDateTime {
    let days = (now_ms / 86_400_000) as i64 + plus_days;
    let (y, m, d) = civil_from_days(days);
    rcgen::date_time_ymd(y, m, d)
}

fn ca_params(now_ms: u64) -> Result<CertificateParams, String> {
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Inkwell local tablet CA");
    dn.push(DnType::OrganizationName, "Inkwell");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let subnet = |a: [u8; 4], m: [u8; 4]| GeneralSubtree::IpAddress(CidrSubnet::V4(a, m));
    p.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![
            subnet([10, 0, 0, 0], [255, 0, 0, 0]),
            subnet([172, 16, 0, 0], [255, 240, 0, 0]),
            subnet([192, 168, 0, 0], [255, 255, 0, 0]),
            subnet([100, 64, 0, 0], [255, 192, 0, 0]),
            GeneralSubtree::DnsName(".local".into()),
        ],
        excluded_subtrees: vec![],
    });
    p.not_before = ymd(now_ms, -1);
    p.not_after = ymd(now_ms, 365 * CA_YEARS as i64);
    Ok(p)
}

pub struct CaMaterial {
    /// Public certificate, PEM.
    pub cert_pem: String,
    /// Same certificate, DER (for the .crt download and .mobileconfig).
    pub cert_der: Vec<u8>,
}

impl CaMaterial {
    /// SHA-256 fingerprint shown on the PC so it can be compared on the device.
    pub fn fingerprint(&self) -> String {
        Sha256::digest(&self.cert_der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().chunks(2).map(|c| c.concat()).collect::<Vec<_>>().join(" ")
    }
}

/// Create the CA on first use; later calls load it. The key goes to `keys`, the cert is returned
/// (the caller stores the public cert wherever convenient).
pub fn ensure_ca(keys: &dyn KeyStore, existing_cert_pem: Option<&str>, now_ms: u64) -> Result<CaMaterial, String> {
    if let (Some(_), Some(pem)) = (keys.get(), existing_cert_pem) {
        let der = pem_to_der(pem)?;
        return Ok(CaMaterial { cert_pem: pem.to_string(), cert_der: der });
    }
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = ca_params(now_ms)?.self_signed(&key).map_err(|e| e.to_string())?;
    keys.set(&key.serialize_pem())?;
    Ok(CaMaterial { cert_pem: cert.pem(), cert_der: cert.der().to_vec() })
}

pub fn pem_to_der(pem: &str) -> Result<Vec<u8>, String> {
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())
}

pub struct Leaf {
    pub cert_pem: String,
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub issued_ms: u64,
    pub ips: Vec<IpAddr>,
}

/// Issue a 30-day server certificate for the given private IPs (and `inkwell.local`).
pub fn issue_leaf(keys: &dyn KeyStore, ca_cert_pem: &str, ips: &[IpAddr], now_ms: u64) -> Result<Leaf, String> {
    let ips: Vec<IpAddr> = ips.iter().copied().filter(|i| is_private_ip(*i)).collect();
    if ips.is_empty() {
        return Err("no private-range network interface to certify".into());
    }
    let ca_key = KeyPair::from_pem(&keys.get().ok_or("CA key not found in the OS keyring")?).map_err(|e| e.to_string())?;
    let issuer = Issuer::from_ca_cert_pem(ca_cert_pem, ca_key).map_err(|e| e.to_string())?;
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Inkwell tablet server");
    p.distinguished_name = dn;
    p.subject_alt_names = ips.iter().map(|i| SanType::IpAddress(*i)).collect();
    p.subject_alt_names.push(SanType::DnsName("inkwell.local".try_into().map_err(|e: rcgen::Error| e.to_string())?));
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.not_before = ymd(now_ms, -1);
    p.not_after = ymd(now_ms, LEAF_DAYS);
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = p.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    Ok(Leaf { cert_pem: cert.pem(), cert_der: cert.der().to_vec(), key_der: key.serialize_der(), issued_ms: now_ms, ips })
}

impl Leaf {
    /// Re-issue when close to expiry or when the set of LAN IPs changed.
    pub fn needs_renewal(&self, now_ms: u64, current_ips: &[IpAddr]) -> bool {
        let age_days = now_ms.saturating_sub(self.issued_ms) / 86_400_000;
        let mut a: Vec<IpAddr> = self.ips.clone();
        let mut b: Vec<IpAddr> = current_ips.iter().copied().filter(|i| is_private_ip(*i)).collect();
        a.sort();
        b.sort();
        age_days as i64 >= LEAF_DAYS - RENEW_BEFORE_DAYS || a != b
    }
}

/// iOS configuration profile that installs the CA as a trusted root.
pub fn mobileconfig(ca_der: &[u8], uuid_a: &str, uuid_b: &str) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(ca_der);
    let wrapped: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).to_string()).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadCertificateFileName</key><string>inkwell-ca.cer</string>
      <key>PayloadContent</key><data>
{}
      </data>
      <key>PayloadDescription</key><string>Lets this device trust the Inkwell tablet server on your home network. Limited to private IP addresses and .local names.</string>
      <key>PayloadDisplayName</key><string>Inkwell local tablet CA</string>
      <key>PayloadIdentifier</key><string>app.inkwell.ca.{uuid_b}</string>
      <key>PayloadType</key><string>com.apple.security.root</string>
      <key>PayloadUUID</key><string>{uuid_b}</string>
      <key>PayloadVersion</key><integer>1</integer>
    </dict>
  </array>
  <key>PayloadDisplayName</key><string>Inkwell tablet connection</string>
  <key>PayloadIdentifier</key><string>app.inkwell.profile.{uuid_a}</string>
  <key>PayloadRemovalDisallowed</key><false/>
  <key>PayloadType</key><string>Configuration</string>
  <key>PayloadUUID</key><string>{uuid_a}</string>
  <key>PayloadVersion</key><integer>1</integer>
</dict>
</plist>
"#,
        wrapped.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const NOW: u64 = 1_790_000_000_000; // ~Sept 2026

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn private_range_check() {
        for ok in [ip(10, 1, 2, 3), ip(172, 16, 0, 1), ip(172, 31, 255, 1), ip(192, 168, 1, 5), ip(100, 64, 0, 1), ip(100, 127, 1, 1)] {
            assert!(is_private_ip(ok), "{ok}");
        }
        for bad in [ip(8, 8, 8, 8), ip(172, 32, 0, 1), ip(100, 128, 0, 1), ip(192, 169, 1, 1), ip(127, 0, 0, 1), ip(11, 0, 0, 1)] {
            assert!(!is_private_ip(bad), "{bad}");
        }
    }

    #[test]
    fn ca_key_goes_to_keystore_and_cert_has_name_constraints() {
        let keys = MemStore::default();
        let ca = ensure_ca(&keys, None, NOW).unwrap();
        assert!(keys.get().unwrap().contains("PRIVATE KEY"), "key stored in the keystore");
        assert!(!ca.cert_pem.contains("PRIVATE KEY"), "cert output never contains the key");
        let (_, cert) = x509_parser::parse_x509_certificate(&ca.cert_der).unwrap();
        let nc = cert.tbs_certificate.extensions().iter().find_map(|e| match e.parsed_extension() {
            x509_parser::extensions::ParsedExtension::NameConstraints(n) => Some(n.clone()),
            _ => None,
        });
        let nc = nc.expect("CA must have Name Constraints");
        let permitted = nc.permitted_subtrees.expect("permitted subtrees");
        assert_eq!(permitted.len(), 5, "4 private ranges + .local");
        assert!(cert.is_ca());
        // reloading does not mint a new CA
        let again = ensure_ca(&keys, Some(&ca.cert_pem), NOW + 1000).unwrap();
        assert_eq!(again.cert_der, ca.cert_der);
        assert_eq!(ca.fingerprint().split(' ').count(), 16);
    }

    #[test]
    fn leaf_for_private_ip_validates_but_ca_cannot_vouch_for_real_websites() {
        let keys = MemStore::default();
        let ca = ensure_ca(&keys, None, NOW).unwrap();
        let good = issue_leaf(&keys, &ca.cert_pem, &[ip(192, 168, 1, 50)], NOW).unwrap();
        // Chain verification with name constraints enforced by webpki.
        let verify = |leaf_der: &[u8], name: &str| -> bool {
            use webpki::{anchor_from_trusted_cert, EndEntityCert, KeyUsage};
            let anchor_der = rustls_pki_types::CertificateDer::from(ca.cert_der.clone());
            let anchor = anchor_from_trusted_cert(&anchor_der).unwrap();
            let leaf_der = rustls_pki_types::CertificateDer::from(leaf_der.to_vec());
            let cert = EndEntityCert::try_from(&leaf_der).unwrap();
            let now = rustls_pki_types::UnixTime::since_unix_epoch(std::time::Duration::from_millis(NOW + 3_600_000));
            let ok = cert
                .verify_for_usage(webpki::ALL_VERIFICATION_ALGS, &[anchor], &[], now, KeyUsage::server_auth(), None, None)
                .is_ok();
            let server_name = rustls_pki_types::ServerName::try_from(name.to_string()).unwrap();
            ok && cert.verify_is_valid_for_subject_name(&server_name).is_ok()
        };
        assert!(verify(&good.cert_der, "192.168.1.50"));
        assert!(!verify(&good.cert_der, "192.168.1.51"), "wrong IP is rejected");
        // Public IPs are refused outright by issue_leaf ...
        assert!(issue_leaf(&keys, &ca.cert_pem, &[ip(8, 8, 8, 8)], NOW).is_err());
        // ... and even a leaf minted directly for a real website with the CA key fails verification.
        let evil = {
            let ca_key = KeyPair::from_pem(&keys.get().unwrap()).unwrap();
            let issuer = Issuer::from_ca_cert_pem(&ca.cert_pem, ca_key).unwrap();
            let mut p = CertificateParams::new(vec!["example.com".to_string()]).unwrap();
            p.not_before = ymd(NOW, -1);
            p.not_after = ymd(NOW, 30);
            let k = KeyPair::generate().unwrap();
            p.signed_by(&k, &issuer).unwrap().der().to_vec()
        };
        assert!(!verify(&evil, "example.com"), "name constraints stop impersonation of real sites");
    }

    #[test]
    fn leaf_renewal_rules() {
        let keys = MemStore::default();
        let ca = ensure_ca(&keys, None, NOW).unwrap();
        let ips = [ip(192, 168, 1, 50)];
        let leaf = issue_leaf(&keys, &ca.cert_pem, &ips, NOW).unwrap();
        let day = 86_400_000;
        assert!(!leaf.needs_renewal(NOW + 10 * day, &ips));
        assert!(leaf.needs_renewal(NOW + 24 * day, &ips), "renew in the last week");
        assert!(leaf.needs_renewal(NOW + day, &[ip(192, 168, 1, 77)]), "IP changed");
        assert!(!leaf.needs_renewal(NOW + day, &[ip(192, 168, 1, 50), ip(8, 8, 8, 8)]), "public IPs are ignored");
    }

    #[test]
    fn mobileconfig_embeds_the_ca() {
        let keys = MemStore::default();
        let ca = ensure_ca(&keys, None, NOW).unwrap();
        let m = mobileconfig(&ca.cert_der, "AAAA", "BBBB");
        assert!(m.contains("com.apple.security.root") && m.contains("<data>"));
    }
}
