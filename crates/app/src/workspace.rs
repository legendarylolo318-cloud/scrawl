//! The notes workspace: notebooks (optionally encrypted), sections, pages, autosave with crash
//! recovery, trash, history, templates, media, and the search index (persisted for unencrypted
//! notebooks, memory-only for encrypted ones so no plaintext reaches the disk).

use ink::Stroke;
use search::{doc_from_page, Filters, Hit, SearchIndex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use store::{Notebook, NotebookMeta, Page, PageRef, Store, Vault};

pub struct Workspace {
    pub store: Store,
    pub nb: Notebook,
    pub vault: Vault,
    pub page: Page,
    dirty_since: Option<u64>,
    last_recovery_ms: u64,
    index: Option<SearchIndex>,
    pub recovered: Option<String>,
    /// On-disk fingerprint of the open page when we last loaded/saved it (external-change check).
    disk_fp: Option<(u128, u64)>,
}

/// One row in the sidebar tree (sections and their pages, flattened).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub section: String,
    pub section_id: String,
    pub is_current: bool,
    pub is_section: bool,
}

pub fn default_root() -> PathBuf {
    if let Some(d) = std::env::var_os("SCRAWL_NOTES_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Scrawl")
}

/// Parse `tag:x`, `after:YYYY-MM-DD`, `before:YYYY-MM-DD` out of a search query.
pub fn parse_query(q: &str) -> (String, Filters) {
    let mut f = Filters::default();
    let mut words = Vec::new();
    for w in q.split_whitespace() {
        if let Some(t) = w.strip_prefix("tag:") {
            f.tag = Some(t.to_lowercase());
        } else if let Some(d) = w.strip_prefix("after:") {
            f.date_from = Some(d.into());
        } else if let Some(d) = w.strip_prefix("before:") {
            f.date_to = Some(d.into());
        } else {
            words.push(w);
        }
    }
    (words.join(" "), f)
}

impl Workspace {
    /// Open (or create) the notes folder, load the preferred (or first unencrypted) notebook.
    pub fn open(root: &Path) -> store::Result<Self> {
        Self::open_with(root, None, None)
    }

    pub fn open_with(root: &Path, nb_pref: Option<&str>, page_pref: Option<&str>) -> store::Result<Self> {
        let store = Store::open(root)?;
        let vault = Vault::none();
        let metas = store.list_notebooks();
        let chosen = metas.iter().find(|m| Some(m.id.as_str()) == nb_pref && !m.encrypted).or_else(|| metas.iter().find(|m| !m.encrypted));
        let nb = match chosen {
            Some(m) => store.load_notebook(&m.id, &vault)?,
            None => {
                let mut nb = store.create_notebook("My notebook", &vault)?;
                nb.add_section(&store::new_id(), "Notes");
                store.save_notebook(&nb, &vault)?;
                nb
            }
        };
        let mut ws = Self { page: Page::new("", "", ""), store, nb, vault, dirty_since: None, last_recovery_ms: 0, index: None, recovered: None, disk_fp: None };
        ws.enter_notebook(page_pref)?;
        Ok(ws)
    }

    /// After switching `nb`/`vault`: make sure there is a section and a page, open one.
    fn enter_notebook(&mut self, page_pref: Option<&str>) -> store::Result<()> {
        if self.nb.sections.is_empty() {
            self.nb.add_section(&store::new_id(), "Notes");
        }
        let all: Vec<String> = self.nb.sections.iter().flat_map(|s| s.pages.iter().map(|p| p.id.clone())).collect();
        let pick = page_pref.filter(|p| all.iter().any(|a| a == p)).map(String::from).or_else(|| all.first().cloned());
        self.page = match pick.and_then(|id| self.store.load_page(&self.nb.id, &id, &self.vault).ok()) {
            Some(p) => p,
            None => {
                let first_launch = self.nb.name == "My notebook";
                let p = if first_launch { Some(welcome_page(store::now_ms())) } else { self.store.page_from_template("lecture notes", store::now_ms()) }.unwrap_or_else(|| Page::new(&store::new_id(), "Untitled", &store::iso_date(store::now_ms())));
                let sec = self.nb.sections[0].id.clone();
                self.store.create_page(&mut self.nb, &sec, p.clone(), &self.vault)?;
                p
            }
        };
        self.index = None;
        self.dirty_since = None;
        self.apply_recovery();
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    /// If the app crashed with unsaved work for the open page, restore it.
    fn apply_recovery(&mut self) {
        let want = (self.nb.id.clone(), self.page.id.clone());
        if self.store.pending_recoveries().contains(&want) {
            if let Ok(p) = self.store.load_recovery(&want.0, &want.1, &self.vault) {
                // A recovery file only exists while there are unsaved changes (every save deletes it).
                self.recovered = Some(format!("Recovered unsaved changes to \"{}\"", p.title));
                self.page = p;
                self.dirty_since = Some(store::now_ms());
            }
        }
    }

    // ---- notebooks -------------------------------------------------------------------

    pub fn notebooks(&self) -> Vec<NotebookMeta> {
        self.store.list_notebooks()
    }

    /// Display name for the notebook list (encrypted names stay hidden until unlocked).
    pub fn notebook_label(&self, m: &NotebookMeta) -> String {
        if m.id == self.nb.id {
            return format!("{}{}", if m.encrypted { "🔒 " } else { "" }, self.nb.name);
        }
        match &m.display_name {
            Some(n) => n.clone(),
            None => "🔒 Encrypted notebook".into(),
        }
    }

    /// Switch notebooks. Encrypted ones need the password; a wrong one is reported, nothing changes.
    pub fn switch_notebook(&mut self, id: &str, password: Option<&str>, now: u64) -> Result<(), String> {
        let meta = self.notebooks().into_iter().find(|m| m.id == id).ok_or("notebook not found")?;
        let vault = match (meta.encrypted, password) {
            (true, Some(p)) => Vault::with_password(p),
            (true, None) => return Err("password needed".into()),
            _ => Vault::none(),
        };
        let nb = self.store.load_notebook(id, &vault).map_err(|e| match e {
            store::StoreError::Vault(_) => "Wrong password (nothing was changed).".to_string(),
            other => other.to_string(),
        })?;
        if self.is_dirty() {
            self.save(now).map_err(|e| e.to_string())?;
        }
        self.nb = nb;
        self.vault = vault;
        self.enter_notebook(None).map_err(|e| e.to_string())
    }

    pub fn create_notebook(&mut self, name: &str, password: Option<&str>, now: u64) -> Result<(), String> {
        let vault = password.map(Vault::with_password).unwrap_or_else(Vault::none);
        let mut nb = self.store.create_notebook(name, &vault).map_err(|e| e.to_string())?;
        nb.add_section(&store::new_id(), "Notes");
        self.store.save_notebook(&nb, &vault).map_err(|e| e.to_string())?;
        if self.is_dirty() {
            self.save(now).map_err(|e| e.to_string())?;
        }
        self.nb = nb;
        self.vault = vault;
        self.enter_notebook(None).map_err(|e| e.to_string())
    }

    pub fn is_encrypted(&self) -> bool {
        self.vault.is_encrypted()
    }

    // ---- sidebar ---------------------------------------------------------------------

    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for s in &self.nb.sections {
            out.push(Row { id: format!("section:{}", s.id), title: s.name.clone(), section: s.name.clone(), section_id: s.id.clone(), is_current: false, is_section: true });
            for p in &s.pages {
                out.push(Row { id: p.id.clone(), title: p.title.clone(), section: s.name.clone(), section_id: s.id.clone(), is_current: p.id == self.page.id, is_section: false });
            }
        }
        out
    }

    pub fn page_ids(&self) -> Vec<String> {
        self.nb.sections.iter().flat_map(|s| s.pages.iter().map(|p| p.id.clone())).collect()
    }

    pub fn title_of(&self, id: &str) -> Option<String> {
        self.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.id == id).map(|p| p.title.clone())
    }

    pub fn new_section(&mut self, name: &str) -> store::Result<()> {
        self.nb.add_section(&store::new_id(), name);
        self.store.save_notebook(&self.nb, &self.vault)
    }

    /// ▲/▼ buttons: move a page one place up or down, crossing into the neighbouring section.
    pub fn move_page_step(&mut self, id: &str, dir: i32) -> store::Result<bool> {
        let Some((si, pi)) = self.nb.find_page(id) else { return Ok(false) };
        let moved = if dir < 0 {
            if pi > 0 {
                let sid = self.nb.sections[si].id.clone();
                self.nb.move_page(id, &sid, pi - 1)
            } else if si > 0 {
                let sid = self.nb.sections[si - 1].id.clone();
                let end = self.nb.sections[si - 1].pages.len();
                self.nb.move_page(id, &sid, end)
            } else {
                false
            }
        } else if pi + 1 < self.nb.sections[si].pages.len() {
            let sid = self.nb.sections[si].id.clone();
            self.nb.move_page(id, &sid, pi + 1)
        } else if si + 1 < self.nb.sections.len() {
            let sid = self.nb.sections[si + 1].id.clone();
            self.nb.move_page(id, &sid, 0)
        } else {
            false
        };
        if moved {
            self.store.save_notebook(&self.nb, &self.vault)?;
        }
        Ok(moved)
    }

    /// Drag and drop: drop page `src` onto a page row (goes before it) or a section header (goes first).
    pub fn drop_page(&mut self, src: &str, target: &str) -> store::Result<bool> {
        if src.is_empty() || src == target {
            return Ok(false);
        }
        let ok = if let Some(sec) = target.strip_prefix("section:") {
            self.nb.move_page(src, sec, 0)
        } else {
            let Some((si, _)) = self.nb.find_page(target) else { return Ok(false) };
            let sid = self.nb.sections[si].id.clone();
            let Some((ssi, spi)) = self.nb.find_page(src) else { return Ok(false) };
            let (_, ti) = self.nb.find_page(target).unwrap_or((si, 0));
            // removing src first shifts the target left when both are in the same section
            let idx = if ssi == si && spi < ti { ti - 1 } else { ti };
            self.nb.move_page(src, &sid, idx)
        };
        if ok {
            self.store.save_notebook(&self.nb, &self.vault)?;
        }
        Ok(ok)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub fn touch(&mut self) {
        self.dirty_since.get_or_insert(store::now_ms());
    }

    /// Replace the current page's ink (called after every edit).
    pub fn set_strokes(&mut self, strokes: Vec<Stroke>, now: u64) {
        if strokes != self.page.strokes {
            self.page.strokes = strokes;
            self.dirty_since.get_or_insert(now);
        }
    }

    pub fn set_paper(&mut self, kind: &str, spacing: f32) {
        if self.page.paper.kind != kind || (self.page.paper.spacing - spacing).abs() > f32::EPSILON {
            self.page.paper.kind = kind.into();
            self.page.paper.spacing = spacing;
            self.touch();
        }
    }

    pub fn rename(&mut self, title: &str) {
        if self.page.title == title {
            return;
        }
        self.page.title = title.into();
        self.nb.rename_page(&self.page.id.clone(), title);
        self.touch();
    }

    pub fn set_tags(&mut self, tags: &str) {
        let t: Vec<String> = tags.split([',', ' ']).map(|t| t.trim().trim_start_matches('#').to_lowercase()).filter(|t| !t.is_empty()).collect();
        if t != self.page.tags {
            self.page.tags = t;
            self.touch();
        }
    }

    pub fn toggle_favorite(&mut self) {
        self.page.favorite = !self.page.favorite;
        self.touch();
    }

    pub fn save(&mut self, now: u64) -> store::Result<()> {
        self.store.save_page(&self.nb.id, &mut self.page, &self.vault, now)?;
        self.nb.rename_page(&self.page.id.clone(), &self.page.title.clone());
        self.store.save_notebook(&self.nb, &self.vault)?;
        if !self.vault.is_encrypted() {
            self.store.touch_recent(&self.nb.id, &self.page.id)?;
        }
        self.dirty_since = None;
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        let nb = self.nb.id.clone();
        if let Some(ix) = &mut self.index {
            let _ = ix.add(&doc_from_page(&nb, &self.page, ""));
            let _ = ix.commit();
        }
        Ok(())
    }

    /// Call about once a second: crash-recovery snapshot soon after a change, full save after 2 s idle.
    pub fn tick(&mut self, now: u64) -> store::Result<bool> {
        let Some(since) = self.dirty_since else { return Ok(false) };
        if now.saturating_sub(self.last_recovery_ms) > 1000 {
            self.store.write_recovery(&self.nb.id, &self.page, &self.vault)?;
            self.last_recovery_ms = now;
        }
        if now.saturating_sub(since) >= 2000 {
            self.save(now)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The page file changed on disk behind our back (sync tools, git, another editor).
    pub fn changed_on_disk(&self) -> bool {
        let now = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        now.is_some() && self.disk_fp.is_some() && now != self.disk_fp
    }

    /// Reload the open page from disk (after the user accepted the external change).
    pub fn reload_from_disk(&mut self) -> store::Result<()> {
        self.page = self.store.load_page(&self.nb.id, &self.page.id, &self.vault)?;
        self.dirty_since = None;
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    /// Keep our version and stop warning about this external change.
    pub fn ignore_disk_change(&mut self) {
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        self.touch();
    }

    pub fn templates(&self) -> Vec<String> {
        let mut v: Vec<String> = store::templates::BUILTIN.iter().map(|s| s.to_string()).collect();
        v.extend(self.store.list_templates().into_iter().map(|t| format!("my: {t}")));
        v
    }

    fn page_for_template(&self, template: &str, now: u64) -> Option<Page> {
        if let Some(name) = template.strip_prefix("my: ") {
            let mut p = self.store.load_template(name).ok()?;
            p.id = store::new_id();
            p.date = store::iso_date(now);
            p.modified_ms = 0;
            return Some(p);
        }
        self.store.page_from_template(template, now)
    }

    pub fn save_as_template(&self, name: &str) -> store::Result<()> {
        let mut p = self.page.clone();
        p.strokes.clear();
        p.audio = None;
        p.pdf_background = None;
        p.title = name.into();
        self.store.save_template(name, &p)
    }

    pub fn new_page(&mut self, template: &str, now: u64) -> store::Result<()> {
        if self.is_dirty() {
            self.save(now)?;
        }
        let page = self.page_for_template(template, now).ok_or_else(|| store::StoreError::NotFound(format!("template {template}")))?;
        let section = self.nb.find_page(&self.page.id).map(|(si, _)| self.nb.sections[si].id.clone()).or_else(|| self.nb.sections.last().map(|s| s.id.clone())).ok_or_else(|| store::StoreError::NotFound("section".into()))?;
        self.store.create_page(&mut self.nb, &section, page.clone(), &self.vault)?;
        self.page = page;
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    /// Add an already-built page (PDF import) after the current one and open it.
    pub fn add_page(&mut self, page: Page, now: u64) -> store::Result<()> {
        if self.is_dirty() {
            self.save(now)?;
        }
        let section = self.nb.find_page(&self.page.id).map(|(si, _)| self.nb.sections[si].id.clone()).unwrap_or_else(|| self.nb.sections[0].id.clone());
        let mut p = page;
        self.store.create_page(&mut self.nb, &section, p.clone(), &self.vault)?;
        self.store.save_page(&self.nb.id, &mut p, &self.vault, now)?;
        self.page = p;
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    pub fn open_page(&mut self, id: &str, now: u64) -> store::Result<()> {
        if id == self.page.id {
            return Ok(());
        }
        let next = self.store.load_page(&self.nb.id, id, &self.vault)?;
        if self.is_dirty() {
            self.save(now)?;
        }
        self.page = next;
        self.apply_recovery();
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    pub fn load_other(&self, id: &str) -> Option<Page> {
        self.store.load_page(&self.nb.id, id, &self.vault).ok()
    }

    pub fn duplicate_current(&mut self, now: u64) -> store::Result<()> {
        if self.is_dirty() {
            self.save(now)?;
        }
        let id = self.store.duplicate_page(&mut self.nb, &self.page.id.clone(), &self.vault)?;
        self.open_page(&id, now)
    }

    /// Move the current page to the trash and open a neighbour (or a fresh page).
    pub fn delete_current(&mut self, now: u64) -> store::Result<()> {
        let id = self.page.id.clone();
        let next = {
            let all: Vec<&PageRef> = self.nb.sections.iter().flat_map(|s| s.pages.iter()).collect();
            let i = all.iter().position(|p| p.id == id).unwrap_or(0);
            all.iter().enumerate().find(|(k, _)| *k != i).map(|(_, p)| p.id.clone())
        };
        if self.is_dirty() {
            self.save(now)?;
        }
        self.dirty_since = None;
        self.store.clear_recovery(&self.nb.id, &id);
        self.store.delete_page(&mut self.nb, &id, &self.vault, now)?;
        if let Some(ix) = &mut self.index {
            ix.remove(&id);
            let _ = ix.commit();
        }
        match next {
            Some(n) => self.page = self.store.load_page(&self.nb.id, &n, &self.vault)?,
            None => self.new_page("blank", now)?,
        }
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    pub fn trash(&self) -> Vec<store::TrashEntry> {
        let mut t = self.store.list_trash(&self.nb.id, &self.vault);
        t.sort_by_key(|e| std::cmp::Reverse(e.deleted_ms));
        t
    }

    pub fn restore_from_trash(&mut self, id: &str, now: u64) -> store::Result<()> {
        self.store.restore_page(&mut self.nb, id, &self.vault)?;
        self.index = None;
        self.open_page(id, now)
    }

    pub fn history(&self) -> Vec<u64> {
        let mut h = self.store.history(&self.nb.id, &self.page.id, &self.vault);
        h.reverse();
        h
    }

    pub fn snapshot(&self, t: u64) -> Option<Page> {
        self.store.load_snapshot(&self.nb.id, &self.page.id, t, &self.vault).ok()
    }

    pub fn restore_snapshot(&mut self, t: u64, now: u64) -> store::Result<()> {
        if self.is_dirty() {
            self.save(now)?;
        }
        self.page = self.store.restore_snapshot(&self.nb.id, &self.page.id.clone(), t, &self.vault, now)?;
        self.nb.rename_page(&self.page.id.clone(), &self.page.title.clone());
        self.store.save_notebook(&self.nb, &self.vault)?;
        self.dirty_since = None;
        self.disk_fp = self.store.page_fingerprint(&self.nb.id, &self.page.id, &self.vault);
        Ok(())
    }

    // ---- media ---------------------------------------------------------------------

    /// Store media bytes under a fresh name (keeping the extension) and return the name.
    pub fn add_media(&self, ext: &str, bytes: &[u8]) -> store::Result<String> {
        let ext: String = ext.chars().filter(|c| c.is_ascii_alphanumeric()).take(5).collect::<String>().to_lowercase();
        let name = format!("{}.{}", store::new_id(), if ext.is_empty() { "bin" } else { &ext });
        self.store.write_media(&self.nb.id, &name, bytes, &self.vault)?;
        Ok(name)
    }

    pub fn media(&self, name: &str) -> Option<Vec<u8>> {
        self.store.read_media(&self.nb.id, name, &self.vault).ok()
    }

    /// Every media file the page references (for rendering and export).
    pub fn page_media(&self, p: &Page) -> export::Media {
        let mut v = Vec::new();
        for b in &p.blocks {
            if let store::Block::Image { file, .. } = b {
                if let Some(bytes) = self.media(file) {
                    v.push((file.clone(), bytes));
                }
            }
        }
        v
    }

    // ---- search --------------------------------------------------------------------

    fn index_dir(&self) -> PathBuf {
        self.store.root.join(".index").join(&self.nb.id)
    }

    /// Build or update the index. Unencrypted notebooks keep it on disk with a manifest of page
    /// fingerprints so only changed pages are re-indexed; encrypted ones are indexed in memory only.
    fn ensure_index(&mut self) {
        if self.index.is_some() {
            return;
        }
        let persistent = !self.vault.is_encrypted();
        let opened = if persistent { SearchIndex::open(&self.index_dir()).or_else(|_| SearchIndex::in_memory()) } else { SearchIndex::in_memory() };
        let Ok(mut ix) = opened else { return };
        let manifest_path = self.index_dir().join("manifest.json");
        let mut manifest: HashMap<String, (u128, u64)> = if persistent { std::fs::read(&manifest_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() } else { HashMap::new() };
        let ids = self.page_ids();
        for stale in manifest.keys().filter(|k| !ids.contains(k)).cloned().collect::<Vec<_>>() {
            ix.remove(&stale);
            manifest.remove(&stale);
        }
        for id in &ids {
            let fp = self.store.page_fingerprint(&self.nb.id, id, &self.vault);
            if fp.is_some() && manifest.get(id) == fp.as_ref() {
                continue;
            }
            if let Ok(p) = self.store.load_page(&self.nb.id, id, &self.vault) {
                let _ = ix.add(&doc_from_page(&self.nb.id, &p, ""));
                if let Some(fp) = fp {
                    manifest.insert(id.clone(), fp);
                }
            }
        }
        let _ = ix.commit();
        if persistent {
            if let Ok(b) = serde_json::to_vec(&manifest) {
                let _ = store::fsio::atomic_write(&manifest_path, &b);
            }
        }
        self.index = Some(ix);
    }

    /// Full-text search; `tag:`, `after:` and `before:` filters are understood.
    pub fn search(&mut self, query: &str) -> Vec<Hit> {
        self.ensure_index();
        let (q, mut f) = parse_query(query);
        f.notebook = Some(self.nb.id.clone());
        self.index.as_ref().and_then(|ix| ix.search(&q, &f, 30).ok()).unwrap_or_default()
    }

    /// Pages whose text links to the current page with `[[Title]]`.
    pub fn backlinks(&mut self) -> Vec<(String, String)> {
        let title = self.page.title.trim().to_string();
        if title.is_empty() {
            return vec![];
        }
        let link = format!("[[{}]]", title.to_lowercase());
        let me = self.page.id.clone();
        let hits = self.search(&title.replace(['"', ':', '(', ')', '[', ']'], " "));
        hits.into_iter()
            .filter(|h| h.page_id != me)
            .filter_map(|h| {
                let p = self.load_other(&h.page_id)?;
                let has = p.blocks.iter().any(|b| matches!(b, store::Block::Text { markup } | store::Block::Callout { markup, .. } if markup.to_lowercase().contains(&link)));
                has.then_some((h.page_id, h.title))
            })
            .take(20)
            .collect()
    }

    /// Find a page in this notebook by title (case-insensitive) for `[[links]]`.
    pub fn page_by_title(&self, title: &str) -> Option<String> {
        let t = title.trim().to_lowercase();
        self.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.title.trim().to_lowercase() == t).map(|p| p.id.clone())
    }

    pub fn export_current(&mut self, fmt: export::Format, dir: &Path, now: u64, background: Option<Vec<u8>>) -> Result<PathBuf, String> {
        if self.is_dirty() {
            self.save(now).map_err(|e| e.to_string())?;
        }
        let media = self.page_media(&self.page.clone());
        let bytes = export::export_pages_with(&[&self.page], fmt, &media, &[background])?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let safe: String = self.page.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let safe = if safe.trim().is_empty() { "page".to_string() } else { safe.trim().to_string() };
        let path = dir.join(format!("{safe}.{}", fmt.extension()));
        store::fsio::atomic_write(&path, &bytes).map_err(|e| e.to_string())?;
        // text formats reference images by name: put them next to the file
        if matches!(fmt, export::Format::Markdown | export::Format::Latex | export::Format::Typst) {
            for (n, b) in &media {
                let _ = store::fsio::atomic_write(&dir.join(n), b);
            }
        }
        Ok(path)
    }

    /// Whole notebook as a zip of per-page files.
    pub fn export_notebook(&mut self, fmt: export::Format, dir: &Path, now: u64) -> Result<PathBuf, String> {
        if self.is_dirty() {
            self.save(now).map_err(|e| e.to_string())?;
        }
        let pages: Vec<Page> = self.page_ids().iter().filter_map(|id| self.load_other(id)).collect();
        let refs: Vec<&Page> = pages.iter().collect();
        let bytes = if fmt == export::Format::Pdf {
            let media: export::Media = pages.iter().flat_map(|p| self.page_media(p)).collect();
            export::export_pages_with(&refs, fmt, &media, &[])?
        } else {
            export::export_zip(&refs, fmt)?
        };
        let safe: String = self.nb.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' }).collect();
        let path = dir.join(format!("{}.{}", safe.trim(), if fmt == export::Format::Pdf { "pdf" } else { "zip" }));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        store::fsio::atomic_write(&path, &bytes).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::{InkMode, PenKind, Point};

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("scrawl-ws-{}", store::new_id()))
    }

    fn stroke(id: u64) -> Stroke {
        Stroke { id, points: vec![Point::new(0.0, 0.0, 0), Point::new(10.0, 10.0, 5)], color: [255; 4], width: 3.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: 0 }
    }

    fn pages(ws: &Workspace) -> Vec<Row> {
        ws.rows().into_iter().filter(|r| !r.is_section).collect()
    }

    #[test]
    fn first_run_creates_a_notebook_and_page_and_ink_persists() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        assert_eq!(pages(&ws).len(), 1);
        ws.set_strokes(vec![stroke(1), stroke(2)], 1000);
        assert!(ws.is_dirty());
        ws.save(2000).unwrap();
        assert!(!ws.is_dirty());
        let ws2 = Workspace::open(&d).unwrap();
        assert_eq!(ws2.page.strokes.len(), 2, "ink survives a restart");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn autosave_after_idle_and_crash_recovery_before_it() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        ws.set_strokes(vec![stroke(1)], 10_000);
        assert!(!ws.tick(10_500).unwrap(), "not yet: 2 s debounce");
        drop(ws); // "crash": recovery snapshot exists, page file not updated
        let ws2 = Workspace::open(&d).unwrap();
        assert_eq!(ws2.page.strokes.len(), 1, "unsaved ink recovered");
        assert!(ws2.recovered.is_some());
        let mut ws2 = ws2;
        assert!(ws2.tick(store::now_ms() + 5000).unwrap(), "autosave fires after 2 s");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn pages_templates_switching_trash_restore_and_search() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        let first = ws.page.id.clone();
        ws.new_page("problem set", 5000).unwrap();
        assert_eq!(pages(&ws).len(), 2);
        ws.rename("Homework 3 eigenvalues");
        ws.set_tags("linear-algebra, hw");
        ws.save(6000).unwrap();
        assert_eq!(ws.search("eigenvalues").len(), 1);
        assert_eq!(ws.search("tag:hw").len(), 1, "tag filter");
        assert_eq!(ws.search("tag:nothing eigenvalues").len(), 0);
        let hw = ws.page.id.clone();
        ws.open_page(&first, 7000).unwrap();
        ws.open_page(&hw, 7100).unwrap();
        ws.delete_current(8000).unwrap();
        assert_eq!(pages(&ws).len(), 1, "moved to trash");
        assert_eq!(ws.search("eigenvalues").len(), 0, "trashed pages leave the index");
        assert_eq!(ws.trash().len(), 1);
        ws.restore_from_trash(&hw, 9000).unwrap();
        assert_eq!(pages(&ws).len(), 2);
        assert_eq!(ws.page.title, "Homework 3 eigenvalues");
        let out = ws.export_current(export::Format::Markdown, &d.join("exports"), 9000, None).unwrap();
        assert!(out.exists());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn sections_move_and_drop() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        let a = ws.page.id.clone();
        ws.new_page("blank", 1).unwrap();
        let b = ws.page.id.clone();
        ws.new_section("Week 2").unwrap();
        let sec2 = ws.nb.sections[1].id.clone();
        assert!(ws.drop_page(&a, &format!("section:{sec2}")).unwrap());
        assert_eq!(ws.nb.sections[1].pages[0].id, a);
        assert!(ws.move_page_step(&a, -1).unwrap(), "up crosses into the previous section");
        assert_eq!(ws.nb.sections[0].pages.last().unwrap().id, a);
        assert!(ws.drop_page(&a, &b).unwrap());
        assert_eq!(ws.nb.sections[0].pages[0].id, a, "dropped before b");
        let ws2 = Workspace::open(&d).unwrap();
        assert_eq!(ws2.nb.sections.len(), 2, "persisted");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn encrypted_notebook_wrong_password_and_no_plaintext_index() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        let plain_nb = ws.nb.id.clone();
        ws.create_notebook("Diary", Some("correct horse"), 1).unwrap();
        let enc = ws.nb.id.clone();
        ws.rename("secret eigenvalue diary");
        ws.save(2).unwrap();
        assert_eq!(ws.search("eigenvalue").len(), 1);
        ws.switch_notebook(&plain_nb, None, 3).unwrap();
        let e = ws.switch_notebook(&enc, Some("wrong"), 4).unwrap_err();
        assert!(e.contains("Wrong password"), "{e}");
        assert_eq!(ws.nb.id, plain_nb, "nothing changed");
        ws.switch_notebook(&enc, Some("correct horse"), 5).unwrap();
        assert_eq!(ws.page.title, "secret eigenvalue diary");
        // no plaintext anywhere on disk (pages, index, recovery, recent)
        let mut stack = vec![d.clone()];
        while let Some(p) = stack.pop() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    assert!(!String::from_utf8_lossy(&bytes).contains("eigenvalue"), "plaintext leaked into {path:?}");
                }
            }
        }
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn history_snapshot_and_restore() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        ws.rename("v1");
        ws.save(1_000_000).unwrap();
        ws.rename("v2");
        ws.save(1_000_000 + store::history::SNAPSHOT_INTERVAL_MS + 1).unwrap();
        let h = ws.history();
        assert!(!h.is_empty());
        let t = *h.iter().find(|&&t| ws.snapshot(t).is_some_and(|p| p.title == "v1")).expect("a v1 snapshot");
        let old = ws.snapshot(t).unwrap();
        assert_eq!(old.title, "v1");
        ws.restore_snapshot(t, 9_000_000).unwrap();
        assert_eq!(ws.page.title, "v1");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn external_change_is_detected() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        ws.rename("mine");
        ws.save(1).unwrap();
        assert!(!ws.changed_on_disk());
        let mut other = ws.store.load_page(&ws.nb.id, &ws.page.id, &ws.vault).unwrap();
        other.title = "edited elsewhere with a longer title".into();
        std::thread::sleep(std::time::Duration::from_millis(20));
        ws.store.save_page(&ws.nb.id.clone(), &mut other, &ws.vault, 2).unwrap();
        assert!(ws.changed_on_disk());
        ws.reload_from_disk().unwrap();
        assert_eq!(ws.page.title, "edited elsewhere with a longer title");
        assert!(!ws.changed_on_disk());
        std::fs::remove_dir_all(d).ok();
    }
}

/// The first page a new user sees: how to type, how to draw, how handwriting becomes text.
pub fn welcome_page(now: u64) -> Page {
    use notedoc::{ParaKind as K, Run};
    let mut p = Page::new(&store::new_id(), "Welcome to Scrawl", &store::iso_date(now));
    let mut d = notedoc::Doc::new();
    let t = |d: &mut notedoc::Doc, k: K, s: &str| {
        d.push(k, vec![Run::plain(s)]);
    };
    t(&mut d, K::Body, "This is an ordinary page. Click anywhere and type; Enter starts a new paragraph.");
    t(&mut d, K::Heading(2), "Type");
    t(&mut d, K::Bullet { indent: 0 }, "Start a line with - for bullets, 1. for numbers, [] for a checklist, # for a heading.");
    t(&mut d, K::Bullet { indent: 0 }, "Select text and use the bar above, or Ctrl+B and Ctrl+I.");
    d.push(K::Bullet { indent: 0 }, vec![Run::plain("Put math inside a sentence with dollar signs: "), Run::Math { typst: "x^2 + 1".into() }, Run::plain(".")]);
    t(&mut d, K::Heading(2), "Draw and write by hand");
    t(&mut d, K::Bullet { indent: 0 }, "Tap Pen at the bottom and write or sketch anywhere. Ink stays beside the text it was written next to.");
    t(&mut d, K::Bullet { indent: 0 }, "Write words or math and pause: a suggestion appears. Accept turns it into typed text or a formula; Keep ink leaves it as handwriting.");
    t(&mut d, K::Bullet { indent: 0 }, "Tap Aa to go back to typing.");
    t(&mut d, K::Heading(2), "Find things");
    t(&mut d, K::Check { done: false }, "Try + Insert for images, tables, callouts and graphs.");
    t(&mut d, K::Check { done: false }, "The ... menu has everything else. Ctrl+K finds any command.");
    d.push(K::Body, vec![]);
    p.doc = Some(d);
    p.sync_blocks();
    p
}

#[cfg(test)]
mod welcome_tests {
    use super::*;

    #[test]
    fn welcome_page_has_a_document_and_matching_blocks() {
        let p = welcome_page(1_700_000_000_000);
        let d = p.doc.as_ref().unwrap();
        assert!(d.paras.len() >= 10);
        assert!(!p.blocks.is_empty(), "blocks are kept in sync for export and search");
        assert!(p.to_typst().contains("Draw and write by hand"));
        assert!(d.plain_text().contains("Ctrl+K"));
    }
}

impl Workspace {
    /// Every tag in the notebook with the pages that carry it: (tag, [(page id, title)]), most used first.
    pub fn tags_overview(&self) -> Vec<(String, Vec<(String, String)>)> {
        let mut map: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
        for sec in &self.nb.sections {
            for r in &sec.pages {
                let tags = if r.id == self.page.id { Some(self.page.tags.clone()) } else { self.load_other(&r.id).map(|p| p.tags) };
                for t in tags.unwrap_or_default() {
                    let t = t.trim().trim_start_matches('#').to_lowercase();
                    if !t.is_empty() {
                        let e = map.entry(t).or_default();
                        if !e.iter().any(|(id, _)| *id == r.id) {
                            e.push((r.id.clone(), r.title.clone()));
                        }
                    }
                }
            }
        }
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        v
    }
}
