use crate::Objects;
use cosmic_text::{fontdb, Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Shaping, Style as FStyle, SwashCache, Weight};
use notedoc::{Doc, Para, ParaKind, Pos, Run};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

pub const BODY_FONT: &str = "Libertinus Serif";
pub const MONO_FONT: &str = "DejaVu Sans Mono";
/// `Attrs::metadata` bits, read back at render time so colours can change without a re-layout.
pub const META_UNDERLINE: usize = 1;
pub const META_LINK: usize = 2;
pub const META_CODE: usize = 4;
pub const META_MATH: usize = 8;

pub struct Fonts {
    pub fs: FontSystem,
    pub swash: SwashCache,
    /// Advance of the math placeholder (a HAIR SPACE) in the body font, in em.
    pub ph_adv: f32,
}

impl Fonts {
    /// Only the bundled fonts: layout is the same on every machine.
    pub fn bundled() -> Fonts {
        let mut db = fontdb::Database::new();
        for data in typst_assets::fonts() {
            db.load_font_data(data.to_vec());
        }
        db.set_serif_family(BODY_FONT);
        db.set_sans_serif_family(BODY_FONT);
        db.set_monospace_family(MONO_FONT);
        let mut fs = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let mut b = Buffer::new(&mut fs, Metrics::new(100.0, 120.0));
        let a = Attrs::new().family(Family::Name(BODY_FONT));
        b.set_size(Some(1000.0), None);
        b.set_rich_text([("\u{200A}", a.clone())], &a, Shaping::Advanced, None);
        b.shape_until_scroll(&mut fs, false);
        let ph_adv = b.layout_runs().next().and_then(|r| r.glyphs.first().map(|g| g.w / 100.0)).filter(|w| *w >= 0.0).unwrap_or(0.1);
        Fonts { fs, swash: SwashCache::new(), ph_adv }
    }
}

/// Sizes of one paragraph kind, in logical px at zoom 1.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub size: f32,
    pub line: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    /// Left indent of the text (room for list markers and quote bars).
    pub indent: f32,
    pub before: f32,
    pub after: f32,
}

pub fn look(kind: &ParaKind) -> Look {
    let body = Look { size: 17.0, line: 26.0, bold: false, italic: false, mono: false, indent: 0.0, before: 0.0, after: 10.0 };
    match kind {
        ParaKind::Heading(1) => Look { size: 30.0, line: 38.0, bold: true, before: 14.0, after: 8.0, ..body },
        ParaKind::Heading(2) => Look { size: 24.0, line: 32.0, bold: true, before: 10.0, after: 6.0, ..body },
        ParaKind::Heading(_) => Look { size: 20.0, line: 28.0, bold: true, before: 8.0, after: 4.0, ..body },
        ParaKind::Bullet { indent } | ParaKind::Numbered { indent } => Look { indent: 30.0 + 26.0 * *indent as f32, after: 3.0, ..body },
        ParaKind::Check { .. } => Look { indent: 34.0, after: 3.0, ..body },
        ParaKind::Quote => Look { italic: true, indent: 22.0, ..body },
        ParaKind::Callout(_) => Look { indent: 18.0, ..body },
        _ => body,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Text,
    Object,
}

#[derive(Default, Clone)]
pub(crate) struct TextMap {
    pub text: String,
    /// Byte index in `text` for every model offset 0..=len.
    pub offs: Vec<usize>,
    /// (byte start of the placeholder, Typst source, logical size)
    pub math: Vec<(usize, String, (f32, f32))>,
}

impl TextMap {
    pub fn byte(&self, off: usize) -> usize {
        self.offs[off.min(self.offs.len() - 1)]
    }
    pub fn off(&self, byte: usize) -> usize {
        let i = self.offs.partition_point(|&b| b <= byte).saturating_sub(1);
        // inside an inline math placeholder: snap to the nearer edge
        match self.offs.get(i + 1) {
            Some(&next) if byte > self.offs[i] && byte < next => {
                if byte - self.offs[i] < next - byte {
                    i
                } else {
                    i + 1
                }
            }
            _ => i,
        }
    }
}

pub struct Item {
    pub id: u64,
    pub kind: ItemKind,
    pub top: f32,
    pub height: f32,
    pub look: Look,
    /// "•", "3.", "☐" ... drawn left of the first line.
    pub marker: String,
    /// Object size (logical px) for `ItemKind::Object`.
    pub obj: (f32, f32),
    pub(crate) key: u64,
    pub(crate) buf: Option<Buffer>,
    pub(crate) map: TextMap,
}

impl Item {
    pub fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// Changes whenever the paragraph content or the column width changes.
    pub fn key_hash(&self) -> u64 {
        self.key
    }
}

#[derive(Default)]
pub struct Layout {
    pub col_w: f32,
    pub items: Vec<Item>,
    pub height: f32,
    /// Paragraphs laid out since creation (tests use it to check caching).
    pub laid_out: usize,
}

fn hash_of(p: &Para, marker: &str, col_w: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", p).hash(&mut h);
    marker.hash(&mut h);
    col_w.to_bits().hash(&mut h);
    h.finish()
}

fn marker_for(doc: &Doc, i: usize) -> String {
    match &doc.paras[i].kind {
        ParaKind::Bullet { indent } => if indent % 2 == 0 { "•" } else { "◦" }.into(),
        ParaKind::Check { done } => if *done { "☑" } else { "☐" }.into(),
        ParaKind::Numbered { indent } => {
            let mut n = 1;
            let mut j = i;
            while j > 0 {
                j -= 1;
                match &doc.paras[j].kind {
                    ParaKind::Numbered { indent: k } if k == indent => n += 1,
                    ParaKind::Numbered { indent: k } if k > indent => {}
                    _ => break,
                }
            }
            format!("{n}.")
        }
        _ => String::new(),
    }
}

impl Layout {
    pub fn new() -> Layout {
        Layout::default()
    }

    /// Lay out (or re-use) every paragraph for a column `col_w` px wide.
    pub fn update(&mut self, fonts: &mut Fonts, doc: &Doc, col_w: f32, objs: &mut dyn Objects) {
        let col_w = col_w.max(80.0);
        let mut old: HashMap<u64, Item> = self.items.drain(..).map(|it| (it.id, it)).collect();
        let mut y = 0.0f32;
        let mut prev_after = 0.0f32;
        for (i, p) in doc.paras.iter().enumerate() {
            let marker = marker_for(doc, i);
            let key = hash_of(p, &marker, col_w);
            let mut item = match old.remove(&p.id) {
                Some(it) if it.key == key => it,
                _ => {
                    self.laid_out += 1;
                    build_item(fonts, p, &marker, key, col_w, objs)
                }
            };
            let gap = if i == 0 { 0.0 } else { prev_after.max(item.look.before) };
            item.top = y + gap;
            y = item.top + item.height;
            prev_after = item.look.after;
            self.items.push(item);
        }
        self.col_w = col_w;
        self.height = y + 40.0;
    }

    pub fn item_at_y(&self, y: f32) -> Option<usize> {
        if self.items.is_empty() {
            return None;
        }
        let i = self.items.partition_point(|it| it.bottom() + it.look.after / 2.0 <= y);
        Some(i.min(self.items.len() - 1))
    }

    /// The document position under a point (document coordinates, column-relative).
    pub fn hit(&self, x: f32, y: f32) -> Option<Pos> {
        let i = self.item_at_y(y)?;
        let it = &self.items[i];
        Some(match (&it.kind, &it.buf) {
            (ItemKind::Text, Some(buf)) => {
                let c = buf.hit((x - it.look.indent).max(0.0), (y - it.top).clamp(0.0, it.height - 0.5));
                let byte = c.map_or(0, |c| c.index);
                Pos { para: i, off: it.map.off(byte) }
            }
            _ => Pos { para: i, off: usize::from(x > it.obj.0 / 2.0) },
        })
    }

    /// Caret rectangle (x, y, height) in column coordinates.
    pub fn caret(&self, pos: Pos) -> Option<(f32, f32, f32)> {
        let it = self.items.get(pos.para)?;
        match (&it.kind, &it.buf) {
            (ItemKind::Text, Some(buf)) => {
                let byte = it.map.byte(pos.off);
                let cur = Cursor::new(0, byte);
                let line_h = it.look.line;
                // a run per wrapped line; an empty paragraph has one run without glyphs
                for run in buf.layout_runs() {
                    if let Some(x) = run.cursor_position(&cur) {
                        return Some((it.look.indent + x, it.top + run.line_top, run.line_height));
                    }
                }
                Some((it.look.indent, it.top, line_h))
            }
            _ => {
                let x = if pos.off == 0 { 0.0 } else { it.obj.0 };
                Some((x, it.top, it.height))
            }
        }
    }

    /// Highlight rectangles (x, y, w, h) for the selection from `a` to `b` (any order).
    pub fn selection_rects(&self, a: Pos, b: Pos) -> Vec<(f32, f32, f32, f32)> {
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        let mut out = Vec::new();
        for i in s.para..=e.para.min(self.items.len().saturating_sub(1)) {
            let Some(it) = self.items.get(i) else { break };
            match (&it.kind, &it.buf) {
                (ItemKind::Text, Some(buf)) => {
                    let from = if i == s.para { s.off } else { 0 };
                    let to = if i == e.para { e.off } else { usize::MAX };
                    let b0 = it.map.byte(from);
                    let b1 = if to == usize::MAX { it.map.text.len() } else { it.map.byte(to) };
                    for run in buf.layout_runs() {
                        for (x, w) in run.highlight(Cursor::new(0, b0), Cursor::new(0, b1)) {
                            out.push((it.look.indent + x, it.top + run.line_top, w.max(1.0), run.line_height));
                        }
                        // the paragraph break inside a multi-paragraph selection shows as a little extra
                        if i != e.para && run.glyphs.last().is_none() {
                            out.push((it.look.indent, it.top + run.line_top, 6.0, run.line_height));
                        }
                    }
                }
                _ => out.push((0.0, it.top, it.obj.0.max(10.0), it.height)),
            }
        }
        out
    }

    /// Start and end offsets of the visual line containing `pos` (Home / End).
    pub fn line_bounds(&self, pos: Pos) -> (usize, usize) {
        let Some(it) = self.items.get(pos.para) else { return (0, 0) };
        let Some(buf) = &it.buf else { return (0, pos.off) };
        let byte = it.map.byte(pos.off);
        let mut best = (0, it.map.text.len());
        for run in buf.layout_runs() {
            let (Some(first), Some(last)) = (run.glyphs.first(), run.glyphs.last()) else { continue };
            let (lo, hi) = (first.start.min(last.start), first.end.max(last.end));
            if byte >= lo && byte <= hi {
                best = (lo, hi);
                // the end of a wrapped line is also the start of the next: prefer the one that starts here
                if byte < hi || hi == it.map.text.len() {
                    break;
                }
            }
        }
        (it.map.off(best.0), it.map.off(best.1))
    }

    /// Number of laid-out lines of a paragraph.
    pub fn lines_in(&self, para: usize) -> usize {
        self.items.get(para).and_then(|it| it.buf.as_ref()).map_or(1, |b| b.layout_runs().count().max(1))
    }
}

fn build_item(fonts: &mut Fonts, p: &Para, marker: &str, key: u64, col_w: f32, objs: &mut dyn Objects) -> Item {
    let lk = look(&p.kind);
    if !p.kind.is_text() {
        let (w, h) = objs.block_size(&p.kind, col_w);
        return Item { id: p.id, kind: ItemKind::Object, top: 0.0, height: h.max(8.0), look: lk, marker: String::new(), obj: (w, h), key, buf: None, map: TextMap::default() };
    }
    let family = if lk.mono { Family::Name(MONO_FONT) } else { Family::Name(BODY_FONT) };
    let base = Attrs::new().family(family).weight(if lk.bold { Weight::BOLD } else { Weight::NORMAL }).style(if lk.italic { FStyle::Italic } else { FStyle::Normal });
    let mut map = TextMap { offs: vec![0], ..TextMap::default() };
    let mut spans: Vec<(String, Attrs)> = Vec::new();
    for r in &p.runs {
        match r {
            Run::Text { text, style, link } => {
                let mut a = base.clone();
                if style.bold {
                    a = a.weight(Weight::BOLD);
                }
                if style.italic {
                    a = a.style(FStyle::Italic);
                }
                let mut meta = 0;
                if style.underline {
                    meta |= META_UNDERLINE;
                }
                if link.is_some() {
                    meta |= META_LINK | META_UNDERLINE;
                }
                if style.code {
                    meta |= META_CODE;
                    a = a.family(Family::Name(MONO_FONT));
                }
                for ch in text.chars() {
                    map.text.push(ch);
                    map.offs.push(map.text.len());
                }
                spans.push((text.clone(), a.metadata(meta)));
            }
            Run::Math { typst } => {
                let (w, h) = objs.inline_size(typst);
                // a hair space plus letter spacing takes exactly the formula's width
                let start = map.text.len();
                map.text.push('\u{200A}');
                map.offs.push(map.text.len());
                map.math.push((start, typst.clone(), (w, h)));
                spans.push(("\u{200A}".into(), base.clone().metadata(META_MATH).letter_spacing((w / lk.size - fonts.ph_adv).max(0.0)).metrics(Metrics::new(lk.size, (h + 4.0).max(lk.line)))));
            }
        }
    }
    let mut buf = Buffer::new(&mut fonts.fs, Metrics::new(lk.size, lk.line));
    buf.set_size(Some((col_w - lk.indent).max(40.0)), None);
    buf.set_rich_text(spans.iter().map(|(s, a)| (s.as_str(), a.clone())), &base, Shaping::Advanced, None);
    buf.shape_until_scroll(&mut fonts.fs, false);
    let height = buf.layout_runs().last().map_or(lk.line, |r| r.line_top + r.line_height).max(lk.line);
    Item { id: p.id, kind: ItemKind::Text, top: 0.0, height, look: lk, marker: marker.into(), obj: (0.0, 0.0), key, buf: Some(buf), map }
}
