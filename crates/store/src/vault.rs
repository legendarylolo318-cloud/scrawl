//! Optional per-notebook encryption with `age` (scrypt passphrase). The passphrase is never
//! logged, stored or included in Debug output.

use age::secrecy::SecretString;
use std::fmt;

#[derive(Clone, Default)]
pub struct Vault(Option<SecretString>);

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() { "Vault(<redacted>)" } else { "Vault(none)" })
    }
}

#[derive(Debug)]
pub enum VaultError {
    Locked,
    WrongPassword,
    Io(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => write!(f, "notebook is encrypted: password required"),
            Self::WrongPassword => write!(f, "wrong password or corrupted data"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for VaultError {}

impl Vault {
    pub fn none() -> Self {
        Self(None)
    }
    pub fn with_password(p: &str) -> Self {
        Self(Some(SecretString::from(p.to_owned())))
    }
    pub fn is_encrypted(&self) -> bool {
        self.0.is_some()
    }

    pub fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
        match &self.0 {
            None => Ok(plain.to_vec()),
            Some(p) => age::encrypt(&age::scrypt::Recipient::new(p.clone()), plain).map_err(|e| VaultError::Io(e.to_string())),
        }
    }

    pub fn open(&self, data: &[u8]) -> Result<Vec<u8>, VaultError> {
        match &self.0 {
            None => Ok(data.to_vec()),
            Some(p) => age::decrypt(&age::scrypt::Identity::new(p.clone()), data).map_err(|_| VaultError::WrongPassword),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_wrong_password_and_redaction() {
        let v = Vault::with_password("correct horse");
        let sealed = v.seal(b"integral of x dx").unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"integral"), "ciphertext must not contain plaintext");
        assert_eq!(v.open(&sealed).unwrap(), b"integral of x dx");
        assert!(matches!(Vault::with_password("wrong").open(&sealed), Err(VaultError::WrongPassword)));
        assert!(!format!("{v:?}").contains("horse"));
        assert_eq!(Vault::none().seal(b"x").unwrap(), b"x");
    }
}
