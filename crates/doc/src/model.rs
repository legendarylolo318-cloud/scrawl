use layout::Expr;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Link {
    /// `[[Page title]]`
    Page(String),
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Run {
    Text {
        text: String,
        #[serde(default)]
        style: Style,
        #[serde(default)]
        link: Option<Link>,
    },
    /// Inline math as Typst math source (what goes between the `$`).
    Math { typst: String },
}

impl Run {
    pub fn plain(text: &str) -> Run {
        Run::Text { text: text.into(), style: Style::default(), link: None }
    }
    pub fn styled(text: &str, style: Style) -> Run {
        Run::Text { text: text.into(), style, link: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalloutKind {
    Definition,
    Theorem,
    Example,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParaKind {
    Body,
    /// Level 1 to 3.
    Heading(u8),
    Bullet { indent: u8 },
    Numbered { indent: u8 },
    Check { done: bool },
    Quote,
    Callout(CalloutKind),
    MathDisplay { expr: Expr, source: Option<String> },
    Image { file: String, x: f32, y: f32, w: f32, h: f32, crop: Option<[f32; 4]> },
    Plot { functions: Vec<String>, x_range: (f32, f32), y_range: Option<(f32, f32)> },
    Rule,
    /// Typst markup we do not edit natively (tables, code fences, ...), kept exactly as written.
    Markup(String),
}

impl ParaKind {
    /// Paragraph kinds whose content is `runs` (the caret can be placed in them).
    pub fn is_text(&self) -> bool {
        matches!(self, ParaKind::Body | ParaKind::Heading(_) | ParaKind::Bullet { .. } | ParaKind::Numbered { .. } | ParaKind::Check { .. } | ParaKind::Quote | ParaKind::Callout(_))
    }
    pub fn is_list(&self) -> bool {
        matches!(self, ParaKind::Bullet { .. } | ParaKind::Numbered { .. } | ParaKind::Check { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Para {
    pub id: u64,
    pub kind: ParaKind,
    #[serde(default)]
    pub runs: Vec<Run>,
}

impl Para {
    pub fn text_len(&self) -> usize {
        self.runs.iter().map(run_len).sum()
    }
    /// Plain text of the runs (inline math shown as its Typst source between `$`).
    pub fn plain(&self) -> String {
        let mut s = String::new();
        for r in &self.runs {
            match r {
                Run::Text { text, .. } => s += text,
                Run::Math { typst } => s += &format!("${typst}$"),
            }
        }
        s
    }
}

/// Length of a run in carets positions: characters for text, one for an inline math object.
pub fn run_len(r: &Run) -> usize {
    match r {
        Run::Text { text, .. } => text.chars().count(),
        Run::Math { .. } => 1,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub paras: Vec<Para>,
    #[serde(default)]
    pub next_id: u64,
}

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    pub fn fresh_id(&mut self) -> u64 {
        let max = self.paras.iter().map(|p| p.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max) + 1;
        self.next_id
    }

    pub fn push(&mut self, kind: ParaKind, runs: Vec<Run>) -> u64 {
        let id = self.fresh_id();
        self.paras.push(Para { id, kind, runs: normalize(runs) });
        id
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.paras.iter().position(|p| p.id == id)
    }

    /// All text, one paragraph per line (for search and word count).
    pub fn plain_text(&self) -> String {
        self.paras.iter().filter(|p| p.kind.is_text()).map(Para::plain).collect::<Vec<_>>().join("\n")
    }

    pub fn word_count(&self) -> usize {
        self.plain_text().split_whitespace().count()
    }

    pub fn is_empty(&self) -> bool {
        self.paras.iter().all(|p| p.runs.is_empty() && p.kind.is_text() && p.kind == ParaKind::Body)
    }
}

/// Merge neighbouring text runs with the same formatting and drop empty ones.
pub fn normalize(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        if let Run::Text { text, .. } = &r {
            if text.is_empty() {
                continue;
            }
        }
        if let (Some(Run::Text { text: pt, style: ps, link: pl }), Run::Text { text, style, link }) = (out.last_mut(), &r) {
            if ps == style && pl == link {
                pt.push_str(text);
                continue;
            }
        }
        out.push(r);
    }
    out
}

/// A position in the document: paragraph index and offset inside it (characters; inline math counts 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub para: usize,
    pub off: usize,
}

impl Pos {
    pub fn new(para: usize, off: usize) -> Pos {
        Pos { para, off }
    }
}

/// Caret and selection: `head` is where the caret is, `anchor` where the selection started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sel {
    pub anchor: Pos,
    pub head: Pos,
}

impl Sel {
    pub fn caret(p: Pos) -> Sel {
        Sel { anchor: p, head: p }
    }
    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }
    /// (start, end) in document order.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}
