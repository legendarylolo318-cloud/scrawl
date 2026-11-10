//! The page as a document: owns the text layout, caret and selection, and turns visible
//! paragraphs into pixmaps the ink canvas draws underneath the ink (same page coordinates).

use docview::{Colors, Fonts, Layout, Objects};
use notedoc::{Doc, ParaKind};
use std::collections::HashMap;
use std::sync::Arc;
use tiny_skia::Pixmap;

/// Where the text column sits on the page, in page units.
pub const MARGIN_X: f32 = 28.0;
pub const MARGIN_Y: f32 = 24.0;
const PT: f32 = 0.75;

pub struct AppObjects {
    pub math: Arc<render::MathCache>,
    pub blocks: crate::pagedoc::BlockCache,
    pub media: Vec<(String, Vec<u8>)>,
}

fn to_pixmap(r: &render::Rendered) -> Option<Pixmap> {
    Pixmap::from_vec(r.rgba_premul.clone(), tiny_skia::IntSize::from_wh(r.width, r.height)?)
}

impl AppObjects {
    fn block(&mut self, kind: &ParaKind, col_w: f32, scale: f32, color: [u8; 3]) -> Option<Arc<render::Rendered>> {
        let b = store::block_of(kind)?;
        match self.blocks.render(&b, &self.media, col_w, scale, color) {
            crate::pagedoc::Render::Ok(r) => Some(r),
            crate::pagedoc::Render::Err(_) => None,
        }
    }
}

impl Objects for AppObjects {
    fn inline_size(&mut self, typst: &str) -> (f32, f32) {
        match self.math.render(typst, 17.0 * PT, [0, 0, 0], 1.0 / PT) {
            Ok(r) => (r.width as f32, r.height as f32),
            Err(_) => (typst.chars().count() as f32 * 9.0 + 6.0, 22.0),
        }
    }
    fn inline_pixmap(&mut self, typst: &str, scale: f32, color: [u8; 3]) -> Option<Arc<Pixmap>> {
        let r = self.math.render(typst, 17.0 * PT, color, scale / PT).ok()?;
        to_pixmap(&r).map(Arc::new)
    }
    fn block_size(&mut self, kind: &ParaKind, col_w: f32) -> (f32, f32) {
        if matches!(kind, ParaKind::Rule) {
            return (col_w, 18.0);
        }
        match self.block(kind, col_w, 1.0, [0, 0, 0]) {
            Some(r) => (r.width as f32, r.height as f32),
            None => (col_w, 30.0),
        }
    }
    fn block_pixmap(&mut self, kind: &ParaKind, col_w: f32, scale: f32, color: [u8; 3]) -> Option<Arc<Pixmap>> {
        let r = self.block(kind, col_w, scale, color)?;
        to_pixmap(&r).map(Arc::new)
    }
}

pub struct DocEdit {
    pub ed: EditState,
    pub fonts: Option<Fonts>,
    pub layout: Layout,
    pub objs: AppObjects,
    cache: HashMap<(u64, u64, u32, [u8; 3]), Arc<Pixmap>>,
}

impl DocEdit {
    pub fn new(math: Arc<render::MathCache>) -> Self {
        Self { ed: EditState::default(), fonts: None, layout: Layout::new(), objs: AppObjects { math, blocks: Default::default(), media: vec![] }, cache: HashMap::new() }
    }

    /// Column width in page units for a canvas `canvas_w` logical px wide at zoom `zoom`.
    pub fn column_width(canvas_w: f32, zoom: f32) -> f32 {
        ((canvas_w / zoom.max(0.1)) - 2.0 * MARGIN_X).clamp(200.0, 840.0)
    }

    pub fn relayout(&mut self, doc: &Doc, col_w: f32) {
        let fonts = self.fonts.get_or_insert_with(Fonts::bundled);
        self.layout.update(fonts, doc, col_w, &mut self.objs);
    }

    /// Pixmaps of the paragraphs that intersect page rows `y0..y1`, at `phys` device px per page unit:
    /// (page x, page y, pixmap).
    pub fn visible(&mut self, doc: &Doc, y0: f32, y1: f32, phys: f32, colors: &Colors) -> Vec<(f32, f32, Arc<Pixmap>)> {
        let Some(fonts) = self.fonts.as_mut() else { return vec![] };
        let mut out = Vec::new();
        let col_w = self.layout.col_w;
        for (i, it) in self.layout.items.iter().enumerate() {
            let (top, bottom) = (it.top + MARGIN_Y, it.bottom() + MARGIN_Y);
            if bottom < y0 || top > y1 {
                continue;
            }
            let key = (it.id, it.key_hash(), phys.to_bits(), colors.text);
            if !self.cache.contains_key(&key) {
                if self.cache.len() > 400 {
                    self.cache.clear();
                }
                if let Some(pm) = doc.paras.get(i).and_then(|p| docview::render_item(fonts, it, &p.kind, col_w, phys, colors, &mut self.objs)) {
                    self.cache.insert(key, Arc::new(pm));
                }
            }
            if let Some(pm) = self.cache.get(&key) {
                out.push((MARGIN_X, top, pm.clone()));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Editing: keyboard, mouse, clipboard, undo. All of it works on `State` and never calls back into Slint.

use crate::{commands, dirty, State, Tool};
use notedoc::{Fmt, Pos, Sel, Style};
use std::time::{Duration, Instant};

/// Editing state kept next to the layout.
pub struct EditState {
    pub sel: Sel,
    pub undo: Vec<(Doc, Sel)>,
    pub redo: Vec<(Doc, Sel)>,
    /// Format for the next typed text when the caret has no selection (Ctrl+B with a collapsed caret).
    pub pending: Option<Style>,
    pub last_kind: &'static str,
    pub goal_x: Option<f32>,
    pub dragging: bool,
    pub click: Option<(Instant, f32, f32, u8)>,
    pub caret_on: bool,
}

impl Default for EditState {
    fn default() -> Self {
        Self { sel: Sel::default(), undo: vec![], redo: vec![], pending: None, last_kind: "", goal_x: None, dragging: false, click: None, caret_on: true }
    }
}

const UNDO_LIMIT: usize = 200;

/// The new editor is active: a document, the text-cursor tool and nothing modal in the way.
pub fn active(s: &State) -> bool {
    s.new_editor && s.tool == Tool::TextBox && s.overlay.is_empty() && !s.typing && s.ws.as_ref().is_some_and(|w| w.page.doc.is_some())
}

fn column(s: &State) -> f32 {
    DocEdit::column_width(s.size.0 as f32, s.view.scale)
}

pub fn relayout(s: &mut State) {
    let col = column(s);
    if let Some(doc) = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()) {
        s.dedit.relayout(doc, col);
    }
}

fn snapshot(s: &mut State, kind: &'static str) {
    let Some(doc) = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()) else { return };
    if kind.is_empty() || kind != s.dedit.ed.last_kind || s.dedit.ed.undo.is_empty() {
        s.dedit.ed.undo.push((doc.clone(), s.dedit.ed.sel));
        if s.dedit.ed.undo.len() > UNDO_LIMIT {
            s.dedit.ed.undo.remove(0);
        }
    }
    s.dedit.ed.redo.clear();
    s.dedit.ed.last_kind = kind;
}

fn after_edit(s: &mut State) {
    if let Some(ws) = &mut s.ws {
        ws.touch();
    }
    relayout(s);
    s.dedit.ed.caret_on = true;
    s.dirty |= dirty::CANVAS | dirty::DOC;
    scroll_to_caret(s);
}

/// Keep the caret on screen after it moved.
pub fn scroll_to_caret(s: &mut State) {
    let Some((_, y, h)) = s.dedit.layout.caret(s.dedit.ed.sel.head) else { return };
    let zoom = s.view.scale;
    let top = (y + MARGIN_Y) * zoom + s.view.oy;
    let bottom = (y + h + MARGIN_Y) * zoom + s.view.oy;
    let view_h = s.size.1 as f32;
    if top < 24.0 {
        s.view.oy += 24.0 - top;
    } else if bottom > view_h - 40.0 {
        s.view.oy -= bottom - (view_h - 40.0);
    }
    if s.view.oy > 0.0 {
        s.view.oy = 0.0;
    }
    s.dirty |= dirty::CANVAS;
}

fn doc_mut(s: &mut State) -> Option<(&mut Doc, &mut EditState)> {
    let doc = s.ws.as_mut()?.page.doc.as_mut()?;
    Some((doc, &mut s.dedit.ed))
}

fn set_sel(s: &mut State, head: Pos, extend: bool) {
    let ed = &mut s.dedit.ed;
    ed.sel.head = head;
    if !extend {
        ed.sel.anchor = head;
    }
    ed.pending = None;
    ed.last_kind = "";
    ed.caret_on = true;
    s.dirty |= dirty::CANVAS;
    scroll_to_caret(s);
}

fn edit(s: &mut State, kind: &'static str, f: impl FnOnce(&mut Doc, Sel, Option<Style>) -> Pos) {
    snapshot(s, kind);
    let Some((doc, ed)) = doc_mut(s) else { return };
    let pos = f(doc, ed.sel, ed.pending.take());
    ed.sel = Sel::caret(notedoc::clamp(doc, pos));
    ed.goal_x = None;
    after_edit(s);
}

fn type_text(s: &mut State, text: &str) {
    edit(s, if text.contains('\n') { "" } else { "type" }, |doc, sel, pending| {
        let (start, _) = sel.ordered();
        let style = pending.unwrap_or_else(|| notedoc::style_at(doc, start));
        let mut pos = notedoc::insert_text(doc, sel, text, style);
        if text == " " {
            if let Some(p) = notedoc::autoformat_space(doc, pos) {
                pos = p;
            }
        } else if text == "$" {
            if let Some(p) = notedoc::autoformat_dollar(doc, pos) {
                pos = p;
            }
        }
        pos
    });
}

fn vertical(s: &mut State, dir: f32, extend: bool, lines: f32) {
    let head = s.dedit.ed.sel.head;
    let Some((x, y, h)) = s.dedit.layout.caret(head) else { return };
    let gx = s.dedit.ed.goal_x.unwrap_or(x);
    let ty = if dir < 0.0 { y - 2.0 - (lines - 1.0) * h } else { y + h + 2.0 + (lines - 1.0) * h };
    let local_x = gx;
    let target = s.dedit.layout.hit(local_x, ty);
    let doc_end = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).map(notedoc::doc_end);
    let pos = match target {
        Some(_) if ty < 0.0 => Pos::default(),
        Some(p) if ty > s.dedit.layout.height - 40.0 => doc_end.unwrap_or(p),
        Some(p) => p,
        None => head,
    };
    set_sel(s, pos, extend);
    s.dedit.ed.goal_x = Some(gx);
}

pub fn copy_text(s: &State) -> Option<String> {
    let doc = s.ws.as_ref()?.page.doc.as_ref()?;
    let (a, b) = s.dedit.ed.sel.ordered();
    (a != b).then(|| notedoc::slice_plain(doc, a, b))
}

fn paste(s: &mut State) {
    let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
    if !text.is_empty() {
        type_text(s, &text.replace("\r\n", "\n"));
    }
}

fn toggle(s: &mut State, f: Fmt) {
    let sel = s.dedit.ed.sel;
    if sel.is_caret() {
        let Some(doc) = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()) else { return };
        let mut st = s.dedit.ed.pending.unwrap_or_else(|| notedoc::style_at(doc, sel.head));
        match f {
            Fmt::Bold => st.bold = !st.bold,
            Fmt::Italic => st.italic = !st.italic,
            Fmt::Underline => st.underline = !st.underline,
            Fmt::Code => st.code = !st.code,
        }
        s.dedit.ed.pending = Some(st);
        return;
    }
    snapshot(s, "");
    let (a, b) = sel.ordered();
    if let Some((doc, _)) = doc_mut(s) {
        notedoc::toggle_format(doc, a, b, f);
    }
    after_edit(s);
}

/// Keyboard. `text` is Slint's key text. Returns true when the key was used.
pub fn key(s: &mut State, text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> bool {
    if !active(s) {
        return false;
    }
    let mut chars = text.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        let special = ('\u{F700}'..='\u{F8FF}').contains(&ch);
        if !ctrl && !alt && !meta && !ch.is_control() && !special {
            type_text(s, text);
            return true;
        }
    }
    if text == "\u{19}" {
        return indent_key(s, -1);
    }
    let Some(combo) = commands::combo(text, ctrl, alt, shift, meta) else { return false };
    let extend = combo.contains("shift+");
    let base = combo.replace("shift+", "");
    let head = s.dedit.ed.sel.head;
    let sel0 = s.dedit.ed.sel;
    match base.as_str() {
        "left" => {
            let p = if !extend && !sel0.is_caret() { sel0.ordered().0 } else { with_doc(s, |d| notedoc::move_left(d, head)).unwrap_or(head) };
            set_sel(s, p, extend);
        }
        "right" => {
            let p = if !extend && !sel0.is_caret() { sel0.ordered().1 } else { with_doc(s, |d| notedoc::move_right(d, head)).unwrap_or(head) };
            set_sel(s, p, extend);
        }
        "ctrl+left" => {
            let p = with_doc(s, |d| notedoc::word_left(d, head)).unwrap_or(head);
            set_sel(s, p, extend);
        }
        "ctrl+right" => {
            let p = with_doc(s, |d| notedoc::word_right(d, head)).unwrap_or(head);
            set_sel(s, p, extend);
        }
        "up" => vertical(s, -1.0, extend, 1.0),
        "down" => vertical(s, 1.0, extend, 1.0),
        "pageup" => vertical(s, -1.0, extend, ((s.size.1 as f32 / s.view.scale) / 26.0 - 2.0).max(1.0)),
        "pagedown" => vertical(s, 1.0, extend, ((s.size.1 as f32 / s.view.scale) / 26.0 - 2.0).max(1.0)),
        "home" => {
            let (a, _) = s.dedit.layout.line_bounds(head);
            set_sel(s, Pos { para: head.para, off: a }, extend);
        }
        "end" => {
            let (_, b) = s.dedit.layout.line_bounds(head);
            set_sel(s, Pos { para: head.para, off: b }, extend);
        }
        "ctrl+home" => set_sel(s, Pos::default(), extend),
        "ctrl+end" => {
            let p = with_doc(s, notedoc::doc_end).unwrap_or(head);
            set_sel(s, p, extend);
        }
        "enter" => edit(s, "", |doc, sel, _| {
            let (st, e) = sel.ordered();
            let pos = notedoc::delete_range(doc, st, e);
            notedoc::enter(doc, pos)
        }),
        "backspace" => edit(s, "back", |doc, sel, _| notedoc::backspace(doc, sel)),
        "delete" => edit(s, "del", |doc, sel, _| notedoc::delete_forward(doc, sel)),
        "ctrl+backspace" => edit(s, "", |doc, sel, _| {
            let sel = if sel.is_caret() { Sel { anchor: notedoc::word_left(doc, sel.head), head: sel.head } } else { sel };
            notedoc::backspace(doc, sel)
        }),
        "ctrl+delete" => edit(s, "", |doc, sel, _| {
            let sel = if sel.is_caret() { Sel { anchor: sel.head, head: notedoc::word_right(doc, sel.head) } } else { sel };
            notedoc::delete_forward(doc, sel)
        }),
        "tab" => return indent_key(s, 1),
        "escape" => {
            let h = s.dedit.ed.sel.head;
            set_sel(s, h, false);
        }
        "ctrl+a" => {
            if let Some(sel) = with_doc(s, notedoc::select_all) {
                s.dedit.ed.sel = sel;
            }
            s.dirty |= dirty::CANVAS;
        }
        "ctrl+c" => {
            if let Some(t) = copy_text(s) {
                let _ = crate::actions::clipboard_text(&t);
            }
        }
        "ctrl+x" => {
            if let Some(t) = copy_text(s) {
                let _ = crate::actions::clipboard_text(&t);
                edit(s, "", |doc, sel, _| {
                    let (a, b) = sel.ordered();
                    notedoc::delete_range(doc, a, b)
                });
            }
        }
        "ctrl+v" => paste(s),
        "ctrl+b" => toggle(s, Fmt::Bold),
        "ctrl+i" => toggle(s, Fmt::Italic),
        "ctrl+u" => toggle(s, Fmt::Underline),
        _ => return false,
    }
    true
}

/// Tab / Shift+Tab: indent a list item, otherwise type spaces (Tab) or do nothing.
fn indent_key(s: &mut State, delta: i32) -> bool {
    let (a, b) = s.dedit.ed.sel.ordered();
    let is_list = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).is_some_and(|d| d.paras.get(a.para).is_some_and(|p| p.kind.is_list()));
    if is_list {
        snapshot(s, "");
        if let Some((doc, _)) = doc_mut(s) {
            notedoc::indent(doc, a, b, delta);
        }
        after_edit(s);
    } else if delta > 0 {
        type_text(s, "    ");
    }
    true
}

pub fn undo(s: &mut State) -> bool {
    let Some((doc, sel)) = s.dedit.ed.undo.pop() else { return false };
    let Some(cur) = s.ws.as_mut().and_then(|w| w.page.doc.as_mut()) else { return false };
    let now = std::mem::replace(cur, doc);
    s.dedit.ed.redo.push((now, s.dedit.ed.sel));
    s.dedit.ed.sel = sel;
    s.dedit.ed.last_kind = "";
    after_edit(s);
    true
}

pub fn redo(s: &mut State) -> bool {
    let Some((doc, sel)) = s.dedit.ed.redo.pop() else { return false };
    let Some(cur) = s.ws.as_mut().and_then(|w| w.page.doc.as_mut()) else { return false };
    let now = std::mem::replace(cur, doc);
    s.dedit.ed.undo.push((now, s.dedit.ed.sel));
    s.dedit.ed.sel = sel;
    s.dedit.ed.last_kind = "";
    after_edit(s);
    true
}

// ---- mouse -----------------------------------------------------------------------------------

fn doc_point(s: &State, x: f32, y: f32) -> (f32, f32) {
    let (px, py) = s.view.to_page(x, y);
    (px - MARGIN_X, py - MARGIN_Y)
}

/// Returns true when the click belongs to the document editor.
pub fn pointer_down(s: &mut State, x: f32, y: f32) -> bool {
    if !active(s) {
        return false;
    }
    relayout(s);
    let (lx, ly) = doc_point(s, x, y);
    let Some(pos) = s.dedit.layout.hit(lx, ly) else { return true };
    // a click on a checkbox toggles it
    let on_check = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).and_then(|d| d.paras.get(pos.para)).is_some_and(|p| matches!(p.kind, ParaKind::Check { .. })) && lx < 30.0 && lx > -4.0;
    if on_check {
        snapshot(s, "");
        if let Some((doc, _)) = doc_mut(s) {
            notedoc::toggle_check(doc, pos.para);
        }
        after_edit(s);
        return true;
    }
    let now = Instant::now();
    let count = match s.dedit.ed.click {
        Some((t, cx, cy, n)) if now.duration_since(t) < Duration::from_millis(450) && (cx - x).abs() < 8.0 && (cy - y).abs() < 8.0 => (n % 3) + 1,
        _ => 1,
    };
    s.dedit.ed.click = Some((now, x, y, count));
    let sel = match count {
        2 => s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).map(|d| notedoc::word_range(d, pos)).map(|(a, b)| Sel { anchor: a, head: b }),
        3 => s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).map(|d| notedoc::para_range(d, pos)).map(|(a, b)| Sel { anchor: a, head: b }),
        _ => Some(Sel::caret(pos)),
    };
    if let Some(sel) = sel {
        s.dedit.ed.sel = sel;
    }
    s.dedit.ed.dragging = count == 1;
    s.dedit.ed.pending = None;
    s.dedit.ed.last_kind = "";
    s.dedit.ed.goal_x = None;
    s.dedit.ed.caret_on = true;
    s.dirty |= dirty::CANVAS;
    true
}

pub fn pointer_move(s: &mut State, x: f32, y: f32) -> bool {
    if !s.dedit.ed.dragging || !active(s) {
        return false;
    }
    let (lx, ly) = doc_point(s, x, y);
    if let Some(pos) = s.dedit.layout.hit(lx, ly) {
        s.dedit.ed.sel.head = pos;
        s.dirty |= dirty::CANVAS;
        scroll_to_caret(s);
    }
    true
}

pub fn pointer_up(s: &mut State) -> bool {
    let was = s.dedit.ed.dragging;
    s.dedit.ed.dragging = false;
    was
}

/// Caret and selection rectangles in page coordinates (for the overlay).
pub type Rects = Vec<(f32, f32, f32, f32)>;

pub fn overlay_geometry(s: &State) -> (Rects, Option<(f32, f32, f32)>) {
    if !active(s) {
        return (vec![], None);
    }
    let sel = s.dedit.ed.sel;
    let rects = if sel.is_caret() { vec![] } else { s.dedit.layout.selection_rects(sel.anchor, sel.head).into_iter().map(|(x, y, w, h)| (x + MARGIN_X, y + MARGIN_Y, w, h)).collect() };
    let caret = (sel.is_caret() && s.dedit.ed.caret_on).then(|| s.dedit.layout.caret(sel.head)).flatten().map(|(x, y, h)| (x + MARGIN_X, y + MARGIN_Y, h));
    (rects, caret)
}

fn with_doc<R>(s: &State, f: impl FnOnce(&Doc) -> R) -> Option<R> {
    s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).map(f)
}
