use crate::vault::{Vault, VaultError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Unique id, no external RNG needed: time + pid + counter.
pub fn new_id() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}{:02x}", t, std::process::id(), n & 0xff)
}

/// Write via temp file + rename so a crash never leaves a half-written file.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Vault(VaultError),
    NotFound(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Json(e) => write!(f, "format: {e}"),
            Self::Vault(e) => write!(f, "{e}"),
            Self::NotFound(w) => write!(f, "not found: {w}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<VaultError> for StoreError {
    fn from(e: VaultError) -> Self {
        Self::Vault(e)
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;

/// File path with `.age` appended when the vault is active.
pub fn vpath(base: &Path, vault: &Vault) -> PathBuf {
    if vault.is_encrypted() {
        let mut s = base.as_os_str().to_owned();
        s.push(".age");
        PathBuf::from(s)
    } else {
        base.to_path_buf()
    }
}

pub fn write_json<T: serde::Serialize>(base: &Path, v: &T, vault: &Vault) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(v)?;
    atomic_write(&vpath(base, vault), &vault.seal(&bytes)?)?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(base: &Path, vault: &Vault) -> Result<T> {
    let p = vpath(base, vault);
    let data = std::fs::read(&p).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { StoreError::NotFound(p.display().to_string()) } else { e.into() })?;
    Ok(serde_json::from_slice(&vault.open(&data)?)?)
}

/// (mtime in ns, length): cheap change detector for files edited outside the app (Syncthing, Git...).
pub fn fingerprint(path: &Path) -> Option<(u128, u64)> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    Some((t, m.len()))
}

/// Days since epoch -> (year, month, day), proleptic Gregorian.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn iso_date(ms: u64) -> String {
    let (y, m, d) = civil_from_days((ms / 86_400_000) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_ids() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(1_783_641_600_000), "2026-07-10");
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn atomic_write_replaces_without_leftovers() {
        let d = std::env::temp_dir().join(format!("iw-atomic-{}", new_id()));
        let f = d.join("a.json");
        atomic_write(&f, b"one").unwrap();
        atomic_write(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no temp files left");
        std::fs::remove_dir_all(d).ok();
    }
}
