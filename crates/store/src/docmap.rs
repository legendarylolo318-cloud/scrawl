//! Conversion between the old block/canvas-text page content and the document model.
//!
//! `Page.doc` is the source of truth once it exists. `Page.blocks` is kept in sync from it on every save,
//! so exports, search and older versions of Scrawl keep working without knowing about `doc`.

use crate::model::{Block, CalloutKind, Page};
use notedoc::{CalloutKind as DocCallout, Doc, ParaKind};

fn to_doc_kind(k: CalloutKind) -> DocCallout {
    match k {
        CalloutKind::Definition => DocCallout::Definition,
        CalloutKind::Theorem => DocCallout::Theorem,
        CalloutKind::Example => DocCallout::Example,
        CalloutKind::Warning => DocCallout::Warning,
    }
}

fn from_doc_kind(k: DocCallout) -> CalloutKind {
    match k {
        DocCallout::Definition => CalloutKind::Definition,
        DocCallout::Theorem => CalloutKind::Theorem,
        DocCallout::Example => CalloutKind::Example,
        DocCallout::Warning => CalloutKind::Warning,
    }
}

/// Build the document for a page that only has blocks and canvas text (a page from before the editor).
pub fn doc_from_page(p: &Page) -> Doc {
    let mut d = Doc::new();
    for b in &p.blocks {
        match b {
            Block::Text { markup } => d.append_markup(markup),
            Block::Callout { kind, markup } if markup.trim().is_empty() => {
                // an empty callout is still a box the user made
                d.push(ParaKind::Callout(to_doc_kind(*kind)), vec![]);
            }
            Block::Callout { kind, markup } => d.append_markup_as(markup, Some(to_doc_kind(*kind))),
            Block::Math { expr, source } => {
                d.push(ParaKind::MathDisplay { expr: expr.clone(), source: source.clone() }, vec![]);
            }
            Block::Image { file, x, y, w, h, crop } => {
                d.push(ParaKind::Image { file: file.clone(), x: *x, y: *y, w: *w, h: *h, crop: *crop }, vec![]);
            }
            Block::Plot { functions, x_range, y_range } => {
                d.push(ParaKind::Plot { functions: functions.clone(), x_range: *x_range, y_range: *y_range }, vec![]);
            }
            Block::Rule => {
                d.push(ParaKind::Rule, vec![]);
            }
        }
    }
    // text typed onto the canvas: reading order (top to bottom, then left to right), one paragraph per line
    let mut texts: Vec<&crate::model::CanvasText> = p.texts.iter().collect();
    texts.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap_or(std::cmp::Ordering::Equal));
    for t in texts {
        for line in t.text.lines().filter(|l| !l.trim().is_empty()) {
            d.push(ParaKind::Body, vec![notedoc::Run::plain(line)]);
        }
    }
    d
}

/// Blocks equivalent to a document (consecutive list items become one text block).
pub fn blocks_from_doc(d: &Doc) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut list: Vec<String> = Vec::new();
    let flush = |out: &mut Vec<Block>, list: &mut Vec<String>| {
        if !list.is_empty() {
            out.push(Block::Text { markup: list.join("\n") });
            list.clear();
        }
    };
    for p in &d.paras {
        if p.kind.is_list() {
            if let Some(m) = notedoc::para_markup(p) {
                list.push(m);
            }
            continue;
        }
        flush(&mut out, &mut list);
        match &p.kind {
            ParaKind::MathDisplay { expr, source } => out.push(Block::Math { expr: expr.clone(), source: source.clone() }),
            ParaKind::Image { file, x, y, w, h, crop } => out.push(Block::Image { file: file.clone(), x: *x, y: *y, w: *w, h: *h, crop: *crop }),
            ParaKind::Plot { functions, x_range, y_range } => out.push(Block::Plot { functions: functions.clone(), x_range: *x_range, y_range: *y_range }),
            ParaKind::Rule => out.push(Block::Rule),
            ParaKind::Callout(k) => {
                // the callout text without the paragraph guard (callout markup is not a body line)
                let mut plain = p.clone();
                plain.kind = ParaKind::Body;
                out.push(Block::Callout { kind: from_doc_kind(*k), markup: notedoc::para_markup(&plain).unwrap_or_default() });
            }
            _ => {
                if let Some(m) = notedoc::para_markup(p) {
                    out.push(Block::Text { markup: m });
                }
            }
        }
    }
    flush(&mut out, &mut list);
    out
}

/// The block an object paragraph stands for (for rendering it with the block renderer).
pub fn block_of(kind: &ParaKind) -> Option<Block> {
    Some(match kind {
        ParaKind::MathDisplay { expr, source } => Block::Math { expr: expr.clone(), source: source.clone() },
        ParaKind::Image { file, x, y, w, h, crop } => Block::Image { file: file.clone(), x: *x, y: *y, w: *w, h: *h, crop: *crop },
        ParaKind::Plot { functions, x_range, y_range } => Block::Plot { functions: functions.clone(), x_range: *x_range, y_range: *y_range },
        ParaKind::Rule => Block::Rule,
        ParaKind::Markup(m) => Block::Text { markup: m.clone() },
        _ => return None,
    })
}

impl Page {
    /// Make sure the page has a document, converting old blocks and canvas text once.
    /// Returns true if it converted (the caller should snapshot the old page into history before saving).
    pub fn ensure_doc(&mut self) -> bool {
        if self.doc.is_some() {
            return false;
        }
        let d = doc_from_page(self);
        // every paragraph is now in `doc`; the canvas texts live there too, `blocks` is rebuilt on save
        self.texts.clear();
        self.doc = Some(d);
        self.sync_blocks();
        true
    }

    /// Rebuild `blocks` from `doc` (no-op without a document).
    pub fn sync_blocks(&mut self) {
        if let Some(d) = &self.doc {
            self.blocks = blocks_from_doc(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CanvasText;
    use layout::Expr;

    fn old_page() -> Page {
        let mut p = Page::new("p", "Chain rule", "2026-11-10");
        p.blocks = vec![
            Block::Text { markup: "= Statement\nThe *derivative* of a composite, see [[Limits]].\n- one\n- two\n+ first".into() },
            Block::Math { expr: Expr::Frac(Box::new(Expr::sym("d")), Box::new(Expr::sym("x"))), source: None },
            Block::Callout { kind: CalloutKind::Theorem, markup: "Chain rule holds for $f(g(x))$.".into() },
            Block::Text { markup: "#table(columns: 2, [a], [b])".into() },
            Block::Image { file: "a.png".into(), x: 1.0, y: 2.0, w: 120.0, h: 80.0, crop: Some([0.1, 0.1, 0.1, 0.1]) },
            Block::Plot { functions: vec!["y = sin(x)".into()], x_range: (-3.0, 3.0), y_range: None },
            Block::Callout { kind: CalloutKind::Warning, markup: String::new() },
            Block::Rule,
        ];
        p.texts = vec![
            CanvasText { id: 2, x: 10.0, y: 200.0, text: "second\nthird".into(), size: 24.0, color: [0, 0, 0, 255] },
            CanvasText { id: 1, x: 10.0, y: 100.0, text: "first".into(), size: 24.0, color: [0, 0, 0, 255] },
        ];
        p
    }

    #[test]
    fn old_blocks_survive_the_conversion_unchanged() {
        let p = old_page();
        let before_typst = p.to_typst();
        let mut q = p.clone();
        assert!(q.ensure_doc());
        assert!(!q.ensure_doc(), "second call does nothing");
        // canvas text moved into the document, after the old blocks, in reading order
        let texts: Vec<String> = q.blocks.iter().filter_map(|b| if let Block::Text { markup } = b { Some(markup.clone()) } else { None }).collect();
        assert_eq!(&texts[texts.len() - 3..], ["first", "second", "third"]);
        // everything that was a block is still there, identical
        for b in p.blocks.iter().filter(|b| !matches!(b, Block::Text { markup } if markup.starts_with("= Statement"))) {
            assert!(q.blocks.contains(b), "lost {b:?}");
        }
        // the text block was split into heading, paragraph and lists; the Typst reads the same ignoring blank lines
        let squash = |s: String| s.split_whitespace().collect::<String>();
        assert!(squash(q.to_typst()).starts_with(&squash(before_typst)), "{}", q.to_typst());
        assert!(q.texts.is_empty());
        // a second round trip is stable
        let again = blocks_from_doc(q.doc.as_ref().unwrap());
        assert_eq!(again, q.blocks);
    }

    #[test]
    fn a_page_saved_by_the_previous_version_still_loads() {
        let p = old_page();
        let mut v = serde_json::to_value(&p).unwrap();
        v.as_object_mut().unwrap().remove("doc");
        let back: Page = serde_json::from_value(v).unwrap();
        assert!(back.doc.is_none());
        assert_eq!(back.blocks, p.blocks);
    }

    #[test]
    fn first_save_after_conversion_keeps_the_old_page_in_history() {
        use crate::{Store, Vault};
        let dir = std::env::temp_dir().join(format!("scrawl-docmap-{}", crate::new_id()));
        let store = Store::open(&dir).unwrap();
        let vault = Vault::none();
        let mut p = old_page();
        store.save_page("nb", &mut p, &vault, 1_000_000).unwrap();
        let mut q = store.load_page("nb", "p", &vault).unwrap();
        assert!(q.ensure_doc());
        store.save_page("nb", &mut q, &vault, 2_000_000).unwrap();
        let hist = store.history("nb", "p", &vault);
        assert!(!hist.is_empty());
        let snap = store.load_snapshot("nb", "p", *hist.last().unwrap(), &vault).unwrap();
        assert!(snap.doc.is_none() && snap.blocks == p.blocks && snap.texts.len() == 2, "history holds the page exactly as it was");
        let loaded = store.load_page("nb", "p", &vault).unwrap();
        assert!(loaded.doc.is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}

/// `SCRAWL_MIGRATE_CHECK=<notes folder> cargo test -p store --release migrate_real_notes -- --ignored`:
/// converts every unencrypted page (and history snapshot) in memory and checks that no text, block or
/// canvas text is lost. Nothing is written.
#[test]
#[ignore]
fn migrate_real_notes() {
    let Ok(root) = std::env::var("SCRAWL_MIGRATE_CHECK") else { return };
    let mut checked = 0;
    let mut stack = vec![std::path::PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "json") || !(p.to_string_lossy().contains("/pages/") || p.to_string_lossy().contains("/history/")) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else { continue };
            let Ok(page) = serde_json::from_slice::<Page>(&bytes) else { continue };
            let mut q = page.clone();
            q.doc = None;
            q.ensure_doc();
            let squash = |s: String| s.split_whitespace().collect::<String>();
            let before = squash(page.blocks.iter().map(|b| crate::model::block_typst(b, None)).collect());
            let after = squash(q.blocks.iter().map(|b| crate::model::block_typst(b, None)).collect());
            if page.doc.is_none() {
                assert!(after.starts_with(&before), "{}: text changed\nbefore {before}\nafter  {after}", p.display());
            }
            for t in &page.texts {
                for line in t.text.lines().filter(|l| !l.trim().is_empty()) {
                    assert!(q.doc.as_ref().unwrap().plain_text().contains(line.trim()), "{}: lost canvas text {line:?}", p.display());
                }
            }
            assert_eq!(q.strokes, page.strokes);
            checked += 1;
        }
    }
    println!("checked {checked} pages and snapshots");
}
