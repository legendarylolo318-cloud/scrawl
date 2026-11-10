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
    Image {
        file: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// Fractions cut from [left, top, right, bottom].
        #[serde(default)]
        crop: Option<[f32; 4]>,
    },
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
    /// Audio recorded while writing, with stroke-id -> offset marks for playback sync.
    #[serde(default)]
    pub audio: Option<AudioClip>,
    /// Text typed straight onto the canvas with the Text tool.
    #[serde(default)]
    pub texts: Vec<CanvasText>,
    /// Ink mode the page opens in: "math", "text" or "drawing" (None = math).
    #[serde(default)]
    pub ink_mode: Option<String>,
    /// The page as a document (paragraphs, styled text, objects). `None` for pages never opened in the
    /// document editor; `blocks` is then the content. Once set, `blocks` is rebuilt from it on save.
    #[serde(default)]
    pub doc: Option<notedoc::Doc>,
    /// Ink that follows the text: stroke -> paragraph it was written next to (see `InkAnchor`).
    #[serde(default)]
    pub ink_anchors: Vec<InkAnchor>,
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
            audio: None,
            texts: vec![],
            ink_mode: None,
            doc: None,
            ink_anchors: vec![],
            modified_ms: 0,
        }
    }

    /// Human-readable Typst source for this page (the `.typ` file next to the JSON sidecar).
    pub fn to_typst(&self) -> String {
        let mut s = self.header_typst();
        for b in &self.blocks {
            s += "\n";
            s += &block_typst(b, None);
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
/// Escape plain text (titles, course names) for Typst markup so it is shown literally.
pub fn typst_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '#' | '$' | '*' | '_' | '`' | '<' | '>' | '@' | '/' | '[' | ']' | '~' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape a string for use inside a Typst string literal.
pub fn typst_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Typst for one block. `plot_file`: name of a pre-rendered plot image, if the caller has one.
pub fn block_typst(b: &Block, plot_file: Option<&str>) -> String {
    match b {
        Block::Math { expr, source } => match source {
            Some(raw) if expr.is_empty() => format!("$ {raw} $\n"),
            _ => format!("$ {} $\n", expr.to_typst()),
        },
        Block::Text { markup } => format!("{markup}\n"),
        Block::Callout { kind, markup } => {
            let (name, fill) = match kind {
                CalloutKind::Definition => ("Definition", "rgb(\"#e0e7ff\")"),
                CalloutKind::Theorem => ("Theorem", "rgb(\"#dcfce7\")"),
                CalloutKind::Example => ("Example", "rgb(\"#fef9c3\")"),
                CalloutKind::Warning => ("Warning", "rgb(\"#fee2e2\")"),
            };
            format!("#block(fill: {fill}, inset: 8pt, radius: 4pt, width: 100%)[*{name}.* {markup}]\n")
        }
        Block::Image { file, w, h, crop, .. } => match crop {
            // w, h: displayed size of the whole image; crop = [left, top, right, bottom] fractions removed
            Some([l, t, r, btm]) if (*l + *r) < 0.95 && (*t + *btm) < 0.95 => format!(
                "#box(width: {:.2}pt, height: {:.2}pt, clip: true)[#move(dx: -{:.2}pt, dy: -{:.2}pt)[#image(\"{}\", width: {w}pt, height: {h}pt)]]\n",
                w * (1.0 - l - r),
                h * (1.0 - t - btm),
                w * l,
                h * t,
                typst_str(file)
            ),
            _ => format!("#image(\"{}\", width: {w}pt)\n", typst_str(file)),
        },
        Block::Plot { functions, .. } => match plot_file {
            Some(f) => format!("#image(\"{}\", width: 100%)\n", typst_str(f)),
            None => format!("// plot: {}\n", functions.join(", ").replace('\n', " ")),
        },
        Block::Rule => "#line(length: 100%)\n".into(),
    }
}

impl Page {
    /// Typst page setup for the fixed page modes.
    pub fn paper_typst(&self) -> &'static str {
        match self.mode {
            PageMode::Letter => "#set page(paper: \"us-letter\", margin: 18mm)\n",
            PageMode::A4 | PageMode::Infinite => "#set page(paper: \"a4\", margin: 18mm)\n",
        }
    }

    /// Title and metadata header (escaped).
    pub fn header_typst(&self) -> String {
        let mut s = format!("= {}\n", typst_escape(&self.title));
        if !self.course.is_empty() || self.lecture.is_some() {
            s += &format!("_{}", typst_escape(&self.course));
            if let Some(l) = self.lecture {
                s += &format!(" · lecture {l}");
            }
            s += &format!(" · {}_\n", typst_escape(&self.date));
        }
        s
    }
}

/// A recording attached to a page (Ogg Opus file in the notebook's media folder).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AudioClip {
    pub file: String,
    /// (stroke id, milliseconds into the recording when the stroke started).
    pub marks: Vec<(u64, u32)>,
    pub duration_ms: u32,
}

#[cfg(test)]
mod escape_tests {
    use super::*;

    #[test]
    fn hostile_titles_are_shown_literally() {
        let p = Page::new("p", "#read(\"/etc/passwd\") $x$ *b* _i_ // c", "2026-01-01");
        let h = p.header_typst();
        assert!(h.contains("\\#read"), "{h}");
        assert!(h.contains("\\$x\\$") && h.contains("\\/\\/"), "{h}");
    }
}

/// A piece of typed text placed on the canvas (page coordinates, size in page units).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasText {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub color: [u8; 4],
}

impl CanvasText {
    /// Rough bounding box (x0, y0, x1, y1) for hit tests; good enough for picking and erasing.
    pub fn bbox(&self) -> (f32, f32, f32, f32) {
        let lines: Vec<&str> = self.text.lines().collect();
        let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
        (self.x, self.y, self.x + longest * self.size * 0.55, self.y + lines.len().max(1) as f32 * self.size * 1.3)
    }
}

/// A stroke written next to a paragraph: it keeps its distance `dy` (page units) below the paragraph's
/// top, so the ink stays next to the right text when earlier paragraphs grow or shrink.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkAnchor {
    pub stroke: u64,
    pub para: u64,
    pub dy: f32,
}
