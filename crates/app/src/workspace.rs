//! The notes workspace: current notebook/page, autosave with crash recovery, lazy search index.

use ink::Stroke;
use search::{doc_from_page, Filters, Hit, SearchIndex};
use std::path::{Path, PathBuf};
use store::{Notebook, Page, PageRef, Store, Vault};

pub struct Workspace {
    pub store: Store,
    pub nb: Notebook,
    pub vault: Vault,
    pub page: Page,
    dirty_since: Option<u64>,
    last_recovery_ms: u64,
    index: Option<SearchIndex>,
    pub recovered: Option<String>,
}

/// One row in the sidebar tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub section: String,
    pub is_current: bool,
}

pub fn default_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Scrawl")
}

impl Workspace {
    /// Open (or create) the notes folder, load the first notebook and its first page.
    pub fn open(root: &Path) -> store::Result<Self> {
        let store = Store::open(root)?;
        let vault = Vault::none();
        let metas = store.list_notebooks();
        let mut nb = match metas.iter().find(|m| !m.encrypted) {
            Some(m) => store.load_notebook(&m.id, &vault)?,
            None => {
                let mut nb = store.create_notebook("My notebook", &vault)?;
                nb.add_section("notes", "Notes");
                store.save_notebook(&nb, &vault)?;
                nb
            }
        };
        if nb.sections.is_empty() {
            nb.add_section("notes", "Notes");
        }
        let first = nb.sections.iter().flat_map(|s| s.pages.first()).next().map(|p| p.id.clone());
        let page = match first {
            Some(id) => store.load_page(&nb.id, &id, &vault)?,
            None => {
                let p = store.page_from_template("lecture notes", store::now_ms()).unwrap_or_else(|| Page::new(&store::new_id(), "Untitled", &store::iso_date(store::now_ms())));
                let sec = nb.sections[0].id.clone();
                store.create_page(&mut nb, &sec, p.clone(), &vault)?;
                p
            }
        };
        let mut ws = Self { store, nb, vault, page, dirty_since: None, last_recovery_ms: 0, index: None, recovered: None };
        ws.apply_recovery();
        Ok(ws)
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

    pub fn rows(&self) -> Vec<Row> {
        self.nb
            .sections
            .iter()
            .flat_map(|s| s.pages.iter().map(move |p| (s, p)))
            .map(|(s, p)| Row { id: p.id.clone(), title: p.title.clone(), section: s.name.clone(), is_current: p.id == self.page.id })
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
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
            self.dirty_since.get_or_insert(store::now_ms());
        }
    }

    pub fn rename(&mut self, title: &str) {
        self.page.title = title.into();
        self.nb.rename_page(&self.page.id.clone(), title);
        self.dirty_since.get_or_insert(store::now_ms());
    }

    pub fn save(&mut self, now: u64) -> store::Result<()> {
        self.store.save_page(&self.nb.id, &mut self.page, &self.vault, now)?;
        self.nb.rename_page(&self.page.id.clone(), &self.page.title.clone());
        self.store.save_notebook(&self.nb, &self.vault)?;
        self.store.touch_recent(&self.nb.id, &self.page.id)?;
        self.dirty_since = None;
        if let Some(ix) = &mut self.index {
            let _ = ix.add(&doc_from_page(&self.nb.id, &self.page, ""));
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

    pub fn new_page(&mut self, template: &str, now: u64) -> store::Result<()> {
        if self.is_dirty() {
            self.save(now)?;
        }
        let page = self.store.page_from_template(template, now).ok_or_else(|| store::StoreError::NotFound(format!("template {template}")))?;
        let section = self.nb.sections.last().map(|s| s.id.clone()).ok_or_else(|| store::StoreError::NotFound("section".into()))?;
        self.store.create_page(&mut self.nb, &section, page.clone(), &self.vault)?;
        self.page = page;
        Ok(())
    }

    pub fn open_page(&mut self, id: &str, now: u64) -> store::Result<()> {
        if id == self.page.id {
            return Ok(());
        }
        if self.is_dirty() {
            self.save(now)?;
        }
        self.page = self.store.load_page(&self.nb.id, id, &self.vault)?;
        self.apply_recovery();
        Ok(())
    }

    /// Move the current page to the trash and open a neighbour (or a fresh page).
    pub fn delete_current(&mut self, now: u64) -> store::Result<()> {
        let id = self.page.id.clone();
        let next = {
            let all: Vec<&PageRef> = self.nb.sections.iter().flat_map(|s| s.pages.iter()).collect();
            let i = all.iter().position(|p| p.id == id).unwrap_or(0);
            all.iter().enumerate().find(|(k, _)| *k != i).map(|(_, p)| p.id.clone())
        };
        self.dirty_since = None;
        self.store.clear_recovery(&self.nb.id, &id);
        self.store.delete_page(&mut self.nb, &id, &self.vault, now)?;
        match next {
            Some(n) => self.page = self.store.load_page(&self.nb.id, &n, &self.vault)?,
            None => self.new_page("blank", now)?,
        }
        Ok(())
    }

    fn ensure_index(&mut self) {
        if self.index.is_some() {
            return;
        }
        if let Ok(mut ix) = SearchIndex::in_memory() {
            for r in self.nb.sections.iter().flat_map(|s| s.pages.iter()) {
                if let Ok(p) = self.store.load_page(&self.nb.id, &r.id, &self.vault) {
                    let _ = ix.add(&doc_from_page(&self.nb.id, &p, ""));
                }
            }
            let _ = ix.commit();
            self.index = Some(ix);
        }
    }

    /// Full-text search; the index is built lazily on first use.
    pub fn search(&mut self, query: &str) -> Vec<Hit> {
        self.ensure_index();
        self.index.as_ref().and_then(|ix| ix.search(query, &Filters::default(), 20).ok()).unwrap_or_default()
    }

    pub fn export_current(&mut self, fmt: export::Format, dir: &Path, now: u64) -> Result<PathBuf, String> {
        if self.is_dirty() {
            self.save(now).map_err(|e| e.to_string())?;
        }
        let bytes = export::export_page(&self.page, fmt)?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let safe: String = self.page.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' }).collect();
        let path = dir.join(format!("{}.{}", safe.trim(), fmt.extension()));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
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

    #[test]
    fn first_run_creates_a_notebook_and_page_and_ink_persists() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        assert_eq!(ws.rows().len(), 1);
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
        // "crash": drop without saving; recovery snapshot exists
        drop(ws);
        let ws2 = Workspace::open(&d).unwrap();
        assert_eq!(ws2.page.strokes.len(), 1, "unsaved ink recovered");
        assert!(ws2.recovered.is_some());
        let mut ws2 = ws2;
        assert!(ws2.tick(store::now_ms() + 5000).unwrap(), "autosave fires after 2 s");
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn pages_templates_switching_trash_and_search() {
        let d = tmp();
        let mut ws = Workspace::open(&d).unwrap();
        let first = ws.page.id.clone();
        ws.new_page("problem set", 5000).unwrap();
        assert_eq!(ws.rows().len(), 2);
        ws.rename("Homework 3 eigenvalues");
        ws.save(6000).unwrap();
        assert_eq!(ws.search("eigenvalues").len(), 1);
        ws.open_page(&first, 7000).unwrap();
        assert_eq!(ws.page.id, first);
        ws.delete_current(8000).unwrap();
        assert_eq!(ws.rows().len(), 1, "moved to trash");
        assert_eq!(ws.store.list_trash(&ws.nb.id, &ws.vault).len(), 1);
        let out = ws.export_current(export::Format::Markdown, &d.join("exports"), 9000).unwrap();
        assert!(out.exists());
        std::fs::remove_dir_all(d).ok();
    }
}
