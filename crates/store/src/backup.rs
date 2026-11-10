//! Automatic local backups: one zip per day, keep the last N.

use crate::fsio::*;
use std::io::Write;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, base: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p == skip || p.file_name().is_some_and(|n| n.to_string_lossy().contains(".tmp")) {
            continue;
        }
        if p.is_dir() {
            collect(&p, base, skip, out);
        } else {
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

pub fn backup_name(now: u64) -> String {
    let secs = now / 1000;
    format!("scrawl-backup-{}-{:02}{:02}{:02}.zip", iso_date(now).replace('-', ""), secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

/// Zip the whole notes folder into `dest`. Backups inside the notes folder are excluded.
pub fn backup_now(root: &Path, dest: &Path, now: u64) -> Result<PathBuf> {
    std::fs::create_dir_all(dest)?;
    let mut files = Vec::new();
    collect(root, root, dest, &mut files);
    let target = dest.join(backup_name(now));
    let tmp = target.with_extension("zip.tmp");
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for rel in files {
            z.start_file(rel.to_string_lossy().replace('\\', "/"), opts).map_err(|e| StoreError::Io(std::io::Error::other(e.to_string())))?;
            z.write_all(&std::fs::read(root.join(&rel))?)?;
        }
        z.finish().map_err(|e| StoreError::Io(std::io::Error::other(e.to_string())))?;
    }
    std::fs::rename(tmp, &target)?;
    Ok(target)
}

pub fn list_backups(dest: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dest)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("scrawl-backup-") && n.to_string_lossy().ends_with(".zip")))
        .collect();
    v.sort();
    v
}

/// Make today's backup if there is none yet; prune to the newest `keep`. Returns the new file.
pub fn backup_if_due(root: &Path, dest: &Path, now: u64, keep: usize) -> Result<Option<PathBuf>> {
    let today = format!("scrawl-backup-{}", iso_date(now).replace('-', ""));
    let have_today = list_backups(dest).iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&today)));
    let made = if have_today { None } else { Some(backup_now(root, dest, now)?) };
    let all = list_backups(dest);
    for old in all.iter().take(all.len().saturating_sub(keep.max(1))) {
        std::fs::remove_file(old).ok();
    }
    Ok(made)
}

/// Restore a backup zip over `root`. The current state is backed up to `safety_dest` first, so a
/// restore can itself be undone. Entries that would escape `root` are refused (zip-slip).
pub fn restore_backup(zip_path: &Path, root: &Path, safety_dest: &Path, now: u64) -> Result<PathBuf> {
    let zerr = |e: zip::result::ZipError| StoreError::Io(std::io::Error::other(e.to_string()));
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path)?).map_err(zerr)?;
    // validate everything before touching the notes folder
    for i in 0..z.len() {
        let e = z.by_index(i).map_err(zerr)?;
        if e.enclosed_name().is_none() {
            return Err(StoreError::Io(std::io::Error::other(format!("unsafe path in backup: {:?}", e.name()))));
        }
    }
    let safety = backup_now(root, safety_dest, now)?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(zerr)?;
        let Some(rel) = e.enclosed_name() else { continue };
        if e.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut e, &mut data)?;
        atomic_write(&root.join(rel), &data)?;
    }
    Ok(safety)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_zip_keeps_last_n_and_contains_files() {
        let d = std::env::temp_dir().join(format!("iw-bk-{}", new_id()));
        let root = d.join("notes");
        let dest = d.join("backups");
        atomic_write(&root.join("nb-1/index.json"), b"{}").unwrap();
        let day = 86_400_000u64;
        let t0 = 20_000 * day;
        assert!(backup_if_due(&root, &dest, t0, 2).unwrap().is_some());
        assert!(backup_if_due(&root, &dest, t0 + 3_600_000, 2).unwrap().is_none(), "one per day");
        backup_if_due(&root, &dest, t0 + day, 2).unwrap();
        backup_if_due(&root, &dest, t0 + 2 * day, 2).unwrap();
        let all = list_backups(&dest);
        assert_eq!(all.len(), 2, "{all:?}");
        let z = zip::ZipArchive::new(std::fs::File::open(&all[1]).unwrap()).unwrap();
        assert!(z.file_names().any(|n| n.as_deref().ok() == Some("nb-1/index.json")));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn restore_brings_old_state_back_and_keeps_a_safety_copy() {
        let d = std::env::temp_dir().join(format!("iw-bk-{}", new_id()));
        let (root, dest) = (d.join("notes"), d.join("backups"));
        atomic_write(&root.join("nb-1/index.json"), b"old").unwrap();
        let b = backup_now(&root, &dest, 1_000_000).unwrap();
        atomic_write(&root.join("nb-1/index.json"), b"new").unwrap();
        let safety = restore_backup(&b, &root, &dest, 2_000_000).unwrap();
        assert_eq!(std::fs::read(root.join("nb-1/index.json")).unwrap(), b"old");
        assert!(safety.exists());
        // zip-slip entries are refused before anything is written
        let evil = d.join("evil.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
            z.start_file("../outside.txt", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        assert!(restore_backup(&evil, &root, &dest, 3_000_000).is_err());
        assert!(!d.join("outside.txt").exists());
        std::fs::remove_dir_all(d).ok();
    }
}
