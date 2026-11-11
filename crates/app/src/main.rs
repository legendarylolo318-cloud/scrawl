#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod commands;
mod docedit;
mod graphview;
mod menus;
mod notegraph;
mod vault;
mod handwriting;
mod headless;
mod pagedoc;
mod raster;
mod selftest;
mod session;
mod settings;
mod symbols;
mod tablet;
mod workspace;

use graphview::GraphView;
use ink::{is_scribble, smooth_stroke, snap_shape, Document, EraserMode, InkMode, PenKind, Point, SmoothingConfig, Stroke};
use raster::{Background, Canvas, Overlay, Paper, PaperKind, View};
use session::Session;
use settings::Settings;
use slint::{ComponentHandle, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use workspace::Workspace;

slint::include_modules!();

const PRIORITY: usize = recog::PRIORITY_MIN_SAMPLES;
/// Quick colours; the first one follows the theme's text colour so it is always visible.
pub const PALETTE: [[u8; 3]; 6] = [[0xc0, 0xca, 0xf5], [0xf7, 0x76, 0x8e], [0x9e, 0xce, 0x6a], [0x7a, 0xa2, 0xf7], [0xe0, 0xaf, 0x68], [0xbb, 0x9a, 0xf7]];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
    Lasso,
    TextBox,
}

/// What a text prompt overlay is asking for.
#[derive(Clone, Debug, PartialEq)]
pub enum Prompt {
    NewSection,
    NewNotebook,
    NewEncryptedName,
    NewEncryptedPassword(String),
    Unlock(String),
    SaveTemplate,
    ImagePath,
    PdfPath,
    MatPath,
    PlotFunctions,
    RestoreBackup(String),
    ExternalChange,
    FindText,
    ReplaceFind,
    ReplaceWith(String),
    InlineMath,
    LinkTarget,
    ExportVault,
    ImportVault,
}

/// Which parts of the UI need refreshing after a state change.
pub mod dirty {
    pub const CANVAS: u32 = 1;
    pub const SIDEBAR: u32 = 2;
    pub const DOC: u32 = 4;
    pub const STRIP: u32 = 8;
    pub const TABS: u32 = 16;
    pub const TOOLS: u32 = 32;
    pub const ALL: u32 = 0xff;
}

pub struct State {
    pub doc: Document,
    pub view: View,
    pub size: (u32, u32),
    pub tool: Tool,
    pub mode: InkMode,
    pub eraser_mode: EraserMode,
    pub settings: Settings,
    pub color: [u8; 3],
    pub active_pen: Option<usize>,
    pub live: Vec<Point>,
    pub eraser_path: Vec<(f32, f32)>,
    pub lasso: Vec<(f32, f32)>,
    pub selection: Vec<u64>,
    pub drag_from: Option<(f32, f32)>,
    pub drag_total: (f32, f32),
    pub down_at: Option<Instant>,
    pub t0_ms: u64,
    pub epoch: Instant,
    pub session: Session,
    pub paper: Paper,
    pub canvas: Canvas,
    pub background: Option<Background>,
    pub training: bool,
    pub train_labels: Vec<String>,
    pub train_idx: usize,
    pub ws: Option<Workspace>,
    pub graph: GraphView,
    pub graph_drag: Option<(f32, f32)>,
    pub editor: input::Editor,
    pub typing: bool,
    pub typed_since_recog: bool,
    pub consumed: HashSet<u64>,
    pub math_cache: Arc<render::MathCache>,
    pub blocks: pagedoc::BlockCache,
    pub doc_width: f32,
    pub editing: Option<usize>,
    pub speller: Option<pagedoc::Speller>,
    pub tablet: tablet::Tablet,
    pub back: Vec<String>,
    pub fwd: Vec<String>,
    pub tabs: Vec<String>,
    pub split: Option<String>,
    pub overlay: String,
    pub overlay_ids: Vec<String>,
    pub history_pick: Option<u64>,
    pub prompt: Option<Prompt>,
    pub rebinding: Option<String>,
    pub symbol_rows: Vec<symbols::Sym>,
    pub recorder: Option<media::audio::Recorder>,
    pub rec_marks: Vec<(u64, u32)>,
    pub playing: Option<Box<dyn std::any::Any>>,
    pub dirty: u32,
    pub last_backup_check: u64,
    pub last_disk_check: u64,
    pub status: String,
    /// Text ink mode: handwriting becomes words instead of math.
    pub text_mode: bool,
    pub text_result: String,
    /// Other readings of the handwritten text (the chip's "Not right?" buttons in Words mode).
    pub text_alts: Vec<String>,
    /// Canvas text being typed: (existing text id or None, page x, page y).
    pub textbox: Option<(Option<u64>, f32, f32)>,
    pub text_cache: std::collections::HashMap<String, Arc<tiny_skia::Pixmap>>,
    /// Snapshots of the canvas texts before each change, for Undo.
    pub text_history: Vec<Vec<store::CanvasText>>,
    pub last_was_text: bool,
    pub text_redo: Vec<Vec<store::CanvasText>>,
    /// Where typing starts when you just type: the last place you clicked or wrote.
    pub type_cursor: Option<(f32, f32)>,
    /// The open text box was started by typing (cursor goes after the first letter).
    pub textbox_typed: bool,
    pub textbox_first: String,
    /// Hidden while the document editor is built: show the page as a document (setting or SCRAWL_NEW_EDITOR=1).
    /// The handwriting suggestion was dismissed (Keep as ink) until the next stroke.
    pub chip_dismissed: bool,
    /// Handwriting becomes words or math by itself, depending on what the ink reads as.
    pub mode_auto: bool,
    /// The last text searched with Find (the prompt is pre-filled with it).
    pub last_find: String,
    pub plugins: Vec<plugin::Plugin>,
    /// The tag whose pages the Tags screen is listing.
    pub tag_pick: Option<String>,
    pub note_graph: Option<notegraph::NoteGraph>,
    pub ng_size: (f32, f32),
    pub plugin_errors: Vec<String>,
    pub new_editor: bool,
    pub dedit: docedit::DocEdit,
}

impl State {
    fn new(settings: Settings) -> Self {
        let root = workspace::default_root();
        let ws = Workspace::open_with(&root, settings.last_notebook.as_deref(), settings.last_page.as_deref()).ok();
        let paper = Paper { kind: paper_kind(&settings.paper_kind), spacing: settings.paper_spacing, color: [settings.paper_color[0], settings.paper_color[1], settings.paper_color[2], 0xff] };
        let mut st = Self {
            doc: Document::new(),
            view: View::default(),
            size: (800, 600),
            tool: Tool::Pen,
            mode: InkMode::MathText,
            eraser_mode: EraserMode::Stroke,
            color: settings.pens[0].rgb,
            active_pen: Some(0),
            live: Vec::new(),
            eraser_path: Vec::new(),
            lasso: Vec::new(),
            selection: Vec::new(),
            drag_from: None,
            drag_total: (0.0, 0.0),
            down_at: None,
            t0_ms: 0,
            epoch: Instant::now(),
            session: Session::new(session::config_dir().map(|d| d.join("personal.json"))),
            paper,
            canvas: Canvas::default(),
            background: None,
            training: false,
            train_labels: Vec::new(),
            train_idx: 0,
            ws,
            graph: GraphView::default(),
            graph_drag: None,
            editor: input::Editor::default(),
            typing: false,
            typed_since_recog: false,
            consumed: HashSet::new(),
            math_cache: Arc::new(render::MathCache::default()),
            blocks: pagedoc::BlockCache::default(),
            doc_width: 380.0,
            editing: None,
            speller: None,
            tablet: tablet::Tablet::default(),
            back: Vec::new(),
            fwd: Vec::new(),
            tabs: settings.tabs.clone(),
            split: None,
            overlay: String::new(),
            overlay_ids: Vec::new(),
            history_pick: None,
            prompt: None,
            rebinding: None,
            symbol_rows: Vec::new(),
            recorder: None,
            rec_marks: Vec::new(),
            playing: None,
            dirty: dirty::ALL,
            last_backup_check: 0,
            last_disk_check: 0,
            status: String::new(),
            text_mode: false,
            text_result: String::new(),
            text_alts: Vec::new(),
            textbox: None,
            text_cache: Default::default(),
            text_history: Vec::new(),
            last_was_text: false,
            text_redo: Vec::new(),
            type_cursor: None,
            textbox_typed: false,
            textbox_first: String::new(),
            chip_dismissed: false,
            mode_auto: true,
            last_find: String::new(),
            plugins: Vec::new(),
            tag_pick: None,
            note_graph: None,
            ng_size: (900.0, 560.0),
            plugin_errors: Vec::new(),
            new_editor: (settings.new_editor || std::env::var("SCRAWL_NEW_EDITOR").is_ok()) && std::env::var("SCRAWL_OLD_EDITOR").is_err(),
            dedit: docedit::DocEdit::new(Arc::new(render::MathCache::default())),
            settings,
        };
        st.width_from_pen(0);
        st.reload_plugins();
        if st.new_editor {
            st.tool = Tool::TextBox;
        }
        st.load_page_into_doc();
        st
    }

    fn width_from_pen(&mut self, i: usize) {
        if let Some(p) = self.settings.pens.get(i) {
            self.color = p.rgb;
            self.settings.width = p.width;
            if p.highlighter {
                self.tool = Tool::Highlighter;
            }
        }
    }

    pub fn light(&self) -> bool {
        !self.settings.colors().is_dark()
    }

    fn pen_color(&self) -> [u8; 4] {
        let c = self.color;
        [c[0], c[1], c[2], if self.tool == Tool::Highlighter { 0x66 } else { 0xff }]
    }

    fn live_stroke(&self) -> Stroke {
        Stroke {
            id: u64::MAX,
            points: self.live.clone(),
            color: self.pen_color(),
            width: self.settings.width,
            kind: if self.tool == Tool::Highlighter { PenKind::Highlighter } else { PenKind::Pen },
            mode: self.mode,
            t0_ms: self.t0_ms,
        }
    }

    fn now_ms(&self) -> u64 {
        store::now_ms()
    }

    /// Replace the canvas with the current page's saved ink, paper and background.
    pub fn load_page_into_doc(&mut self) {
        self.dedit.ed = docedit::EditState::default();
        self.dedit.layout = docview::Layout::new();
        if let Some(ws) = &self.ws {
            self.doc = Document::with_strokes(ws.page.strokes.clone());
            self.paper.kind = paper_kind(&ws.page.paper.kind);
            self.paper.spacing = ws.page.paper.spacing;
            self.background = ws.page.pdf_background.as_ref().and_then(|name| ws.media(name)).and_then(|png| decode_background(&png));
            // each page remembers its ink mode (the "notes" template opens in Text mode)
            self.mode_auto = self.new_editor && ws.page.ink_mode.is_none();
            match ws.page.ink_mode.as_deref() {
                Some("text") => {
                    self.mode = InkMode::MathText;
                    self.text_mode = true;
                }
                Some("drawing") => {
                    self.mode = InkMode::Drawing;
                    self.text_mode = false;
                }
                _ => {
                    self.mode = InkMode::MathText;
                    self.text_mode = false;
                }
            }
            if !self.tabs.contains(&ws.page.id) {
                self.tabs.push(ws.page.id.clone());
                if self.tabs.len() > 12 {
                    self.tabs.remove(0);
                }
            }
        }
        self.session.symbols.clear();
        self.session.selected = None;
        self.editor = input::Editor::default();
        self.consumed.clear();
        self.typed_since_recog = false;
        self.selection.clear();
        self.lasso.clear();
        self.editing = None;
        self.text_result.clear();
        self.textbox = None;
        self.text_history.clear();
        self.last_was_text = false;
        self.text_redo.clear();
        self.dirty = dirty::ALL;
    }

    /// Push canvas ink and paper into the workspace (autosave picks it up).
    pub fn sync_ws(&mut self) {
        self.last_was_text = false;
        self.text_redo.clear();
        let now = self.now_ms();
        let (strokes, kind, spacing) = (self.doc.strokes.clone(), self.paper.kind.name().to_lowercase(), self.paper.spacing);
        if let Some(ws) = &mut self.ws {
            ws.set_strokes(strokes, now);
            ws.set_paper(&kind, spacing);
        }
        self.dirty |= dirty::CANVAS;
    }

    /// Open a page, recording navigation history (back/forward) unless `nav` is false.
    pub fn goto_page(&mut self, id: &str, nav: bool) -> Result<(), String> {
        self.sync_ws();
        let now = self.now_ms();
        let cur = self.ws.as_ref().map(|w| w.page.id.clone());
        if cur.as_deref() == Some(id) {
            return Ok(());
        }
        let res = self.ws.as_mut().map(|w| w.open_page(id, now)).unwrap_or(Ok(()));
        match res {
            Ok(()) => {
                if nav {
                    if let Some(c) = cur {
                        self.back.push(c);
                        if self.back.len() > 100 {
                            self.back.remove(0);
                        }
                    }
                    self.fwd.clear();
                }
                self.load_page_into_doc();
                Ok(())
            }
            Err(e) => Err(format!("Could not open page: {e}")),
        }
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.dirty |= dirty::TOOLS;
    }
}

pub fn paper_kind(s: &str) -> PaperKind {
    match s {
        "blank" => PaperKind::Blank,
        "grid" => PaperKind::Grid,
        "dots" => PaperKind::Dots,
        "engineering" => PaperKind::Engineering,
        "polar" => PaperKind::Polar,
        _ => PaperKind::Lined,
    }
}

/// PDF page backgrounds are stored as PNG rendered at 2 px per pt.
pub const PDF_RENDER_SCALE: f32 = 2.0;

fn decode_background(png: &[u8]) -> Option<Background> {
    let pm = tiny_skia::Pixmap::decode_png(png).ok()?;
    Some(Background { pm, scale: PDF_RENDER_SCALE })
}

pub fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn color(c: [u8; 3]) -> slint::Color {
    slint::Color::from_rgb_u8(c[0], c[1], c[2])
}

pub fn image_of(r: &render::Rendered) -> slint::Image {
    slint::Image::from_rgba8_premultiplied(SharedPixelBuffer::clone_from_slice(&r.rgba_premul, r.width, r.height))
}

/// Apply theme colours and UI scale to the Slint `Theme` global.
pub fn apply_theme(ui: &MainWindow, st: &State) {
    let c = st.settings.colors();
    let t = ui.global::<Theme>();
    let mix = |a: [u8; 3], b: [u8; 3], f: f32| -> [u8; 3] { [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - f) + b[i] as f32 * f) as u8) };
    t.set_bg(color(c.bg));
    t.set_surface(color(c.surface));
    t.set_panel(color(c.panel));
    t.set_hover(color(mix(c.surface, c.accent, 0.25)));
    t.set_text(color(c.text));
    t.set_muted(color(c.muted));
    t.set_accent(color(c.accent));
    t.set_urgent(color(c.urgent));
    t.set_on_accent(color(if mix(c.accent, [0; 3], 0.0).iter().map(|&v| v as u32).sum::<u32>() > 380 { [0x10, 0x10, 0x18] } else { [0xff, 0xff, 0xff] }));
    t.set_dark(c.is_dark());
    let scale = st.settings.ui_scale * if st.settings.tablet_mode { 1.25 } else { 1.0 };
    t.set_scale(scale);
    ui.set_dark_scheme(c.is_dark());
    ui.set_theme_name(st.settings.theme.as_str().into());
    ui.set_ui_scale(st.settings.ui_scale);
    ui.set_tablet_mode(st.settings.tablet_mode);
    ui.set_spell_check(st.settings.spell_check);
    ui.set_snap_shapes(st.settings.snap_shapes);
    ui.set_backup_daily(st.settings.backup_daily);
    st.graph_dark_hint(c.is_dark());
}

impl State {
    fn graph_dark_hint(&self, _dark: bool) {}
}

fn refresh_canvas(ui: &MainWindow, st: &mut State) {
    let scale = ui.window().scale_factor();
    let (w, h) = st.size;
    let live = (!st.live.is_empty() && matches!(st.tool, Tool::Pen | Tool::Highlighter)).then(|| st.live_stroke());
    let bg = st.settings.colors().bg;
    let page = st.ws.as_ref().and_then(|ws| raster::page_size(match ws.page.mode { store::PageMode::Letter => "letter", store::PageMode::A4 => "a4", _ => "" }));
    let sel = if st.selection.is_empty() {
        None
    } else {
        let pts: Vec<Point> = st.doc.strokes.iter().filter(|s| st.selection.contains(&s.id)).flat_map(|s| s.points.iter().copied()).collect();
        ink::bbox(&pts)
    };
    let light = st.light();
    let mut texts = canvas_texts(st, scale);
    texts.extend(doc_layer(st, scale, w as f32, h as f32));
    let texts_key = { use std::hash::{Hash, Hasher}; let mut h = std::collections::hash_map::DefaultHasher::new(); for (x, y, p) in &texts { (x.to_bits(), y.to_bits(), Arc::as_ptr(p) as usize).hash(&mut h); } h.finish() };
    let (doc_sel, caret) = docedit::overlay_geometry(st);
    update_chip(ui, st);
    let ov = Overlay { lasso: &st.lasso, selection: sel, light, texts: &texts, texts_key, doc_sel: &doc_sel, caret, accent: st.settings.colors().accent };
    let mut paper = st.paper;
    if st.new_editor && paper.kind == PaperKind::Lined {
        // ruled lines would not line up with the text of a document; grid, dots etc. are still available
        paper.kind = PaperKind::Blank;
    }
    if light && paper.color[..3] == [0x41, 0x48, 0x68] {
        paper.color = [0xc4, 0xc9, 0xd9, 0xff];
    }
    let img = st.canvas.frame(&st.doc.strokes, live.as_ref(), st.view, ((w as f32 * scale) as u32, (h as f32 * scale) as u32), scale, bg, paper, page, st.background.as_ref(), &ov);
    ui.set_frame(img);
    ui.set_can_undo(st.doc.can_undo());
    ui.set_can_redo(st.doc.can_redo());
    ui.set_has_selection(!st.selection.is_empty());
}

fn refresh_sidebar(ui: &MainWindow, st: &mut State) {
    let Some(ws) = &mut st.ws else { return };
    let filter = ui.get_filter_index();
    let recent: Vec<String> = ws.store.recent().into_iter().filter(|(n, _)| *n == ws.nb.id).map(|(_, p)| p).collect();
    let mut rows: Vec<PageRow> = Vec::new();
    let pages_meta: Vec<(String, bool)> = Vec::new();
    let _ = pages_meta;
    for r in ws.rows() {
        let fav = !r.is_section && (r.is_current && ws.page.favorite || favorites_cache(ws, &r.id));
        let keep = match filter {
            1 => r.is_section || fav,
            2 => r.is_section || recent.contains(&r.id),
            _ => true,
        };
        if keep {
            rows.push(PageRow { id: r.id.into(), title: r.title.into(), detail: SharedString::new(), current: r.is_current, favorite: fav, is_section: r.is_section, section: r.section.into() });
        }
    }
    if filter == 2 {
        // recent: most recent first, sections hidden
        let mut pages: Vec<PageRow> = rows.into_iter().filter(|r| !r.is_section).collect();
        pages.sort_by_key(|r| recent.iter().position(|p| p.as_str() == r.id.as_str()).unwrap_or(usize::MAX));
        rows = pages;
    }
    ui.set_pages(model(rows));
    ui.set_page_title(ws.page.title.as_str().into());
    ui.set_page_favorite(ws.page.favorite);
    ui.set_page_tags(ws.page.tags.join(", ").into());
    let metas = ws.notebooks();
    let labels: Vec<SharedString> = metas.iter().map(|m| ws.notebook_label(m).into()).collect();
    ui.set_notebook_index(metas.iter().position(|m| m.id == ws.nb.id).unwrap_or(0) as i32);
    ui.set_notebooks(model(labels));
    ui.set_templates(model(ws.templates().into_iter().map(SharedString::from).collect()));
    ui.set_can_back(!st.back.is_empty());
    ui.set_can_forward(!st.fwd.is_empty());
}

/// Favourite flags are stored in each page; cache them per page id so the sidebar does not load
/// every page on every refresh.
fn favorites_cache(ws: &Workspace, id: &str) -> bool {
    thread_local! {
        static FAV: RefCell<std::collections::HashMap<String, bool>> = RefCell::new(Default::default());
    }
    if id == ws.page.id {
        FAV.with(|f| f.borrow_mut().insert(id.to_string(), ws.page.favorite));
        return ws.page.favorite;
    }
    FAV.with(|f| {
        if let Some(v) = f.borrow().get(id) {
            return *v;
        }
        let v = ws.load_other(id).is_some_and(|p| p.favorite);
        f.borrow_mut().insert(id.to_string(), v);
        v
    })
}

fn block_rows(st: &mut State, page: &store::Page, scale: f32) -> Vec<BlockRow> {
    let media = st.ws.as_ref().map(|w| w.page_media(page)).unwrap_or_default();
    let text = st.settings.colors().text;
    let width = st.doc_width;
    page.blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let r = st.blocks.render(b, &media, width, scale, text);
            let (image, has_image, error) = match r {
                pagedoc::Render::Ok(px) => (image_of(&px), true, String::new()),
                pagedoc::Render::Err(e) => (slint::Image::default(), false, e),
            };
            BlockRow { idx: i as i32, kind: pagedoc::kind(b).into(), image, has_image, text: pagedoc::edit_text(b).into(), error: error.into() }
        })
        .collect()
}

fn refresh_doc(ui: &MainWindow, st: &mut State) {
    if st.new_editor {
        // the old side panel is not shown; only the "Linked from" strip needs data
        let back = st.ws.as_mut().map(|w| w.backlinks()).unwrap_or_default();
        ui.set_backlinks(model(back.into_iter().map(|(id, t)| ItemRow { id: id.into(), title: t.into(), detail: SharedString::new() }).collect()));
        return;
    }
    let scale = ui.window().scale_factor();
    let Some(page) = st.ws.as_ref().map(|w| w.page.clone()) else { return };
    let rows = block_rows(st, &page, scale);
    ui.set_blocks(model(rows));
    ui.set_word_count(format!("{} words", pagedoc::word_count(&page)).into());
    let mut outline: Vec<ItemRow> = pagedoc::outline(&page).into_iter().map(|(i, lvl, t)| ItemRow { id: i.to_string().into(), title: format!("{}{t}", "  ".repeat(lvl - 1)).into(), detail: SharedString::new() }).collect();
    outline.extend(pagedoc::links(&page).into_iter().map(|t| ItemRow { id: format!("link:{t}").into(), title: format!("→ {t}").into(), detail: SharedString::new() }));
    ui.set_outline(model(outline));
    let back = st.ws.as_mut().map(|w| w.backlinks()).unwrap_or_default();
    ui.set_backlinks(model(back.into_iter().map(|(id, t)| ItemRow { id: id.into(), title: t.into(), detail: SharedString::new() }).collect()));
    ui.set_editing_index(st.editing.map_or(-1, |i| i as i32));
    // split view
    let split = st.split.clone().and_then(|id| st.ws.as_ref().and_then(|w| w.load_other(&id)));
    match split {
        Some(p) => {
            let rows = block_rows(st, &p, scale);
            ui.set_split_blocks(model(rows));
            ui.set_split_title(p.title.as_str().into());
            ui.set_split_active(true);
        }
        None => ui.set_split_active(false),
    }
    ui.set_page_mode_name(match page.mode { store::PageMode::Letter => "Letter", store::PageMode::A4 => "A4", store::PageMode::Infinite => "Infinite" }.into());
    ui.set_has_audio(page.audio.is_some());
}

fn refresh_tabs(ui: &MainWindow, st: &mut State) {
    let Some(ws) = &st.ws else { return };
    let ids = ws.page_ids();
    st.tabs.retain(|t| ids.contains(t));
    let rows: Vec<TabRow> = st.tabs.iter().map(|id| TabRow { id: id.as_str().into(), title: ws.title_of(id).unwrap_or_default().into(), current: *id == ws.page.id }).collect();
    ui.set_tabs(model(rows));
}

fn refresh_tools(ui: &MainWindow, st: &State) {
    ui.set_new_editor(st.new_editor);
    if st.new_editor {
        let (b, i, u, k) = docedit::format_state(st);
        ui.set_fmt_bold(b);
        ui.set_fmt_italic(i);
        ui.set_fmt_underline(u);
        ui.set_para_kind(k.into());
        ui.set_obj_kind(docedit::object_kind(st).into());
        let sug = docedit::link_suggestions(st);
        ui.set_link_suggest(model(sug.into_iter().map(|(t, l)| ItemRow { id: t.into(), title: l.into(), detail: SharedString::new() }).collect()));
        if let Some((x, y)) = docedit::caret_screen(st) {
            ui.set_link_x(x);
            ui.set_link_y(y);
        }
        let (cu, cr) = docedit::can_undo(st);
        ui.set_doc_can_undo(cu);
        ui.set_doc_can_redo(cr);
        ui.set_current_color(slint::Color::from_rgb_u8(st.color[0], st.color[1], st.color[2]));
    }
    ui.set_tool(match st.tool { Tool::Pen => "pen", Tool::Highlighter => "highlighter", Tool::Eraser => "eraser", Tool::Lasso => "lasso", Tool::TextBox => "textbox" }.into());
    ui.set_mode(if st.mode == InkMode::Drawing { "drawing" } else if st.text_mode { "text" } else { "math" }.into());
    match st.textbox {
        Some((id, x, y)) => {
            let (sx, sy) = (x * st.view.scale + st.view.ox, y * st.view.scale + st.view.oy);
            let initial = id.and_then(|i| st.ws.as_ref().and_then(|w| w.page.texts.iter().find(|t| t.id == i).map(|t| t.text.clone()))).unwrap_or_default();
            ui.set_textbox_x(sx);
            ui.set_textbox_y(sy - 8.0);
            let initial = if st.textbox_typed { st.textbox_first.clone() } else { initial };
            ui.set_textbox_initial(initial.into());
            ui.set_textbox_select(!st.textbox_typed);
            ui.set_textbox_visible(true);
        }
        None => ui.set_textbox_visible(false),
    }
    ui.set_pixel_erase(st.eraser_mode == EraserMode::Pixel);
    ui.set_smoothing(st.settings.smoothing);
    ui.set_stroke_width(st.settings.width);
    ui.set_recog_delay(st.settings.recog_delay_ms as f32);
    ui.set_paper_name(st.paper.kind.name().into());
    ui.set_paper_spacing(st.paper.spacing);
    ui.set_typing(st.typing);
    ui.set_training(st.training);
    ui.set_recording(st.recorder.is_some());
    ui.set_status(st.status.as_str().into());
    let first = if st.light() { [0x1f, 0x23, 0x35] } else { PALETTE[0] };
    let mut sw: Vec<[u8; 3]> = std::iter::once(first).chain(PALETTE[1..].iter().copied()).collect();
    sw.extend(st.settings.custom_colors.iter().copied());
    ui.set_swatches(model(sw.iter().map(|c| SwatchRow { r: c[0] as i32, g: c[1] as i32, b: c[2] as i32, active: *c == st.color }).collect()));
    ui.set_pens(model(st.settings.pens.iter().map(|p| SwatchRow { r: p.rgb[0] as i32, g: p.rgb[1] as i32, b: p.rgb[2] as i32, active: false }).collect()));
    ui.set_active_pen(st.active_pen.map_or(-1, |i| i as i32));
    ui.set_rebinding(st.rebinding.clone().unwrap_or_default().into());
}

/// Refresh whatever the last change marked dirty.
pub fn flush(ui: &MainWindow, st: &mut State) {
    let d = std::mem::take(&mut st.dirty);
    if d & dirty::TOOLS != 0 {
        refresh_tools(ui, st);
    }
    if d & dirty::SIDEBAR != 0 {
        refresh_sidebar(ui, st);
    }
    if d & dirty::TABS != 0 {
        refresh_tabs(ui, st);
    }
    if d & dirty::DOC != 0 {
        refresh_doc(ui, st);
    }
    if d & dirty::STRIP != 0 {
        update_strip(ui, st);
    }
    if d & dirty::CANVAS != 0 {
        refresh_canvas(ui, st);
    }
}

pub fn refresh_graph(ui: &MainWindow, st: &State) {
    let f = st.graph.frame();
    let mut buf = SharedPixelBuffer::<slint::Rgba8Pixel>::new(f.width, f.height);
    buf.make_mut_bytes().copy_from_slice(&f.rgba);
    ui.set_graph_frame(slint::Image::from_rgba8_premultiplied(buf));
    ui.set_graph_labels(model(f.labels.iter().map(|l| LabelRow { x: l.0, y: l.1, text: l.2.as_str().into() }).collect()));
    let params: Vec<ParamRow> = st.graph.graph.params.iter().map(|(k, p)| ParamRow { name: k.as_str().into(), value: p.value as f32, min: p.min as f32, max: p.max as f32 }).collect();
    ui.set_graph_params(model(params));
    ui.set_graph_info(st.graph.info().into());
}

pub fn update_strip(ui: &MainWindow, st: &State) {
    update_chip(ui, st);
    let cands: Vec<SharedString> = if st.new_editor && st.text_mode {
        st.text_alts.iter().map(|t| SharedString::from(t.replace('\n', " / "))).collect()
    } else {
        st.session.selected.and_then(|i| st.session.symbols.get(i)).map(|s| s.cands.iter().map(|c| SharedString::from(c.label.as_str())).collect()).unwrap_or_default()
    };
    ui.set_candidates(model(cands));
    update_expr(ui, st);
}

/// The expression being built: typed and drawn input share it.
pub fn current_expr(st: &State) -> layout::Expr {
    if st.typing || st.typed_since_recog || !st.editor.expr().is_empty() {
        st.editor.expr()
    } else {
        st.session.expression()
    }
}

/// The expression line: Typst text plus a Typst-rendered image, and autocomplete suggestions.
pub fn update_expr(ui: &MainWindow, st: &State) {
    if st.text_mode && !st.typing {
        ui.set_suggestions(model(vec![]));
        ui.set_has_expr_image(false);
        ui.set_expr_text(if st.text_result.is_empty() { "".into() } else { format!("“{}”  (Insert ↵ adds it to the page)", st.text_result.replace('\n', " / ")).into() });
        return;
    }
    let sugg: Vec<ItemRow> = if st.typing && !st.editor.pending_word().is_empty() {
        st.editor.suggestions(6).into_iter().map(|s| ItemRow { id: s.name.as_str().into(), title: s.name.as_str().into(), detail: s.preview.as_str().into() }).collect()
    } else {
        vec![]
    };
    ui.set_suggestions(model(sugg));
    let e = current_expr(st);
    if e.is_empty() {
        ui.set_expr_text(if st.typing && !st.editor.pending_word().is_empty() { format!("typing: {}", st.editor.pending_word()).into() } else { "".into() });
        ui.set_has_expr_image(false);
        return;
    }
    let typst = e.to_typst();
    ui.set_expr_text(format!("= {typst}").into());
    let c = st.settings.colors().text;
    match st.math_cache.render(&typst, 16.0 * st.settings.ui_scale, c, ui.window().scale_factor()) {
        Ok(r) => {
            ui.set_expr_image(image_of(&r));
            ui.set_has_expr_image(true);
        }
        Err(_) => ui.set_has_expr_image(false),
    }
}

pub fn update_training_ui(ui: &MainWindow, st: &mut State) {
    if st.train_labels.is_empty() {
        st.train_labels = st.session.labels();
    }
    let Some(label) = st.train_labels.get(st.train_idx).cloned() else { return };
    let n = st.session.personal_count(&label);
    ui.set_train_info(format!("{n} sample(s) saved, {PRIORITY}+ gives your handwriting priority").into());
    ui.set_train_label(label.into());
}

/// After recognition, feed the symbols into the shared editor so typing continues from the same cursor.
fn feed_editor(st: &mut State) {
    let glyph_of = |s: &session::SymbolRec| layout::Glyph { label: s.label.clone(), rect: s.rect };
    if !st.typed_since_recog {
        st.editor.set_expr(st.session.expression());
        st.consumed = st.session.symbols.iter().flat_map(|s| s.stroke_ids.clone()).collect();
        return;
    }
    // typed content exists: only insert ink that has not been consumed yet, at the cursor
    let fresh: Vec<layout::Glyph> = st.session.symbols.iter().filter(|s| s.stroke_ids.iter().any(|i| !st.consumed.contains(i))).map(glyph_of).collect();
    if !fresh.is_empty() {
        st.editor.insert_expr(layout::parse(&fresh));
    }
    st.consumed = st.session.symbols.iter().flat_map(|s| s.stroke_ids.clone()).collect();
}

/// Auto/manual recognition. In training mode it stores the drawn symbols for the target label.
pub fn run_recognition(ui: &MainWindow, st: &mut State) {
    if st.training {
        let Some(label) = st.train_labels.get(st.train_idx).cloned() else { return };
        match st.session.train(&label, &st.doc.strokes) {
            Ok(ids) if !ids.is_empty() => {
                st.doc.remove_ids(&ids);
                st.set_status(format!("Saved a sample of {label}"));
                st.sync_ws();
            }
            Ok(_) => {}
            Err(e) => st.set_status(e),
        }
        update_training_ui(ui, st);
        return;
    }
    match st.session.recognize(&st.doc.strokes) {
        Ok(()) => {
            if st.session.symbols.is_empty() && st.doc.strokes.iter().any(|s| s.mode == InkMode::MathText) {
                st.set_status("Nothing to recognize");
            }
            if st.mode_auto && st.mode == InkMode::MathText && !st.typed_since_recog {
                if st.speller.is_none() {
                    st.speller = pagedoc::Speller::new();
                }
                let (t, _) = handwriting::to_text_alts(&st.session.symbols, st.speller.as_ref());
                st.text_mode = looks_like_words(&t, st.speller.as_ref(), st.session.symbols.len());
            }
            if st.text_mode {
                if st.speller.is_none() {
                    st.speller = pagedoc::Speller::new();
                }
                (st.text_result, st.text_alts) = handwriting::to_text_alts(&st.session.symbols, st.speller.as_ref());
            } else {
                feed_editor(st);
            }
        }
        Err(e) => st.set_status(format!("Recognizer failed to load: {e}")),
    }
    st.dirty |= dirty::STRIP;
    tablet::push_state(st);
}

fn main() -> Result<(), slint::PlatformError> {
    if std::env::args().any(|a| a == "--self-test") {
        std::process::exit(selftest::run());
    }
    if std::env::args().any(|a| a == "--version") {
        println!("scrawl {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let shot_path = std::env::var("SCRAWL_SHOT").ok();
    let shot_size: (u32, u32) = std::env::var("SCRAWL_SHOT_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1500, 900));
    let shot_scale: f32 = std::env::var("SCRAWL_SHOT_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let shot_window = shot_path.as_ref().map(|_| headless::install(shot_size.0, shot_size.1, shot_scale));
    let mut settings = if shot_path.is_some() { Settings::default() } else { Settings::load() };
    if let Ok(t) = std::env::var("SCRAWL_SHOT_THEME") {
        settings.theme = t;
    }
    let ui = MainWindow::new()?;
    let st = Rc::new(RefCell::new(State::new(settings)));
    let weak = ui.as_weak();

    {
        let mut s = st.borrow_mut();
        apply_theme(&ui, &s);
        ui.set_export_formats(model(["PDF", "PNG", "Markdown", "Typst", "LaTeX", "HTML"].iter().map(|t| SharedString::from(*t)).collect()));
        ui.set_show_sidebar(s.settings.show_sidebar);
        ui.set_show_doc(s.settings.show_doc);
        let mut dt = slint::DataTransfer::default();
        dt.set_plain_text("scrawl-page".into());
        ui.set_drag_data(dt);
        refresh_graph(&ui, &s);
        if let Some(msg) = s.ws.as_ref().and_then(|w| w.recovered.clone()) {
            s.set_status(msg);
        }
        if s.ws.is_none() {
            s.set_status(format!("Could not open the notes folder {}", workspace::default_root().display()));
        }
        flush(&ui, &mut s);
        if let Some((w, h)) = s.settings.window.filter(|_| shot_path.is_none()) {
            ui.window().set_size(slint::LogicalSize::new(w as f32, h as f32));
        }
        if s.settings.maximized && shot_path.is_none() {
            ui.window().set_maximized(true);
        }
    }

    // Run a state mutation, then refresh what it marked dirty.
    macro_rules! with {
        (|$ui:ident, $s:ident $(, $a:pat_param = $t:ty)*| $body:expr) => {{
            let st = st.clone();
            let weak = weak.clone();
            move |$($a: $t),*| {
                let Some($ui) = weak.upgrade() else { return };
                let $s = &mut *st.borrow_mut();
                let _ = &$ui;
                $body;
                flush(&$ui, $s);
            }
        }};
    }

    ui.on_window_resized(with!(|_ui, s, w = f32, h = f32| {
        s.settings.window = Some((w as u32, h as u32));
    }));
    ui.on_doc_resized(with!(|_ui, s, w = f32| {
        let w = w.max(120.0);
        if (w - s.doc_width).abs() > 4.0 {
            s.doc_width = w;
            s.dirty |= dirty::DOC;
        }
    }));
    ui.on_canvas_resized(with!(|_ui, s, w = f32, h = f32| {
        s.size = (w.max(1.0) as u32, h.max(1.0) as u32);
        s.dirty |= dirty::CANVAS;
    }));

    ui.on_pointer_down(with!(|_ui, s, x = f32, y = f32| actions::pointer_down(s, x, y)));
    ui.on_pointer_move(with!(|_ui, s, x = f32, y = f32| actions::pointer_move(s, x, y)));
    {
        let (st2, w) = (st.clone(), weak.clone());
        ui.on_pointer_up(move || {
            let Some(ui) = w.upgrade() else { return };
            let schedule = {
                let s = &mut *st2.borrow_mut();
                let r = actions::pointer_up(&ui, s);
                flush(&ui, s);
                r
            };
            if schedule {
                ui.invoke_pen_lifted();
            }
        });
    }

    ui.on_zoom(with!(|_ui, s, f = f32, x = f32, y = f32| {
        s.view.zoom_at(f, x, y);
        s.dirty |= dirty::CANVAS;
    }));
    ui.on_pan(with!(|_ui, s, dx = f32, dy = f32| {
        s.view.ox += dx;
        s.view.oy += dy;
        s.dirty |= dirty::CANVAS;
    }));
    ui.on_set_tool(with!(|_ui, s, t = SharedString| actions::set_tool(s, &t)));
    ui.on_set_mode(with!(|_ui, s, m = SharedString| {
        actions::set_mode(s, &m);
    }));
    ui.on_toggle_pixel_erase(with!(|_ui, s| {
        s.eraser_mode = if s.eraser_mode == EraserMode::Stroke { EraserMode::Pixel } else { EraserMode::Stroke };
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_set_smoothing(with!(|_ui, s, v = f32| {
        s.settings.smoothing = v.clamp(0.0, 1.0);
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_set_width(with!(|_ui, s, v = f32| {
        s.settings.width = v.clamp(0.5, 24.0);
        s.active_pen = None;
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_set_swatch(with!(|_ui, s, i = i32| {
        let first = if s.light() { [0x1f, 0x23, 0x35] } else { PALETTE[0] };
        let all: Vec<[u8; 3]> = std::iter::once(first).chain(PALETTE[1..].iter().copied()).chain(s.settings.custom_colors.iter().copied()).collect();
        if let Some(c) = all.get(i.max(0) as usize) {
            s.color = *c;
            s.active_pen = None;
        }
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_set_custom_color(with!(|_ui, s, r = i32, g = i32, b = i32| {
        let c = [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8];
        s.color = c;
        if !s.settings.custom_colors.contains(&c) {
            s.settings.custom_colors.push(c);
            if s.settings.custom_colors.len() > 8 {
                s.settings.custom_colors.remove(0);
            }
        }
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_use_pen(with!(|_ui, s, i = i32| actions::run(&_ui, s, &format!("pen{}", i + 1))));
    ui.on_save_pen(with!(|_ui, s, i = i32| {
        if let Some(p) = s.settings.pens.get_mut(i.max(0) as usize) {
            p.rgb = s.color;
            p.width = s.settings.width;
            p.highlighter = s.tool == Tool::Highlighter;
            s.active_pen = Some(i as usize);
            s.set_status(format!("Saved favourite pen {}", i + 1));
        }
    }));
    ui.on_set_spacing(with!(|_ui, s, v = f32| {
        s.paper.spacing = v.clamp(8.0, 128.0);
        s.settings.paper_spacing = s.paper.spacing;
        s.sync_ws();
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_set_paper_color(with!(|_ui, s, i = i32| {
        const C: [[u8; 3]; 6] = [[0x41, 0x48, 0x68], [0x56, 0x5f, 0x89], [0x7a, 0xa2, 0xf7], [0x9e, 0xce, 0x6a], [0xe0, 0xaf, 0x68], [0xc0, 0xca, 0xf5]];
        let c = C[(i.max(0) as usize).min(5)];
        s.settings.paper_color = c;
        s.paper.color = [c[0], c[1], c[2], 0xff];
        s.dirty |= dirty::CANVAS;
    }));
    ui.on_set_delay(with!(|_ui, s, v = f32| {
        s.settings.recog_delay_ms = (v as u64).clamp(150, 5000);
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_choose(with!(|ui, s, i = i32| {
        if s.text_mode && s.new_editor && !s.text_alts.is_empty() {
            // Words mode: the buttons are whole-text readings
            let i = i.max(0) as usize;
            if i < s.text_alts.len() {
                std::mem::swap(&mut s.text_result, &mut s.text_alts[i]);
                // learn the letters that changed, so the same handwriting reads right next time
                let mut learned = 0;
                for (sym, ch) in handwriting::letters_for(&s.session.symbols, &s.text_result) {
                    let was = s.session.symbols.get(sym).and_then(handwriting::reads_as);
                    if ch.is_alphanumeric() && was.is_some_and(|w| !w.eq_ignore_ascii_case(&ch)) && s.session.learn_symbol(sym, &ch.to_string(), &s.doc.strokes) {
                        learned += 1;
                    }
                }
                if learned > 0 {
                    s.set_status(format!("Learned {learned} letter{} of your handwriting", if learned == 1 { "" } else { "s" }));
                }
                s.dirty |= dirty::STRIP;
            }
            return;
        }
        if s.session.choose(i.max(0) as usize, &s.doc.strokes) {
            if !s.typed_since_recog {
                s.editor.set_expr(s.session.expression());
            }
            s.dirty |= dirty::STRIP;
            if s.text_mode {
                (s.text_result, s.text_alts) = handwriting::to_text_alts(&s.session.symbols, s.speller.as_ref());
            }
            s.set_status("Correction saved to your handwriting profile");
            tablet::push_state(s);
        }
    }));
    ui.on_train_step(with!(|ui, s, d = i32| {
        let n = s.train_labels.len().max(1) as i32;
        s.train_idx = ((s.train_idx as i32 + d).rem_euclid(n)) as usize;
        update_training_ui(&ui, s);
    }));
    ui.on_train_unfilled(with!(|ui, s| {
        let n = s.train_labels.len();
        for k in 1..=n {
            let i = (s.train_idx + k) % n;
            let l = s.train_labels[i].clone();
            if s.session.personal_count(&l) < PRIORITY {
                s.train_idx = i;
                break;
            }
        }
        update_training_ui(&ui, s);
    }));

    // ---- keyboard: every shortcut goes through the (rebindable) command table ----------------
    {
        let (st2, w) = (st.clone(), weak.clone());
        ui.on_shortcut(move |key, ctrl, alt, shift, meta| {
            let Some(ui) = w.upgrade() else { return false };
            let s = &mut *st2.borrow_mut();
            let handled = actions::shortcut(&ui, s, &key, ctrl, alt, shift, meta);
            flush(&ui, s);
            handled
        });
    }
    {
        let (st2, w) = (st.clone(), weak.clone());
        ui.on_plain_key(move |key| {
            let Some(ui) = w.upgrade() else { return false };
            let s = &mut *st2.borrow_mut();
            let handled = actions::plain_key(&ui, s, &key);
            flush(&ui, s);
            handled
        });
    }
    {
        let (st2, w) = (st.clone(), weak.clone());
        ui.on_doc_key(move |key, ctrl, alt, shift, meta| {
            let Some(ui) = w.upgrade() else { return false };
            let s = &mut *st2.borrow_mut();
            let handled = docedit::key(s, &key, ctrl, alt, shift, meta);
            flush(&ui, s);
            handled
        });
    }
    ui.on_note_graph_resized(with!(|ui, s, w = f32, h = f32| {
        if w > 50.0 && h > 50.0 && ((w - s.ng_size.0).abs() > 1.0 || (h - s.ng_size.1).abs() > 1.0 || s.overlay == "notegraph") {
            s.ng_size = (w, h);
            refresh_note_graph(&ui, s);
        }
    }));
    ui.on_note_graph_click(with!(|ui, s, fx = f32, fy = f32| {
        let title = s.note_graph.as_ref().and_then(|g| g.hit(fx, fy, s.ng_size.0, s.ng_size.1, 10.0).map(|i| g.nodes[i].title.clone()));
        if let Some(t) = title {
            ui.set_overlay("".into());
            s.overlay.clear();
            ui.invoke_focus_canvas();
            actions::follow_link(s, &t);
        }
    }));
    ui.on_open_menu(with!(|ui, s, name = SharedString| {
        if ui.get_menu() == name {
            ui.set_menu("".into());
        } else {
            let mut rows: Vec<MenuRow> = Vec::new();
            let mut last = "";
            for it in menus::menu(&name) {
                if it.group != last {
                    rows.push(MenuRow { id: "".into(), label: it.group.into(), hint: "".into(), header: true });
                    last = it.group;
                }
                rows.push(MenuRow { id: it.id.into(), label: it.label.into(), hint: it.hint.into(), header: false });
            }
            if name == "more" {
                let mut last_plugin = String::new();
                for (id, title, plugin) in s.plugin_commands() {
                    if last_plugin.is_empty() {
                        rows.push(MenuRow { id: "".into(), label: "Plugins".into(), hint: "".into(), header: true });
                    }
                    last_plugin = plugin.clone();
                    rows.push(MenuRow { id: id.into(), label: title.into(), hint: plugin.into(), header: false });
                }
            }
            ui.set_menu_items(model(rows));
            ui.set_menu(name);
        }
    }));
    ui.on_run_command(with!(|ui, s, id = SharedString| actions::run(&ui, s, &id)));

    // ---- typing mode ------------------------------------------------------------------------
    ui.on_type_text(with!(|ui, s, t = SharedString| {
        s.editor.insert_str(&t);
        s.typed_since_recog = true;
        s.dirty |= dirty::STRIP;
        tablet::push_state(s);
    }));
    ui.on_type_nav(with!(|ui, s, k = SharedString| {
        tablet::nav(s, k.as_str());
        s.typed_since_recog = true;
        s.dirty |= dirty::STRIP;
    }));
    ui.on_type_paste(with!(|ui, s, t = SharedString| actions::type_paste(s, &t)));
    ui.on_accept_suggestion(with!(|ui, s, name = SharedString| {
        if let Some(sug) = s.editor.suggestions(20).into_iter().find(|x| x.name == name.as_str()) {
            s.editor.accept(&sug);
            s.typed_since_recog = true;
        }
        s.dirty |= dirty::STRIP;
    }));

    // ---- notes: pages, search, export ----------------------------------------------------------
    ui.on_open_page(with!(|ui, s, id = SharedString| {
        if let Err(e) = s.goto_page(&id, true) {
            s.set_status(e);
        }
        ui.set_search_active(false);
    }));
    ui.on_open_tab(with!(|ui, s, id = SharedString| {
        if let Err(e) = s.goto_page(&id, true) {
            s.set_status(e);
        }
    }));
    ui.on_close_tab(with!(|ui, s, id = SharedString| actions::close_tab(s, &id)));
    ui.on_new_page(with!(|ui, s, t = SharedString| actions::new_page(s, &t)));
    ui.on_rename_page(with!(|ui, s, t = SharedString| {
        if let Some(w) = &mut s.ws {
            w.rename(&t);
        }
        s.dirty |= dirty::SIDEBAR | dirty::TABS;
    }));
    ui.on_set_tags(with!(|ui, s, t = SharedString| {
        if let Some(w) = &mut s.ws {
            w.set_tags(&t);
        }
    }));
    ui.on_do_search(with!(|ui, s, q = SharedString| {
        if q.trim().is_empty() {
            ui.set_search_active(false);
            return;
        }
        let hits = s.ws.as_mut().map(|w| w.search(q.trim())).unwrap_or_default();
        let rows: Vec<PageRow> = hits
            .into_iter()
            .map(|h| PageRow { id: h.page_id.into(), title: h.title.into(), detail: h.snippet.replace("<b>", "").replace("</b>", "").into(), current: false, favorite: false, is_section: false, section: SharedString::new() })
            .collect();
        ui.set_search_results(model(rows));
        ui.set_search_active(true);
    }));
    ui.on_export_page(with!(|ui, s, f = SharedString| actions::export(s, &f)));
    ui.on_set_notebook(with!(|ui, s, i = i32| actions::set_notebook(&ui, s, i)));
    ui.on_set_filter(with!(|ui, s, i = i32| {
        ui.set_filter_index(i);
        s.dirty |= dirty::SIDEBAR;
    }));
    ui.on_move_row(with!(|ui, s, id = SharedString, d = i32| {
        if let Some(w) = &mut s.ws {
            let _ = w.move_page_step(&id, d);
        }
        s.dirty |= dirty::SIDEBAR;
    }));
    ui.on_drop_row(with!(|ui, s, src = SharedString, target = SharedString| {
        if let Some(w) = &mut s.ws {
            match w.drop_page(&src, &target) {
                Ok(true) => s.status = "Page moved".into(),
                Ok(false) => {}
                Err(e) => s.status = format!("Could not move: {e}"),
            }
        }
        s.dirty |= dirty::SIDEBAR | dirty::TOOLS;
    }));

    // ---- page blocks -------------------------------------------------------------------------
    ui.on_block_action(with!(|ui, s, i = i32, a = SharedString| actions::block_action(&ui, s, i.max(0) as usize, &a)));
    ui.on_editor_format(with!(|ui, s, f = SharedString| {
        ui.set_editor_text(pagedoc::apply_format(&ui.get_editor_text(), &f).into());
    }));
    ui.on_editor_commit(with!(|ui, s| actions::editor_commit(&ui, s)));
    ui.on_editor_cancel(with!(|ui, s| {
        s.editing = None;
        ui.set_editor_text("".into());
        ui.set_spelling(model(vec![]));
        s.dirty |= dirty::DOC;
    }));
    ui.on_fix_spelling(with!(|ui, s, from = SharedString, to = SharedString| {
        ui.set_editor_text(pagedoc::replace_word(&ui.get_editor_text(), &from, &to).into());
        actions::spell(&ui, s);
    }));
    ui.on_find_replace(with!(|ui, s, f = SharedString, r = SharedString, apply = bool| {
        let n = s.ws.as_mut().map(|w| {
            let n = pagedoc::find_replace(&mut w.page, &f, &r, apply);
            if apply && n > 0 {
                w.touch();
                let (id, t) = (w.page.id.clone(), w.page.title.clone());
                w.nb.rename_page(&id, &t);
            }
            n
        });
        let n = n.unwrap_or(0);
        ui.set_find_info(if f.is_empty() { "".into() } else if apply { format!("Replaced {n}").into() } else { format!("{n} match(es) on this page").into() });
        if apply {
            s.dirty |= dirty::DOC | dirty::SIDEBAR;
        }
    }));
    ui.on_goto_item(with!(|ui, s, id = SharedString| {
        if let Some(title) = id.strip_prefix("link:") {
            // follow a [[link]]; a link to a page that does not exist yet creates it
            actions::follow_link(s, title);
        } else if let Ok(i) = id.parse::<usize>() {
            s.editing = Some(i);
            let text = s.ws.as_ref().and_then(|w| w.page.blocks.get(i)).map(pagedoc::edit_text).unwrap_or_default();
            ui.set_editor_text(text.into());
            s.dirty |= dirty::DOC;
        }
    }));

    // ---- overlays ------------------------------------------------------------------------------
    ui.on_overlay_query(with!(|ui, s, q = SharedString| actions::overlay_query(&ui, s, &q)));
    ui.on_overlay_pick(with!(|ui, s, id = SharedString| actions::overlay_pick(&ui, s, &id)));
    ui.on_overlay_closed(with!(|ui, s| {
        s.overlay.clear();
        s.rebinding = None;
        s.prompt = None;
        s.history_pick = None;
        ui.set_has_preview(false);
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_prompt_ok(with!(|ui, s, t = SharedString| actions::prompt_ok(&ui, s, &t)));
    ui.on_insert_symbol(with!(|ui, s, sym = SharedString| actions::insert_symbol(&ui, s, &sym)));
    ui.on_start_rebind(with!(|ui, s, id = SharedString| {
        s.rebinding = Some(id.to_string());
        s.dirty |= dirty::TOOLS;
    }));
    ui.on_reset_shortcuts(with!(|ui, s| {
        s.settings.shortcuts.clear();
        actions::overlay_query(&ui, s, "");
        s.set_status("Shortcuts reset to defaults");
    }));

    // ---- settings ------------------------------------------------------------------------------
    ui.on_set_theme(with!(|ui, s, t = SharedString| {
        s.settings.theme = t.to_string();
        if s.settings.theme == "custom" && s.settings.custom == settings::ThemeColors::dark() {
            s.settings.custom = settings::ThemeColors::dark();
        }
        actions::theme_changed(&ui, s);
    }));
    ui.on_set_custom_theme(with!(|ui, s, part = SharedString, r = i32, g = i32, b = i32| {
        let c = [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8];
        let t = &mut s.settings.custom;
        match part.as_str() {
            "bg" => t.bg = c,
            "surface" => t.surface = c,
            "panel" => t.panel = c,
            "text" => t.text = c,
            "accent" => t.accent = c,
            "urgent" => t.urgent = c,
            _ => {}
        }
        actions::theme_changed(&ui, s);
    }));
    ui.on_set_ui_scale(with!(|ui, s, v = f32| {
        s.settings.ui_scale = (v * 20.0).round() / 20.0;
        s.settings.sanitize();
        actions::theme_changed(&ui, s);
    }));
    ui.on_toggle_setting(with!(|ui, s, which = SharedString| {
        match which.as_str() {
            "tablet-mode" => s.settings.tablet_mode = !s.settings.tablet_mode,
            "snap" => s.settings.snap_shapes = !s.settings.snap_shapes,
            "spell" => s.settings.spell_check = !s.settings.spell_check,
            "backup" => s.settings.backup_daily = !s.settings.backup_daily,
            _ => {}
        }
        actions::theme_changed(&ui, s);
    }));

    // ---- graph -------------------------------------------------------------------------------
    ui.on_graph_text_edited(with!(|ui, s, t = SharedString| {
        s.graph.set_text(&t);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_set_param(with!(|ui, s, n = SharedString, v = f32| {
        s.graph.graph.set_param(&n, v as f64);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_pointer(with!(|ui, s, kind = i32, x = f32, y = f32| {
        match kind {
            0 => s.graph_drag = Some((x, y)),
            1 => {
                if let Some((lx, ly)) = s.graph_drag {
                    s.graph.vp.pan_px((x - lx) as f64, (y - ly) as f64);
                    s.graph_drag = Some((x, y));
                    refresh_graph(&ui, s);
                }
            }
            _ => s.graph_drag = None,
        }
    }));
    ui.on_graph_zoom(with!(|ui, s, f = f32, x = f32, y = f32| {
        let (w, h) = (s.graph.size.0 as f64, s.graph.size.1 as f64);
        s.graph.vp.zoom_at(f as f64, x as f64, y as f64, w, h);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_resized(with!(|ui, s, w = f32, h = f32| {
        s.graph.size = (w.max(50.0) as u32, h.max(50.0) as u32);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_run_matlab(with!(|ui, s, script = SharedString| {
        let out = s.graph.run_matlab(&script);
        ui.set_graph_out(out.into());
        refresh_graph(&ui, s);
    }));
    ui.on_graph_load_mat(with!(|ui, s, _p = SharedString| actions::load_mat(&ui, s)));

    // ---- tablet --------------------------------------------------------------------------------
    tablet::wire(&ui, &st);

    // ---- auto-recognize (restart a single-shot timer on every pen-up) ----------------------------
    let timer = Rc::new(slint::Timer::default());
    {
        let (st2, weak2, timer2) = (st.clone(), weak.clone(), timer.clone());
        ui.on_textbox_commit(with!(|ui, s, t = SharedString| actions::textbox_finish(s, &t, true)));
    ui.on_textbox_done(with!(|ui, s, t = SharedString| {
        actions::textbox_finish(s, &t, false);
        ui.invoke_focus_canvas();
    }));
    ui.on_textbox_cancel(with!(|ui, s| {
        s.textbox = None;
        s.dirty |= dirty::TOOLS;
        ui.invoke_focus_canvas();
    }));
    ui.on_pen_lifted(move || {
            let (st3, weak3) = (st2.clone(), weak2.clone());
            // Undo/redo/delete call this while State is already borrowed: never borrow again here.
            let delay = st2.try_borrow().map(|s| s.settings.recog_delay_ms).unwrap_or_else(|_| Settings::load().recog_delay_ms);
            timer2.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(delay), move || {
                if let Some(ui) = weak3.upgrade() {
                    let s = &mut *st3.borrow_mut();
                    if s.down_at.is_none() {
                        run_recognition(&ui, s);
                        flush(&ui, s);
                    }
                }
            });
        });
    }

    // ---- autosave + housekeeping: 1 s tick, does nothing while idle and saved ---------------------
    let autosave = Rc::new(slint::Timer::default());
    {
        let (st2, weak2) = (st.clone(), weak.clone());
        autosave.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
            let Some(ui) = weak2.upgrade() else { return };
            let s = &mut *st2.borrow_mut();
            actions::housekeeping(&ui, s);
            flush(&ui, s);
        });
    }

    // caret blink for the document editor
    let blink = Rc::new(slint::Timer::default());
    {
        let (st2, weak2) = (st.clone(), weak.clone());
        blink.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(530), move || {
            let Some(ui) = weak2.upgrade() else { return };
            let Ok(mut s) = st2.try_borrow_mut() else { return };
            if docedit::active(&s) && s.dedit.ed.sel.is_caret() {
                s.dedit.ed.caret_on = !s.dedit.ed.caret_on;
                s.dirty |= dirty::CANVAS;
                flush(&ui, &mut s);
            }
        });
    }
    let _keep_blink = blink;

    // Save, remember the session, and stop the tablet server when the window closes.
    {
        let st2 = st.clone();
        let w = weak.clone();
        ui.window().on_close_requested(move || {
            let mut s = st2.borrow_mut();
            s.sync_ws();
            let now = s.now_ms();
            if let Some(ws) = &mut s.ws {
                let _ = ws.save(now);
            }
            if let Some(rec) = s.recorder.take() {
                let _ = rec.finish();
            }
            if let Some(ui) = w.upgrade() {
                s.settings.maximized = ui.window().is_maximized();
                s.settings.show_doc = ui.get_show_doc();
                s.settings.show_sidebar = ui.get_show_sidebar();
            }
            s.settings.last_notebook = s.ws.as_ref().filter(|w| !w.is_encrypted()).map(|w| w.nb.id.clone());
            s.settings.last_page = s.ws.as_ref().filter(|w| !w.is_encrypted()).map(|w| w.page.id.clone());
            s.settings.tabs = if s.ws.as_ref().is_some_and(|w| w.is_encrypted()) { vec![] } else { s.tabs.clone() };
            s.settings.save();
            s.tablet.stop();
            slint::CloseRequestResponse::HideWindow
        });
    }

    // Files dropped on the window (X11 and Windows; winit has no Wayland drag-and-drop yet).
    {
        use slint::winit_030::{winit, WinitWindowAccessor};
        let (st2, w) = (st.clone(), weak.clone());
        ui.window().on_winit_window_event(move |_, ev| {
            if let winit::event::WindowEvent::DroppedFile(path) = ev {
                if let Some(ui) = w.upgrade() {
                    let s = &mut *st2.borrow_mut();
                    actions::open_file(&ui, s, path);
                    flush(&ui, s);
                }
            }
            slint::winit_030::EventResult::Propagate
        });
    }

    if let (Some(path), Some(win)) = (shot_path, shot_window) {
        headless::settle(&win, &ui, 150);
        run_scene(&ui, &st, &std::env::var("SCRAWL_SHOT_SCENE").unwrap_or_default());
        {
            let s = &mut *st.borrow_mut();
            flush(&ui, s);
        }
        return match headless::capture(&win, &ui, &path, shot_size, 800) {
            Ok(()) => {
                println!("saved {path}");
                Ok(())
            }
            Err(e) => Err(slint::PlatformError::Other(e)),
        };
    }

    ui.run()
}

/// Scripted UI scenes for off-screen screenshots (drives the real callbacks).
fn run_scene(ui: &MainWindow, st: &Rc<RefCell<State>>, scene: &str) {
    let draw = |pts: &[(f32, f32)]| {
        ui.invoke_pointer_down(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            ui.invoke_pointer_move(p.0, p.1);
        }
        ui.invoke_pointer_up();
    };
    let line = |a: (f32, f32), b: (f32, f32)| -> Vec<(f32, f32)> { (0..=20).map(|i| (a.0 + (b.0 - a.0) * i as f32 / 20.0, a.1 + (b.1 - a.1) * i as f32 / 20.0)).collect() };
    for scene in scene.split(',') {
        match scene {
            "draw" => {
                draw(&line((100.0, 300.0), (160.0, 300.0)));
                draw(&line((130.0, 270.0), (130.0, 330.0)));
                draw(&line((220.0, 285.0), (290.0, 285.0)));
                draw(&line((220.0, 315.0), (290.0, 315.0)));
                ui.invoke_run_command("recognize".into());
            }
            "insert" => ui.invoke_run_command("insert".into()),
            "undo" => {
                ui.invoke_run_command("undo".into());
                ui.invoke_run_command("redo".into());
                ui.invoke_run_command("undo".into());
            }
            "typing" => {
                ui.invoke_run_command("typing".into());
                for c in "alpha + int".chars() {
                    ui.invoke_type_text(c.to_string().into());
                }
                ui.invoke_type_text(" ".into());
                for c in "_0^1 x^2/3".chars() {
                    ui.invoke_type_text(c.to_string().into());
                }
            }
            "text" => {
                ui.set_editor_text("= Chain rule\nIf $f$ and $g$ are differentiable then\n- the derivitive of $f(g(x))$ is $f'(g(x)) g'(x)$\n- see [[Limits]]".into());
                ui.invoke_editor_commit();
            }
            "graph" => {
                ui.invoke_run_command("graph".into());
                ui.invoke_graph_resized(1000.0, 560.0);
                let t = "y = a sin(x)\ny = x^2/4 - 2\nx^2 + y^2 = 9\na = 2";
                ui.set_graph_text(t.into());
                ui.invoke_graph_text_edited(t.into());
            }
            "matlab" => {
                ui.invoke_run_command("graph".into());
                ui.invoke_graph_resized(1000.0, 560.0);
                ui.invoke_graph_text_edited("".into());
                ui.invoke_graph_run_matlab("x = linspace(0, 2*pi, 200);\ny = zeros(1, 200);\nfor k = 1:2:7\n  y = y + sin(k*x)/k;\nend\nplot(x, y); title('Square wave (for loop)')".into());
            }
            "doc" => {
                let s = &mut *st.borrow_mut();
                s.new_editor = true;
                if let Some(ws) = &mut s.ws {
                    let mut d = notedoc::Doc::from_markup("= Chain rule\nIf $f$ and $g$ are *differentiable* then the derivative of the composite $f(g(x))$ is $f'(g(x)) g'(x)$, see [[Limits]]. This paragraph is long enough to wrap onto a second and even a third line in a narrow window.\n\n== Things to remember\n- bullets work\n  - and nest\n- ☐ buy brandy\n- ☑ write the notes\n+ numbered one\n+ numbered two\n\n#quote(block: true)[A quotation in italics.]\n\nPlain closing paragraph with `code` and #underline[underline].");
                    d.push(notedoc::ParaKind::MathDisplay { expr: layout::Expr::Frac(Box::new(layout::Expr::sym("a")), Box::new(layout::Expr::sym("b"))), source: None }, vec![]);
                    d.push(notedoc::ParaKind::Rule, vec![]);
                    ws.page.doc = Some(d);
                }
                s.dirty |= dirty::CANVAS;
            }
            "docedit" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    s.tool = Tool::TextBox;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("= Meeting notes\nFirst paragraph of the notes."));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                let key = |t: &str, ctrl: bool, shift: bool| {
                    assert!(ui.invoke_doc_key(t.into(), ctrl, false, shift, false), "key {t:?} was not used");
                };
                let ty = |t: &str| t.chars().for_each(|c| key(&c.to_string(), false, false));
                // click in the first paragraph, jump to the end, then Enter and type a list and some words
                ui.invoke_pointer_down(120.0, 100.0);
                ui.invoke_pointer_up();
                key("\u{F72B}", false, false);
                key("\n", false, false);
                ty("- apples");
                key("\n", false, false);
                ty("bread and ");
                ty("$x^2$");
                key("\n", false, false);
                key("\n", false, false);
                ty("Done. Select this with the keyboard");
                for _ in 0..8 {
                    key("\u{F702}", true, true);
                }
                key("b", true, false);
                let s = st.borrow();
                let d = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).expect("document");
                assert_eq!(d.paras[2].kind, notedoc::ParaKind::Bullet { indent: 0 }, "{:?}", d.paras);
                assert!(d.paras.len() >= 5, "{:?}", d.paras);
            }
            "docword" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("= Notes\nShopping list:"));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                ui.invoke_set_tool("pen".into());
                ui.invoke_set_mode("text".into());
                draw(&line((100.0, 280.0), (100.0, 340.0)));
                let o: Vec<(f32, f32)> = (0..=30).map(|i| { let a = i as f32 / 30.0 * std::f32::consts::TAU; (135.0 + 14.0 * a.cos(), 326.0 + 14.0 * a.sin()) }).collect();
                draw(&o);
                ui.invoke_run_command("recognize".into());
            }
            "undoorder" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("start"));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                let words = |st: &Rc<RefCell<State>>| st.borrow().ws.as_ref().and_then(|w| w.page.doc.as_ref()).map(|d| d.plain_text()).unwrap_or_default();
                let ink = |st: &Rc<RefCell<State>>| st.borrow().doc.strokes.len();
                ui.invoke_set_tool("pen".into());
                draw(&line((100.0, 300.0), (200.0, 300.0)));
                // a letter typed with the pen tool switches to typing at the end of the text
                assert!(ui.invoke_doc_key("x".into(), false, false, false, false));
                assert_eq!(words(st), "startx");
                ui.invoke_set_tool("pen".into());
                draw(&line((100.0, 350.0), (200.0, 350.0)));
                draw(&line((100.0, 400.0), (200.0, 400.0)));
                assert_eq!(ink(st), 3);
                ui.invoke_run_command("undo".into());
                assert_eq!((ink(st), words(st).as_str()), (2, "startx"), "the last change (ink) is undone first");
                ui.invoke_run_command("undo".into());
                assert_eq!(ink(st), 1, "both newer strokes before the older text");
                ui.invoke_run_command("undo".into());
                assert_eq!((ink(st), words(st).as_str()), (1, "start"), "then the typing");
                ui.invoke_run_command("redo".into());
                assert_eq!(words(st), "startx");
                println!("undo order: ok");
            }
            "linksuggest" => {
                ui.invoke_run_command("quick-note".into());
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    s.tool = Tool::TextBox;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("Related reading: "));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                ui.invoke_pointer_down(300.0, 60.0);
                ui.invoke_pointer_up();
                ui.invoke_doc_key("\u{F72B}".into(), false, false, false, false);
                for c in "[[W".chars() {
                    ui.invoke_doc_key(c.to_string().into(), false, false, false, false);
                }
                assert!(ui.invoke_doc_key("\t".into(), false, false, false, false), "Tab takes the first suggestion");
                let s = st.borrow();
                let d = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).unwrap();
                assert_eq!(d.plain_text(), "Related reading: Welcome to Scrawl", "{:?}", d.paras);
                drop(s);
                for c in " and [[Zeta".chars() {
                    ui.invoke_doc_key(c.to_string().into(), false, false, false, false);
                }
            }
            "tagtyping" => {
                // typing "#calculus " must add one tag, not "c", "ca", "cal", ...
                ui.invoke_run_command("quick-note".into());
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    s.tool = Tool::TextBox;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("Topic: "));
                        ws.page.tags = vec!["kept".into()];
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                ui.invoke_pointer_down(300.0, 60.0);
                ui.invoke_pointer_up();
                ui.invoke_doc_key("\u{F72B}".into(), false, false, false, false);
                for c in "#calculus and #x1 ".chars() {
                    ui.invoke_doc_key(c.to_string().into(), false, false, false, false);
                }
                let tags = st.borrow().ws.as_ref().unwrap().page.tags.clone();
                assert_eq!(tags, vec!["kept", "calculus", "x1"], "only finished tags are added");
                // deleting the tag text again removes a tag that was only added while typing here
                for _ in 0.."and #x1 ".len() + "calculus ".len() + 1 {
                    ui.invoke_doc_key("\u{8}".into(), false, false, false, false);
                }
                let s = st.borrow();
                assert_eq!(s.ws.as_ref().unwrap().page.tags, vec!["kept"], "{:?}", s.ws.as_ref().unwrap().page.doc.as_ref().map(|d| d.plain_text()));
            }
            "plugins" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    s.tool = Tool::TextBox;
                    let dir = plugins_dir().join("starter-snippets");
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("plugin.json"), include_str!("../starter/plugin.json")).unwrap();
                    s.reload_plugins();
                    assert_eq!(s.plugins.len(), 1, "{:?}", s.plugin_errors);
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("= Standup"));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                ui.invoke_run_command("plugin:starter-snippets:snippet:meeting".into());
                {
                    let s = st.borrow();
                    let d = s.ws.as_ref().and_then(|w| w.page.doc.as_ref()).unwrap();
                    assert!(d.plain_text().contains("Agenda") && d.paras.iter().any(|p| matches!(p.kind, notedoc::ParaKind::Check { .. })), "{:?}", d.paras);
                }
                ui.invoke_run_command("undo".into());
                assert_eq!(st.borrow().ws.as_ref().and_then(|w| w.page.doc.as_ref()).unwrap().plain_text(), "Standup", "one undo removes the whole snippet");
                ui.invoke_run_command("plugin:starter-snippets:snippet:meeting".into());
                ui.invoke_run_command("plugins".into());
            }
            "notegraph" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    if let Some(ws) = &mut s.ws {
                        let sec = ws.nb.sections[0].id.clone();
                        let spec: [(&str, &str); 8] = [("Calculus", "[[Limits]] [[Derivatives]] [[Integrals]] #math"), ("Limits", "[[Calculus]] [[Series]]"), ("Derivatives", "[[Chain rule]] [[Calculus]]"), ("Chain rule", "[[Derivatives]]"), ("Integrals", "[[Calculus]] [[Series]]"), ("Series", "[[Limits]] [[Integrals]] [[Taylor]]"), ("Lonely note", "nothing links here"), ("Reading list", "[[Calculus]] [[Missing book]]")];
                        for (t, md) in spec {
                            let mut p = store::Page::new(&store::new_id(), t, "2026-11-10");
                            p.doc = Some(notedoc::Doc::from_markup(md));
                            p.sync_blocks();
                            ws.store.create_page(&mut ws.nb, &sec, p, &ws.vault).unwrap();
                        }
                    }
                }
                ui.invoke_run_command("note-graph".into());
                {
                    let s = st.borrow();
                    let g = s.note_graph.as_ref().expect("graph built");
                    assert_eq!(g.nodes.len(), 11, "welcome page + 8 pages + 2 unresolved links");
                    assert!(g.nodes.iter().any(|n| n.title == "Taylor" && !n.exists));
                    assert_eq!(g.edges.len(), 9, "each pair of linked notes is one line");
                }
                if std::env::var_os("SCRAWL_SHOT_CLICK").is_some() {
                    // click a ghost node the way the Slint TouchArea does: it must open a new page, without a re-borrow panic
                    let (fx, fy) = st.borrow().note_graph.as_ref().and_then(|g| g.nodes.iter().find(|n| n.title == "Taylor").map(|n| (n.x, n.y))).unwrap();
                    ui.invoke_note_graph_click(fx, fy);
                    let s = st.borrow();
                    assert_eq!(s.ws.as_ref().unwrap().page.title, "Taylor", "clicking a grey dot creates and opens that page");
                    assert!(s.overlay.is_empty());
                }
            }
            "menu" => ui.invoke_open_menu("insert".into()),
            "menumore" => ui.invoke_open_menu("more".into()),
            "drawer" => ui.set_drawer_open(true),
            "tray" => ui.set_tray_panel("settings".into()),
            "trayhidden" => ui.set_tray_open(false),
            "docink" => {
                {
                    let s = &mut *st.borrow_mut();
                    s.new_editor = true;
                    if let Some(ws) = &mut s.ws {
                        ws.page.doc = Some(notedoc::Doc::from_markup("= Shopping and sums\nWrite anywhere with the pen: ink sits on top of the text and scrolls with it.\n\n- milk and bread"));
                    }
                    s.dirty |= dirty::CANVAS | dirty::TOOLS;
                    flush(ui, s);
                }
                ui.invoke_set_tool("pen".into());
                let (ox, oy) = (60.0, 330.0);
                draw(&line((ox, oy + 30.0), (ox + 60.0, oy + 30.0)));
                draw(&line((ox + 30.0, oy), (ox + 30.0, oy + 60.0)));
                draw(&line((ox + 110.0, oy + 15.0), (ox + 170.0, oy + 15.0)));
                draw(&line((ox + 110.0, oy + 45.0), (ox + 170.0, oy + 45.0)));
                ui.invoke_run_command("recognize".into());
            }
            "tablet" => ui.set_show_tablet(true),
            "notes" => {
                ui.invoke_new_page("problem set".into());
                ui.invoke_new_page("cornell notes".into());
            }
            "typehere" => {
                ui.invoke_new_page("notes".into());
                ui.invoke_set_tool("textbox".into());
                ui.invoke_pointer_down(120.0, 80.0);
                ui.invoke_pointer_up();
                ui.invoke_textbox_commit("Typed straight onto the page".into());
                ui.invoke_pointer_down(120.0, 160.0);
                ui.invoke_pointer_up();
                ui.invoke_textbox_commit("Second note: buy brandy".into());
                ui.invoke_pointer_down(130.0, 90.0);
                ui.invoke_pointer_up();
                ui.invoke_run_command("undo".into());
                ui.invoke_run_command("redo".into());
                ui.invoke_set_tool("pen".into());
                // handwriting in Text mode (the notes template opens in it): a tall l, then an o
                draw(&line((140.0, 300.0), (140.0, 360.0)));
                let o: Vec<(f32, f32)> = (0..=30).map(|i| { let a = i as f32 / 30.0 * std::f32::consts::TAU; (175.0 + 14.0 * a.cos(), 346.0 + 14.0 * a.sin()) }).collect();
                draw(&o);
                ui.invoke_run_command("recognize".into());
            }
            "justtype" => {
                ui.invoke_new_page("notes".into());
                ui.invoke_pointer_down(140.0, 90.0);
                ui.invoke_pointer_up();
                // no tool chosen: just start typing
                assert!(ui.invoke_plain_key("H".into()), "a letter starts a text box");
                assert!(ui.get_textbox_visible() && ui.get_textbox_initial() == "H" && !ui.get_textbox_select());
                ui.invoke_textbox_commit("Hello, this was typed without picking a tool".into());
                assert!(ui.get_textbox_visible(), "Enter opens the next line");
                ui.invoke_textbox_done("and Enter went to the next line; Esc finished".into());
                assert!(!ui.get_textbox_visible());
                assert!(ui.invoke_plain_key("2".into()), "digits type too when no candidates are shown");
                ui.invoke_textbox_done("2 + 2 = 4 typed as plain text".into());
            }
            "palette" => ui.invoke_run_command("palette".into()),
            "pick" => {
                ui.invoke_run_command("palette".into());
                ui.invoke_overlay_pick("symbols".into());
                ui.invoke_insert_symbol("α".into());
                ui.invoke_run_command("switcher".into());
                ui.invoke_overlay_query("".into());
            }
            "symbols" => ui.invoke_run_command("symbols".into()),
            "settings" => ui.invoke_run_command("settings".into()),
            "shortcuts" => ui.invoke_run_command("shortcuts".into()),
            "focus" => ui.invoke_run_command("focus".into()),
            "lasso" => {
                ui.invoke_set_tool("lasso".into());
                let c: Vec<(f32, f32)> = (0..=40).map(|i| { let a = i as f32 / 40.0 * std::f32::consts::TAU; (190.0 + 140.0 * a.cos(), 300.0 + 70.0 * a.sin()) }).collect();
                draw(&c);
            }
            "nodoc" => ui.invoke_run_command("toggle-doc".into()),
            _ => {}
        }
    }
    let _ = st;
}

/// Typeset each canvas text at the current zoom (cached by content, colour and size on screen).
fn canvas_texts(st: &mut State, px_scale: f32) -> Vec<(f32, f32, Arc<tiny_skia::Pixmap>)> {
    let Some(ws) = &st.ws else { return vec![] };
    let light = !st.settings.colors().is_dark();
    let mut out = Vec::new();
    for t in &ws.page.texts {
        let [r, g, b, _] = t.color;
        // same rule as ink: near-white text is shown dark on light paper
        let (r, g, b) = if light && r > 180 && g > 180 && b > 180 { (0x1f, 0x23, 0x35) } else { (r, g, b) };
        let px_per_pt = t.size * st.view.scale * px_scale / 11.0;
        let key = format!("{}\u{0}{r},{g},{b}\u{0}{:.3}", t.text, px_per_pt);
        if !st.text_cache.contains_key(&key) {
            let body: Vec<String> = t.text.lines().map(store::typst_escape).collect();
            let src = format!("#set page(width: auto, height: auto, margin: 1pt, fill: none)\n#set text(size: 11pt, fill: rgb({r}, {g}, {b}))\n{}\n", body.join(" \\\n"));
            let pm = render::compile(&src, &[])
                .ok()
                .and_then(|d| render::png(&d, 0, px_per_pt.clamp(0.2, 40.0)))
                .and_then(|png| tiny_skia::Pixmap::decode_png(&png).ok());
            if let Some(pm) = pm {
                if st.text_cache.len() > 400 {
                    st.text_cache.clear();
                }
                st.text_cache.insert(key.clone(), Arc::new(pm));
            }
        }
        if let Some(pm) = st.text_cache.get(&key) {
            out.push((t.x, t.y, pm.clone()));
        }
    }
    out
}

/// The page's text paragraphs (new document editor) as pixmaps in page coordinates.
fn doc_layer(st: &mut State, px_scale: f32, canvas_w: f32, canvas_h: f32) -> Vec<(f32, f32, Arc<tiny_skia::Pixmap>)> {
    if !st.new_editor {
        return vec![];
    }
    let colors = st.settings.colors();
    let c = docview::Colors { text: colors.text, link: colors.accent, muted: colors.muted, accent: colors.accent };
    let zoom = st.view.scale;
    let col_w = docedit::DocEdit::column_width(canvas_w, zoom);
    {
        let Some(ws) = &mut st.ws else { return vec![] };
        if ws.page.doc.is_none() {
            ws.page.ensure_doc();
            ws.touch();
        }
        if let Some(d) = ws.page.doc.as_mut() {
            notedoc::ensure_not_empty(d);
        }
        let Some(doc) = ws.page.doc.as_ref() else { return vec![] };
        for p in &doc.paras {
            if let notedoc::ParaKind::Image { file, .. } = &p.kind {
                if !st.dedit.objs.media.iter().any(|(n, _)| n == file) {
                    if let Some(bytes) = ws.media(file) {
                        st.dedit.objs.media.push((file.clone(), bytes));
                    }
                }
            }
        }
        st.dedit.relayout(doc, col_w, canvas_w, zoom);
    }
    docedit::sync_anchors(st);
    let Some(doc) = st.ws.as_ref().and_then(|w| w.page.doc.as_ref()) else { return vec![] };
    let (y0, y1) = (-st.view.oy / zoom, (canvas_h - st.view.oy) / zoom);
    st.dedit.visible(doc, y0, y1, zoom * px_scale, &c)
}

/// The handwriting suggestion chip next to the ink that was just written.
fn update_chip(ui: &MainWindow, st: &State) {
    let has_text = if st.text_mode { !st.text_result.trim().is_empty() } else { !current_expr(st).is_empty() };
    let show = st.new_editor && !st.chip_dismissed && !st.training && !st.typing && has_text && !st.session.symbols.is_empty() && st.tool != Tool::TextBox;
    ui.set_chip_visible(show);
    ui.set_mode_auto(st.mode_auto);
    if !show {
        return;
    }
    let (mut x0, mut y1) = (f32::MAX, f32::MIN);
    for s in &st.session.symbols {
        x0 = x0.min(s.rect.x0);
        y1 = y1.max(s.rect.y1);
    }
    ui.set_chip_x(x0 * st.view.scale + st.view.ox);
    ui.set_chip_y(y1 * st.view.scale + st.view.oy + 14.0);
    ui.set_chip_math(!st.text_mode);
    ui.set_chip_text(format!("“{}”", st.text_result.trim().replace('\n', " / ")).into());
}

/// Does this reading look like ordinary words (so the ink is handwriting, not a formula)?
fn looks_like_words(text: &str, sp: Option<&pagedoc::Speller>, symbols: usize) -> bool {
    let Some(sp) = sp else { return false };
    let words: Vec<&str> = text.split_whitespace().collect();
    symbols >= 2 && !words.is_empty() && words.iter().all(|w| w.chars().count() >= 2 && w.chars().all(char::is_alphabetic) && sp.is_word(&w.to_lowercase()))
}

/// Plugins live next to the notes: `Scrawl/plugins/<name>/plugin.json`.
pub fn plugins_dir() -> std::path::PathBuf {
    workspace::default_root().join("plugins")
}

/// First start: put the starter snippet pack in place so the plugin menu is not empty.
fn ensure_starter_plugin() {
    let dir = plugins_dir();
    if dir.exists() || std::env::var_os("SCRAWL_SHOT").is_some() {
        return;
    }
    let target = dir.join("starter-snippets");
    if std::fs::create_dir_all(&target).is_ok() {
        let _ = std::fs::write(target.join("plugin.json"), include_str!("../starter/plugin.json"));
    }
}

impl State {
    pub fn reload_plugins(&mut self) {
        ensure_starter_plugin();
        let (ok, errors) = plugin::load_all(&plugins_dir());
        self.plugins = ok;
        self.plugin_errors = errors;
    }

    /// (id, title, plugin name) of every command of the plugins that are switched on.
    pub fn plugin_commands(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        for p in self.plugins.iter().filter(|p| !self.settings.disabled_plugins.contains(&p.manifest.name)) {
            for c in &p.manifest.commands {
                out.push((format!("plugin:{}:{}", p.manifest.name, c.id), c.title.clone(), p.manifest.name.clone()));
            }
            for sn in &p.manifest.snippets {
                out.push((format!("plugin:{}:snippet:{}", p.manifest.name, sn.id), sn.title.clone(), p.manifest.name.clone()));
            }
        }
        out
    }
}

/// Redraw the note graph picture and its labels for the current card size.
pub fn refresh_note_graph(ui: &MainWindow, st: &State) {
    let Some(g) = &st.note_graph else { return };
    let scale = ui.window().scale_factor();
    let c = st.settings.colors();
    let (w, h) = st.ng_size;
    let Some((pm, labels)) = g.render(w, h, scale, c.text, c.muted, c.accent) else { return };
    let mut buf = SharedPixelBuffer::<slint::Rgba8Pixel>::new(pm.width(), pm.height());
    buf.make_mut_bytes().copy_from_slice(pm.data());
    ui.set_note_graph_frame(slint::Image::from_rgba8_premultiplied(buf));
    ui.set_note_graph_labels(model(labels.into_iter().map(|(x, y, t)| LabelRow { x, y, text: t.into() }).collect()));
}

#[cfg(test)]
mod auto_mode_tests {
    use super::*;

    #[test]
    fn words_are_told_from_formulas() {
        let sp = pagedoc::Speller::new();
        assert!(looks_like_words("slate", sp.as_ref(), 5));
        assert!(looks_like_words("hello world", sp.as_ref(), 10));
        assert!(!looks_like_words("x2", sp.as_ref(), 2));
        assert!(!looks_like_words("q", sp.as_ref(), 1));
        assert!(!looks_like_words("xqz", sp.as_ref(), 3));
        assert!(!looks_like_words("slate", None, 5));
    }
}
