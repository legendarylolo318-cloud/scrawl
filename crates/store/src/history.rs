//! Per-page version history: throttled snapshots while editing, pruned over time.

use crate::fsio::*;
use crate::model::Page;
use crate::store::Store;
use crate::vault::Vault;
use std::path::PathBuf;

pub const SNAPSHOT_INTERVAL_MS: u64 = 5 * 60_000;
const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;

/// Which snapshot timestamps to keep: everything under an hour old, one per hour for a day,
/// one per day for 30 days; the rest are dropped.
pub fn prune_keep(stamps: &[u64], now: u64) -> Vec<u64> {
    let mut sorted = stamps.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a)); // newest first
    let mut keep = Vec::new();
    let mut seen_buckets = std::collections::HashSet::new();
    for t in sorted {
        let age = now.saturating_sub(t);
        let bucket = if age < HOUR {
            Some((0u8, t))
        } else if age < DAY {
            Some((1, t / HOUR))
        } else if age < 30 * DAY {
            Some((2, t / DAY))
        } else {
            None
        };
        if let Some(b) = bucket {
            if seen_buckets.insert(b) {
                keep.push(t);
            }
        }
    }
    keep.sort_unstable();
    keep
}

impl Store {
    fn hist_dir(&self, nb: &str, page: &str) -> PathBuf {
        self.nb_dir(nb).join("history").join(page)
    }

    /// Timeline of snapshot times (ms), oldest first.
    pub fn history(&self, nb: &str, page: &str, vault: &Vault) -> Vec<u64> {
        let mut v: Vec<u64> = std::fs::read_dir(self.hist_dir(nb, page))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let stem = if vault.is_encrypted() { n.strip_suffix(".json.age")? } else { n.strip_suffix(".json")? };
                stem.parse().ok()
            })
            .collect();
        v.sort_unstable();
        v
    }

    /// Snapshot the page *as currently on disk* if the last snapshot is old enough.
    pub(crate) fn maybe_snapshot(&self, nb: &str, page: &str, vault: &Vault, now: u64) {
        let Ok(current) = self.load_page(nb, page, vault) else { return };
        let hist = self.history(nb, page, vault);
        if hist.last().is_some_and(|&t| now.saturating_sub(t) < SNAPSHOT_INTERVAL_MS) {
            return;
        }
        let _ = self.snapshot_now(nb, &current, vault, now);
        let keep = prune_keep(&self.history(nb, page, vault), now);
        for t in self.history(nb, page, vault) {
            if !keep.contains(&t) {
                std::fs::remove_file(vpath(&self.hist_dir(nb, page).join(format!("{t}.json")), vault)).ok();
            }
        }
    }

    pub fn snapshot_now(&self, nb: &str, page: &Page, vault: &Vault, now: u64) -> crate::fsio::Result<()> {
        write_json(&self.hist_dir(nb, &page.id).join(format!("{now}.json")), page, vault)
    }

    /// Preview / restore: load the page as it was at snapshot `t`.
    pub fn load_snapshot(&self, nb: &str, page: &str, t: u64, vault: &Vault) -> crate::fsio::Result<Page> {
        read_json(&self.hist_dir(nb, page).join(format!("{t}.json")), vault)
    }

    /// Restore snapshot `t` as the current page (the current state is snapshotted first, so a
    /// restore can itself be undone).
    pub fn restore_snapshot(&self, nb: &str, page: &str, t: u64, vault: &Vault, now: u64) -> crate::fsio::Result<Page> {
        let mut old = self.load_snapshot(nb, page, t, vault)?;
        if let Ok(cur) = self.load_page(nb, page, vault) {
            self.snapshot_now(nb, &cur, vault, now)?;
        }
        self.save_page(nb, &mut old, vault, now + 1)?;
        Ok(old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn pruning_policy() {
        let now = 100 * DAY + 30 * 60_000; // mid-hour so the "same hour" samples share a bucket
        let mut stamps = vec![now - 10 * 60_000, now - 20 * 60_000]; // recent: both kept
        stamps.extend([now - 3 * HOUR, now - 3 * HOUR - 60_000, now - 3 * HOUR - 120_000]); // same hour: one kept
        stamps.extend([now - 3 * DAY, now - 3 * DAY - 5 * 60_000]); // same day: one kept
        stamps.push(now - 40 * DAY); // too old
        let keep = prune_keep(&stamps, now);
        assert_eq!(keep.len(), 2 + 1 + 1, "{keep:?}");
        assert!(!keep.contains(&(now - 40 * DAY)));
    }

    #[test]
    fn snapshots_throttle_and_restore() {
        let d = std::env::temp_dir().join(format!("iw-hist-{}", new_id()));
        let s = Store::open(&d).unwrap();
        let v = Vault::none();
        let mut nb = s.create_notebook("N", &v).unwrap();
        nb.add_section("s", "S");
        let mut p = Page::new("p", "v1", "d");
        s.create_page(&mut nb, "s", p.clone(), &v).unwrap();
        let t0 = 10 * DAY;
        p.title = "v2".into();
        s.save_page(&nb.id, &mut p, &v, t0).unwrap(); // snapshots v1
        p.title = "v3".into();
        s.save_page(&nb.id, &mut p, &v, t0 + 60_000).unwrap(); // within 5 min: no new snapshot
        assert_eq!(s.history(&nb.id, "p", &v), vec![t0]);
        p.title = "v4".into();
        s.save_page(&nb.id, &mut p, &v, t0 + 10 * 60_000).unwrap(); // snapshots v3
        assert_eq!(s.history(&nb.id, "p", &v).len(), 2);
        assert_eq!(s.load_snapshot(&nb.id, "p", t0, &v).unwrap().title, "v1");
        let restored = s.restore_snapshot(&nb.id, "p", t0, &v, t0 + 20 * 60_000).unwrap();
        assert_eq!(restored.title, "v1");
        assert_eq!(s.load_page(&nb.id, "p", &v).unwrap().title, "v1");
        assert!(s.history(&nb.id, "p", &v).len() >= 3, "restore snapshots the replaced state");
        std::fs::remove_dir_all(d).ok();
    }
}
