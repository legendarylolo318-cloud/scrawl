use layout::Expr;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paper {
    /// "blank" | "lined" | "grid" | "dots" | "engineering" | "polar"
    pub kind: String,
    pub spacing: f32,
    pub color: [u8; 4],
}

impl Default for Paper {
    fn default() -> Self {
        Self { kind: "lined".into(), spacing: 32.0, color: [0x41, 0x48, 0x68, 0xff] }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PageMode {
    #[default]
    Infinite,
    Letter,
    A4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalloutKind {
    Definition,
    Theorem,
    Example,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    /// A math expression; `source` holds raw Typst/LaTeX the user typed when it did not parse.
    Math { expr: Expr, source: Option<String> },
    /// Rich text as Typst markup (may contain inline `$math$`).
    Text { markup: String },
    Callout { kind: CalloutKind, markup: String },
    Image { file: String, x: f32, y: f32, w: f32, h: f32 },
    Plot { functions: Vec<String>, x_range: (f32, f32), y_range: Option<(f32, f32)> },
    Rule,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub course: String,
    /// ISO date, auto-filled on creation.
    pub date: String,
    pub lecture: Option<u32>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub paper: Paper,
    pub mode: PageMode,
    pub strokes: Vec<ink::Stroke>,
    pub blocks: Vec<Block>,
    /// Imported PDF used as the page background (file name inside the notebook's media folder).
    pub pdf_background: Option<String>,
    pub modified_ms: u64,
}

impl Page {
    pub fn new(id: &str, title: &str, date: &str) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            course: String::new(),
            date: date.into(),
            lecture: None,
            tags: vec![],
            favorite: false,
            paper: Paper::default(),
            mode: PageMode::Infinite,
            strokes: vec![],
            blocks: vec![],
            pdf_background: None,
            modified_ms: 0,
        }
    }

    /// Human-readable Typst source for this page (the `.typ` file next to the JSON sidecar).
    pub fn to_typst(&self) -> String {
        let mut s = format!("= {}\n", self.title);
        if !self.course.is_empty() || self.lecture.is_some() {
            s += &format!("_{}", self.course);
            if let Some(l) = self.lecture {
                s += &format!(" · lecture {l}");
            }
            s += &format!(" · {}_\n", self.date);
        }
        for b in &self.blocks {
            s += "\n";
            match b {
                Block::Math { expr, source } => match source {
                    Some(raw) if expr.is_empty() => s += &format!("$ {raw} $\n"),
                    _ => s += &format!("$ {} $\n", expr.to_typst()),
                },
                Block::Text { markup } => s += &format!("{markup}\n"),
                Block::Callout { kind, markup } => {
                    let (name, fill) = match kind {
                        CalloutKind::Definition => ("Definition", "rgb(\"#e0e7ff\")"),
                        CalloutKind::Theorem => ("Theorem", "rgb(\"#dcfce7\")"),
                        CalloutKind::Example => ("Example", "rgb(\"#fef9c3\")"),
                        CalloutKind::Warning => ("Warning", "rgb(\"#fee2e2\")"),
                    };
                    s += &format!("#block(fill: {fill}, inset: 8pt, radius: 4pt)[*{name}.* {markup}]\n");
                }
                Block::Image { file, w, .. } => s += &format!("#image(\"{file}\", width: {w}pt)\n"),
                Block::Plot { functions, .. } => s += &format!("// plot: {}\n", functions.join(", ")),
                Block::Rule => s += "#line(length: 100%)\n",
            }
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageRef {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub name: String,
    pub pages: Vec<PageRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notebook {
    pub id: String,
    pub name: String,
    pub sections: Vec<Section>,
}

impl Notebook {
    pub fn section_mut(&mut self, id: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.id == id)
    }

    pub fn add_section(&mut self, id: &str, name: &str) {
        self.sections.push(Section { id: id.into(), name: name.into(), pages: vec![] });
    }

    pub fn add_page(&mut self, section: &str, page: PageRef) -> bool {
        match self.section_mut(section) {
            Some(s) => {
                s.pages.push(page);
                true
            }
            None => false,
        }
    }

    pub fn find_page(&self, page: &str) -> Option<(usize, usize)> {
        self.sections.iter().enumerate().find_map(|(si, s)| s.pages.iter().position(|p| p.id == page).map(|pi| (si, pi)))
    }

    /// Drag-and-drop: move a page to `index` in `to_section` (works within and across sections).
    pub fn move_page(&mut self, page: &str, to_section: &str, index: usize) -> bool {
        let Some((si, pi)) = self.find_page(page) else { return false };
        if !self.sections.iter().any(|s| s.id == to_section) {
            return false;
        }
        let p = self.sections[si].pages.remove(pi);
        let dest = self.section_mut(to_section).unwrap();
        dest.pages.insert(index.min(dest.pages.len()), p);
        true
    }

    pub fn move_section(&mut self, section: &str, index: usize) -> bool {
        let Some(i) = self.sections.iter().position(|s| s.id == section) else { return false };
        let s = self.sections.remove(i);
        self.sections.insert(index.min(self.sections.len()), s);
        true
    }

    pub fn rename_page(&mut self, page: &str, title: &str) {
        for s in &mut self.sections {
            for p in &mut s.pages {
                if p.id == page {
                    p.title = title.into();
                }
            }
        }
    }

    pub fn remove_page(&mut self, page: &str) -> Option<(String, PageRef)> {
        let (si, pi) = self.find_page(page)?;
        let sec = self.sections[si].id.clone();
        Some((sec, self.sections[si].pages.remove(pi)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nb() -> Notebook {
        let mut n = Notebook { id: "n".into(), name: "Calc".into(), sections: vec![] };
        n.add_section("s1", "Limits");
        n.add_section("s2", "Series");
        for (s, id) in [("s1", "a"), ("s1", "b"), ("s1", "c"), ("s2", "d")] {
            n.add_page(s, PageRef { id: id.into(), title: id.into() });
        }
        n
    }

    #[test]
    fn drag_and_drop_reorder_and_move() {
        let mut n = nb();
        assert!(n.move_page("c", "s1", 0));
        assert_eq!(n.sections[0].pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        assert!(n.move_page("a", "s2", 1));
        assert_eq!(n.sections[1].pages.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["d", "a"]);
        assert!(!n.move_page("zzz", "s2", 0));
        assert!(!n.move_page("a", "nope", 0));
        assert!(n.move_section("s2", 0));
        assert_eq!(n.sections[0].id, "s2");
    }

    #[test]
    fn typst_source_contains_title_and_math() {
        let mut p = Page::new("p", "Chain rule", "2026-11-09");
        p.course = "Calc 1".into();
        p.lecture = Some(4);
        p.blocks.push(Block::Math { expr: Expr::Sup(Box::new(Expr::sym("x")), Box::new(Expr::num("2"))), source: None });
        let t = p.to_typst();
        assert!(t.starts_with("= Chain rule"));
        assert!(t.contains("Calc 1 · lecture 4 · 2026-11-09"));
        assert!(t.contains("$ x^2 $"));
    }
}
