use crate::fsio::*;
use crate::model::*;
use crate::templates;
use crate::vault::Vault;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const TRASH_RETENTION_MS: u64 = 30 * 86_400_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NotebookMeta {
    pub id: String,
    pub encrypted: bool,
    /// None for encrypted notebooks (the name lives in the encrypted index).
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrashEntry {
    pub page_id: String,
    pub title: String,
    pub section_id: String,
    pub deleted_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self { root: root.to_path_buf() })
    }

    pub(crate) fn nb_dir(&self, nb: &str) -> PathBuf {
        self.root.join(format!("nb-{nb}"))
    }
    pub(crate) fn page_base(&self, nb: &str, page: &str) -> PathBuf {
        self.nb_dir(nb).join("pages").join(format!("{page}.json"))
    }
    fn index_base(&self, nb: &str) -> PathBuf {
        self.nb_dir(nb).join("index.json")
    }
    fn trash_index_base(&self, nb: &str) -> PathBuf {
        self.nb_dir(nb).join("trash.json")
    }

    // ---- notebooks -------------------------------------------------------------------

    pub fn create_notebook(&self, name: &str, vault: &Vault) -> Result<Notebook> {
        let nb = Notebook { id: new_id(), name: name.into(), sections: vec![] };
        let meta = NotebookMeta { id: nb.id.clone(), encrypted: vault.is_encrypted(), display_name: (!vault.is_encrypted()).then(|| name.to_string()) };
        atomic_write(&self.nb_dir(&nb.id).join("meta.json"), &serde_json::to_vec_pretty(&meta)?)?;
        self.save_notebook(&nb, vault)?;
        Ok(nb)
    }

    pub fn list_notebooks(&self) -> Vec<NotebookMeta> {
        let mut v: Vec<NotebookMeta> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| std::fs::read(e.path().join("meta.json")).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        v.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.id.cmp(&b.id)));
        v
    }

    pub fn load_notebook(&self, id: &str, vault: &Vault) -> Result<Notebook> {
        read_json(&self.index_base(id), vault)
    }

    pub fn save_notebook(&self, nb: &Notebook, vault: &Vault) -> Result<()> {
        write_json(&self.index_base(&nb.id), nb, vault)
    }

    // ---- pages -----------------------------------------------------------------------

    pub fn create_page(&self, nb: &mut Notebook, section: &str, mut page: Page, vault: &Vault) -> Result<()> {
        if !nb.add_page(section, PageRef { id: page.id.clone(), title: page.title.clone() }) {
            return Err(StoreError::NotFound(format!("section {section}")));
        }
        self.save_page(&nb.id, &mut page, vault, now_ms())?;
        self.save_notebook(nb, vault)
    }

    /// New page from a built-in or user template, with today's date auto-filled.
    pub fn page_from_template(&self, template: &str, now: u64) -> Option<Page> {
        let date = iso_date(now);
        templates::builtin(template, &new_id(), &date, 5).or_else(|| {
            let mut p = self.load_template(template).ok()?;
            p.id = new_id();
            p.date = date;
            p.strokes.clear();
            Some(p)
        })
    }

    pub fn save_page(&self, nb: &str, page: &mut Page, vault: &Vault, now: u64) -> Result<()> {
        page.modified_ms = now;
        let base = self.page_base(nb, &page.id);
        // history snapshot of what is about to be replaced (throttled)
        self.maybe_snapshot(nb, &page.id, vault, now);
        write_json(&base, page, vault)?;
        atomic_write(&vpath(&base.with_extension("typ"), vault), &vault.seal(page.to_typst().as_bytes())?)?;
        self.clear_recovery(nb, &page.id);
        Ok(())
    }

    pub fn load_page(&self, nb: &str, page: &str, vault: &Vault) -> Result<Page> {
        read_json(&self.page_base(nb, page), vault)
    }

    pub fn page_fingerprint(&self, nb: &str, page: &str, vault: &Vault) -> Option<(u128, u64)> {
        fingerprint(&vpath(&self.page_base(nb, page), vault))
    }

    pub fn duplicate_page(&self, nb: &mut Notebook, page_id: &str, vault: &Vault) -> Result<String> {
        let (si, pi) = nb.find_page(page_id).ok_or_else(|| StoreError::NotFound(page_id.into()))?;
        let mut p = self.load_page(&nb.id, page_id, vault)?;
        p.id = new_id();
        p.title = format!("{} (copy)", p.title);
        let id = p.id.clone();
        nb.sections[si].pages.insert(pi + 1, PageRef { id: id.clone(), title: p.title.clone() });
        self.save_page(&nb.id, &mut p, vault, now_ms())?;
        self.save_notebook(nb, vault)?;
        Ok(id)
    }

    pub fn duplicate_notebook(&self, nb: &Notebook, vault: &Vault) -> Result<Notebook> {
        let mut copy = self.create_notebook(&format!("{} (copy)", nb.name), vault)?;
        copy.sections = nb.sections.clone();
        for s in &mut copy.sections {
            for r in &mut s.pages {
                let mut p = self.load_page(&nb.id, &r.id, vault)?;
                p.id = new_id();
                r.id = p.id.clone();
                self.save_page(&copy.id, &mut p, vault, now_ms())?;
            }
        }
        self.save_notebook(&copy, vault)?;
        Ok(copy)
    }

    // ---- trash -----------------------------------------------------------------------

    pub fn list_trash(&self, nb: &str, vault: &Vault) -> Vec<TrashEntry> {
        read_json(&self.trash_index_base(nb), vault).unwrap_or_default()
    }

    pub fn delete_page(&self, nb: &mut Notebook, page_id: &str, vault: &Vault, now: u64) -> Result<()> {
        let (section, r) = nb.remove_page(page_id).ok_or_else(|| StoreError::NotFound(page_id.into()))?;
        let mut trash = self.list_trash(&nb.id, vault);
        trash.push(TrashEntry { page_id: r.id.clone(), title: r.title, section_id: section, deleted_ms: now });
        write_json(&self.trash_index_base(&nb.id), &trash, vault)?;
        // move the page file into trash/ (keeps history for restore)
        let from = vpath(&self.page_base(&nb.id, page_id), vault);
        let to = self.nb_dir(&nb.id).join("trash").join(from.file_name().unwrap());
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::rename(from, to)?;
        self.save_notebook(nb, vault)
    }

    pub fn restore_page(&self, nb: &mut Notebook, page_id: &str, vault: &Vault) -> Result<()> {
        let mut trash = self.list_trash(&nb.id, vault);
        let i = trash.iter().position(|t| t.page_id == page_id).ok_or_else(|| StoreError::NotFound(page_id.into()))?;
        let entry = trash.remove(i);
        let name = vpath(&self.page_base(&nb.id, page_id), vault);
        let from = self.nb_dir(&nb.id).join("trash").join(name.file_name().unwrap());
        std::fs::create_dir_all(name.parent().unwrap())?;
        std::fs::rename(from, &name)?;
        let section = if nb.sections.iter().any(|s| s.id == entry.section_id) { entry.section_id } else { nb.sections.first().map(|s| s.id.clone()).ok_or_else(|| StoreError::NotFound("any section".into()))? };
        nb.add_page(&section, PageRef { id: entry.page_id, title: entry.title });
        write_json(&self.trash_index_base(&nb.id), &trash, vault)?;
        self.save_notebook(nb, vault)
    }

    /// Permanently remove trash entries older than 30 days. Returns how many were purged.
    pub fn purge_trash(&self, nb: &str, vault: &Vault, now: u64) -> Result<usize> {
        let trash = self.list_trash(nb, vault);
        let (old, keep): (Vec<_>, Vec<_>) = trash.into_iter().partition(|t| now.saturating_sub(t.deleted_ms) > TRASH_RETENTION_MS);
        for t in &old {
            let name = vpath(&self.page_base(nb, &t.page_id), vault);
            std::fs::remove_file(self.nb_dir(nb).join("trash").join(name.file_name().unwrap())).ok();
            std::fs::remove_dir_all(self.nb_dir(nb).join("history").join(&t.page_id)).ok();
        }
        if !old.is_empty() {
            write_json(&self.trash_index_base(nb), &keep, vault)?;
        }
        Ok(old.len())
    }

    // ---- recent pages ----------------------------------------------------------------

    pub fn touch_recent(&self, nb: &str, page: &str) -> Result<()> {
        let path = self.root.join("recent.json");
        let mut v: Vec<(String, String)> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        v.retain(|(n, p)| !(n == nb && p == page));
        v.insert(0, (nb.into(), page.into()));
        v.truncate(20);
        atomic_write(&path, &serde_json::to_vec(&v)?)?;
        Ok(())
    }

    pub fn recent(&self) -> Vec<(String, String)> {
        std::fs::read(self.root.join("recent.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    // ---- crash recovery --------------------------------------------------------------

    fn recovery_base(&self, nb: &str, page: &str) -> PathBuf {
        self.root.join("recovery").join(format!("{nb}__{page}.json"))
    }

    /// Called frequently while editing; cheap, atomic, removed on a successful save.
    pub fn write_recovery(&self, nb: &str, page: &Page, vault: &Vault) -> Result<()> {
        write_json(&self.recovery_base(nb, &page.id), page, vault)
    }

    pub fn clear_recovery(&self, nb: &str, page: &str) {
        for v in [Vault::none(), Vault::with_password("")] {
            std::fs::remove_file(vpath(&self.recovery_base(nb, page), &v)).ok();
        }
    }

    /// (notebook id, page id) pairs with unsaved recovery data from a previous crash.
    pub fn pending_recoveries(&self) -> Vec<(String, String)> {
        std::fs::read_dir(self.root.join("recovery"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let stem = name.strip_suffix(".json.age").or_else(|| name.strip_suffix(".json"))?;
                let (n, p) = stem.split_once("__")?;
                Some((n.to_string(), p.to_string()))
            })
            .collect()
    }

    pub fn load_recovery(&self, nb: &str, page: &str, vault: &Vault) -> Result<Page> {
        read_json(&self.recovery_base(nb, page), vault)
    }

    // ---- user templates --------------------------------------------------------------

    pub fn save_template(&self, name: &str, page: &Page) -> Result<()> {
        let safe: String = name.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect();
        write_json(&self.root.join("templates").join(format!("{safe}.json")), page, &Vault::none())
    }

    pub fn load_template(&self, name: &str) -> Result<Page> {
        let safe: String = name.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect();
        read_json(&self.root.join("templates").join(format!("{safe}.json")), &Vault::none())
    }

    pub fn list_templates(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root.join("templates"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (Store, PathBuf) {
        let d = std::env::temp_dir().join(format!("iw-store-{}", new_id()));
        (Store::open(&d).unwrap(), d)
    }

    fn setup(vault: &Vault) -> (Store, PathBuf, Notebook) {
        let (s, d) = tmp();
        let mut nb = s.create_notebook("Calc 1", vault).unwrap();
        nb.add_section("s1", "Limits");
        s.save_notebook(&nb, vault).unwrap();
        (s, d, nb)
    }

    #[test]
    fn page_roundtrip_with_typst_and_json_sidecar() {
        let v = Vault::none();
        let (s, d, mut nb) = setup(&v);
        let mut p = Page::new("p1", "Limits", "2026-11-09");
        p.tags = vec!["calc".into()];
        p.blocks.push(Block::Math { expr: layout::Expr::sym("x"), source: None });
        s.create_page(&mut nb, "s1", p.clone(), &v).unwrap();
        let back = s.load_page(&nb.id, "p1", &v).unwrap();
        assert_eq!(back.title, "Limits");
        assert_eq!(back.blocks, p.blocks);
        assert!(s.nb_dir(&nb.id).join("pages/p1.typ").exists(), "human-readable Typst file");
        assert!(s.nb_dir(&nb.id).join("pages/p1.json").exists());
        assert_eq!(s.load_notebook(&nb.id, &v).unwrap().sections[0].pages.len(), 1);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn encrypted_notebook_has_no_plaintext_on_disk() {
        let v = Vault::with_password("pw");
        let (s, d, mut nb) = setup(&v);
        let mut p = Page::new("p1", "Secret derivation", "2026-11-09");
        p.blocks.push(Block::Text { markup: "tangent plane".into() });
        s.create_page(&mut nb, "s1", p, &v).unwrap();
        for e in walk(&d) {
            if e.file_name().unwrap() == "meta.json" || e.extension().is_some_and(|x| x != "age") {
                continue;
            }
            let bytes = std::fs::read(&e).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("tangent"), "{e:?}");
        }
        assert!(s.load_page(&nb.id, "p1", &v).is_ok());
        assert!(s.load_page(&nb.id, "p1", &Vault::with_password("bad")).is_err());
        assert!(s.list_notebooks()[0].display_name.is_none(), "name hidden for encrypted notebooks");
        std::fs::remove_dir_all(d).ok();
    }

    fn walk(d: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(d).unwrap().flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    #[test]
    fn trash_restore_and_30_day_purge() {
        let v = Vault::none();
        let (s, d, mut nb) = setup(&v);
        s.create_page(&mut nb, "s1", Page::new("a", "A", "d"), &v).unwrap();
        s.create_page(&mut nb, "s1", Page::new("b", "B", "d"), &v).unwrap();
        let t0 = 1_000_000_000_000;
        s.delete_page(&mut nb, "a", &v, t0).unwrap();
        s.delete_page(&mut nb, "b", &v, t0 + 20 * 86_400_000).unwrap();
        assert!(nb.sections[0].pages.is_empty());
        assert_eq!(s.list_trash(&nb.id, &v).len(), 2);
        s.restore_page(&mut nb, "a", &v).unwrap();
        assert_eq!(s.load_page(&nb.id, "a", &v).unwrap().title, "A");
        // 31 days after b was deleted -> purged; a was restored so unaffected
        assert_eq!(s.purge_trash(&nb.id, &v, t0 + 51 * 86_400_000).unwrap(), 1);
        assert!(s.list_trash(&nb.id, &v).is_empty());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn duplicate_page_and_notebook() {
        let v = Vault::none();
        let (s, d, mut nb) = setup(&v);
        s.create_page(&mut nb, "s1", Page::new("a", "A", "d"), &v).unwrap();
        let id = s.duplicate_page(&mut nb, "a", &v).unwrap();
        assert_eq!(s.load_page(&nb.id, &id, &v).unwrap().title, "A (copy)");
        assert_eq!(nb.sections[0].pages.len(), 2);
        let c = s.duplicate_notebook(&nb, &v).unwrap();
        assert_eq!(c.sections[0].pages.len(), 2);
        assert_ne!(c.sections[0].pages[0].id, nb.sections[0].pages[0].id);
        assert!(s.load_page(&c.id, &c.sections[0].pages[0].id, &v).is_ok());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn crash_recovery_and_recent_and_templates() {
        let v = Vault::none();
        let (s, d, nb) = setup(&v);
        let mut p = Page::new("a", "Draft", "d");
        p.blocks.push(Block::Rule);
        s.write_recovery(&nb.id, &p, &v).unwrap();
        assert_eq!(s.pending_recoveries(), vec![(nb.id.clone(), "a".to_string())]);
        assert_eq!(s.load_recovery(&nb.id, "a", &v).unwrap().blocks.len(), 1);
        s.clear_recovery(&nb.id, "a");
        assert!(s.pending_recoveries().is_empty());
        s.touch_recent("n", "1").unwrap();
        s.touch_recent("n", "2").unwrap();
        s.touch_recent("n", "1").unwrap();
        assert_eq!(s.recent()[0].1, "1");
        assert_eq!(s.recent().len(), 2);
        s.save_template("my lab", &p).unwrap();
        assert_eq!(s.list_templates(), vec!["my lab"]);
        assert!(s.page_from_template("my lab", 0).is_some());
        assert!(s.page_from_template("cornell notes", 0).is_some());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn external_change_detected_via_fingerprint() {
        let v = Vault::none();
        let (s, d, mut nb) = setup(&v);
        s.create_page(&mut nb, "s1", Page::new("a", "A", "d"), &v).unwrap();
        let f1 = s.page_fingerprint(&nb.id, "a", &v);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut p = s.load_page(&nb.id, "a", &v).unwrap();
        p.title = "Edited elsewhere".into();
        write_json(&s.page_base(&nb.id, "a"), &p, &v).unwrap();
        assert_ne!(f1, s.page_fingerprint(&nb.id, "a", &v));
        std::fs::remove_dir_all(d).ok();
    }
}
