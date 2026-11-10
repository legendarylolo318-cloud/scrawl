mod graphview;
mod headless;
mod raster;
mod selftest;
mod session;
mod tablet;
mod workspace;

use graphview::GraphView;
use ink::{is_scribble, smooth_stroke, Document, EraserMode, InkMode, PenKind, Point, SmoothingConfig, Stroke};
use raster::{Paper, PaperKind, View};
use session::Session;
use slint::{ComponentHandle, ModelRc, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;
use workspace::Workspace;

slint::include_modules!();

const PRIORITY: usize = recog::PRIORITY_MIN_SAMPLES;
const PALETTE: [[u8; 3]; 6] = [[0xc0, 0xca, 0xf5], [0xf7, 0x76, 0x8e], [0x9e, 0xce, 0x6a], [0x7a, 0xa2, 0xf7], [0xe0, 0xaf, 0x68], [0xbb, 0x9a, 0xf7]];

#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
}

pub struct State {
    pub doc: Document,
    pub view: View,
    pub size: (u32, u32),
    pub tool: Tool,
    pub mode: InkMode,
    pub eraser_mode: EraserMode,
    pub smoothing: f32,
    pub width: f32,
    pub color: usize,
    pub live: Vec<Point>,
    pub eraser_path: Vec<(f32, f32)>,
    pub down_at: Option<Instant>,
    pub t0_ms: u64,
    pub epoch: Instant,
    pub session: Session,
    pub recog_delay_ms: u64,
    pub paper: Paper,
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
    pub tablet: tablet::Tablet,
}

impl State {
    fn new() -> Self {
        let ws = Workspace::open(&workspace::default_root()).ok();
        let mut st = Self {
            doc: Document::new(),
            view: View::default(),
            size: (800, 600),
            tool: Tool::Pen,
            mode: InkMode::MathText,
            eraser_mode: EraserMode::Stroke,
            smoothing: 0.5,
            width: 3.0,
            color: 0,
            live: Vec::new(),
            eraser_path: Vec::new(),
            down_at: None,
            t0_ms: 0,
            epoch: Instant::now(),
            session: Session::new(session::config_dir().map(|d| d.join("personal.json"))),
            recog_delay_ms: 600,
            paper: Paper::default(),
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
            tablet: tablet::Tablet::default(),
        };
        st.load_page_into_doc();
        st
    }

    fn pen_color(&self) -> [u8; 4] {
        let c = PALETTE[self.color];
        [c[0], c[1], c[2], if self.tool == Tool::Highlighter { 0x66 } else { 0xff }]
    }

    fn live_stroke(&self) -> Stroke {
        Stroke {
            id: u64::MAX,
            points: self.live.clone(),
            color: self.pen_color(),
            width: self.width,
            kind: if self.tool == Tool::Highlighter { PenKind::Highlighter } else { PenKind::Pen },
            mode: self.mode,
            t0_ms: self.t0_ms,
        }
    }

    fn now_ms(&self) -> u64 {
        store::now_ms()
    }

    /// Replace the canvas with the current page's saved ink and paper.
    fn load_page_into_doc(&mut self) {
        if let Some(ws) = &self.ws {
            self.doc = Document::with_strokes(ws.page.strokes.clone());
            self.paper.kind = match ws.page.paper.kind.as_str() {
                "blank" => PaperKind::Blank,
                "grid" => PaperKind::Grid,
                "dots" => PaperKind::Dots,
                "engineering" => PaperKind::Engineering,
                "polar" => PaperKind::Polar,
                _ => PaperKind::Lined,
            };
            self.paper.spacing = ws.page.paper.spacing;
        }
        self.session.symbols.clear();
        self.session.selected = None;
        self.editor = input::Editor::default();
        self.consumed.clear();
        self.typed_since_recog = false;
    }

    /// Push canvas ink and paper into the workspace (autosave picks it up).
    fn sync_ws(&mut self) {
        let now = self.now_ms();
        let (strokes, kind, spacing) = (self.doc.strokes.clone(), self.paper.kind.name().to_lowercase(), self.paper.spacing);
        if let Some(ws) = &mut self.ws {
            ws.set_strokes(strokes, now);
            ws.set_paper(&kind, spacing);
        }
    }
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn refresh(ui: &MainWindow, st: &State) {
    let scale = ui.window().scale_factor();
    let (w, h) = st.size;
    let live = (!st.live.is_empty() && st.tool != Tool::Eraser).then(|| st.live_stroke());
    let img = raster::render(st.doc.strokes.iter().chain(live.iter()), st.view, (w as f32 * scale) as u32, (h as f32 * scale) as u32, scale, [0x1a, 0x1b, 0x26], st.paper);
    ui.set_frame(img);
    ui.set_can_undo(st.doc.can_undo());
    ui.set_can_redo(st.doc.can_redo());
}

fn refresh_sidebar(ui: &MainWindow, st: &State) {
    let Some(ws) = &st.ws else { return };
    let rows: Vec<PageRow> = ws.rows().into_iter().map(|r| PageRow { id: r.id.into(), title: r.title.into(), detail: r.section.into(), current: r.is_current }).collect();
    ui.set_pages(model(rows));
    ui.set_page_title(ws.page.title.as_str().into());
}

fn refresh_graph(ui: &MainWindow, st: &State) {
    let f = st.graph.frame();
    let mut buf = SharedPixelBuffer::<slint::Rgba8Pixel>::new(f.width, f.height);
    buf.make_mut_bytes().copy_from_slice(&f.rgba);
    ui.set_graph_frame(slint::Image::from_rgba8_premultiplied(buf));
    ui.set_graph_labels(model(f.labels.iter().map(|l| LabelRow { x: l.0, y: l.1, text: l.2.as_str().into() }).collect()));
    let params: Vec<ParamRow> = st.graph.graph.params.iter().map(|(k, p)| ParamRow { name: k.as_str().into(), value: p.value as f32, min: p.min as f32, max: p.max as f32 }).collect();
    ui.set_graph_params(model(params));
    ui.set_graph_info(st.graph.info().into());
}

fn update_strip(ui: &MainWindow, st: &State) {
    let cands: Vec<SharedString> = st.session.selected.and_then(|i| st.session.symbols.get(i)).map(|s| s.cands.iter().map(|c| SharedString::from(c.label.as_str())).collect()).unwrap_or_default();
    ui.set_candidates(model(cands));
    update_expr(ui, st);
}

/// The expression line: typed/drawn expression as Typst text plus a Typst-rendered image.
fn update_expr(ui: &MainWindow, st: &State) {
    let e = if st.typing || st.typed_since_recog || !st.editor.expr().is_empty() { st.editor.expr() } else { st.session.expression() };
    if e.is_empty() {
        ui.set_expr_text("".into());
        ui.set_has_expr_image(false);
        return;
    }
    let typst = e.to_typst();
    ui.set_expr_text(format!("= {typst}").into());
    match st.math_cache.render(&typst, 16.0, [0xc0, 0xca, 0xf5], ui.window().scale_factor()) {
        Ok(r) => {
            ui.set_expr_image(slint::Image::from_rgba8_premultiplied(SharedPixelBuffer::clone_from_slice(&r.rgba_premul, r.width, r.height)));
            ui.set_has_expr_image(true);
        }
        Err(_) => ui.set_has_expr_image(false),
    }
}

fn update_training_ui(ui: &MainWindow, st: &mut State) {
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
fn run_recognition(ui: &MainWindow, st: &mut State) {
    if st.training {
        let Some(label) = st.train_labels.get(st.train_idx).cloned() else { return };
        match st.session.train(&label, &st.doc.strokes) {
            Ok(ids) if !ids.is_empty() => {
                st.doc.remove_ids(&ids);
                ui.set_status("Saved".into());
            }
            Ok(_) => {}
            Err(e) => ui.set_status(e.into()),
        }
        update_training_ui(ui, st);
        return;
    }
    match st.session.recognize(&st.doc.strokes) {
        Ok(()) => {
            ui.set_status(if st.session.symbols.is_empty() { "Nothing to recognize".into() } else { "".into() });
            feed_editor(st);
        }
        Err(e) => ui.set_status(format!("Recognizer failed to load: {e}").into()),
    }
    update_strip(ui, st);
    tablet::push_state(st);
}

fn main() -> Result<(), slint::PlatformError> {
    if std::env::args().any(|a| a == "--self-test") {
        std::process::exit(selftest::run());
    }
    let shot_path = std::env::var("SCRAWL_SHOT").ok();
    let shot_size: (u32, u32) = std::env::var("SCRAWL_SHOT_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1500, 900));
    let shot_window = shot_path.as_ref().map(|_| headless::install(shot_size.0, shot_size.1));
    let ui = MainWindow::new()?;
    let st = Rc::new(RefCell::new(State::new()));
    let weak = ui.as_weak();

    {
        let s = st.borrow();
        ui.set_templates(model(store::templates::BUILTIN.iter().map(|t| SharedString::from(*t)).collect()));
        ui.set_export_formats(model(["PDF", "PNG", "Markdown", "Typst", "LaTeX", "HTML"].iter().map(|t| SharedString::from(*t)).collect()));
        ui.set_paper_name(s.paper.kind.name().into());
        ui.set_paper_spacing(s.paper.spacing);
        refresh_sidebar(&ui, &s);
        refresh_graph(&ui, &s);
        if let Some(msg) = s.ws.as_ref().and_then(|w| w.recovered.clone()) {
            ui.set_status(msg.into());
        }
    }

    // Run a state mutation, then redraw. Closure params after `s` mirror the Slint callback args.
    macro_rules! with {
        (|$ui:ident, $s:ident $(, $a:pat_param = $t:ty)*| $body:expr) => {{
            let st = st.clone();
            let weak = weak.clone();
            move |$($a: $t),*| {
                let $ui = weak.upgrade().unwrap();
                {
                    let $s = &mut *st.borrow_mut();
                    let _ = &$ui;
                    $body
                }
                refresh(&$ui, &st.borrow());
            }
        }};
    }
    // Same, but without redrawing the ink canvas (graph / sidebar / tablet callbacks).
    macro_rules! quiet {
        (|$ui:ident, $s:ident $(, $a:pat_param = $t:ty)*| $body:expr) => {{
            let st = st.clone();
            let weak = weak.clone();
            move |$($a: $t),*| {
                let $ui = weak.upgrade().unwrap();
                let $s = &mut *st.borrow_mut();
                let _ = &$ui;
                $body
            }
        }};
    }

    ui.on_canvas_resized(with!(|_ui, s, w = f32, h = f32| {
        s.size = (w.max(1.0) as u32, h.max(1.0) as u32);
    }));

    ui.on_pointer_down(with!(|_ui, s, x = f32, y = f32| {
        let (px, py) = s.view.to_page(x, y);
        s.down_at = Some(Instant::now());
        s.t0_ms = s.epoch.elapsed().as_millis() as u64;
        match s.tool {
            Tool::Eraser => s.eraser_path = vec![(px, py)],
            _ => s.live = vec![Point::new(px, py, 0)],
        }
    }));

    ui.on_pointer_move(with!(|_ui, s, x = f32, y = f32| {
        let Some(t0) = s.down_at else { return };
        let (px, py) = s.view.to_page(x, y);
        match s.tool {
            Tool::Eraser => s.eraser_path.push((px, py)),
            _ => {
                let t = t0.elapsed().as_millis() as u32;
                s.live.push(Point::new(px, py, t));
            }
        }
    }));

    let up_inner = with!(|ui, s| {
        if s.down_at.take().is_none() {
            return;
        }
        match s.tool {
            Tool::Eraser => {
                let path = std::mem::take(&mut s.eraser_path);
                let r = (s.width * 2.0 + 4.0) / s.view.scale;
                let mode = s.eraser_mode;
                s.doc.erase_path(&path, r, mode);
            }
            tool => {
                let raw = std::mem::take(&mut s.live);
                if tool == Tool::Pen && is_scribble(&raw) {
                    let path: Vec<(f32, f32)> = raw.iter().map(|p| (p.x, p.y)).collect();
                    let n = s.doc.erase_path(&path, s.width + 3.0, EraserMode::Stroke);
                    ui.set_status(format!("Scribble deleted {n} stroke(s)").into());
                } else if !raw.is_empty() {
                    let pts = smooth_stroke(&raw, SmoothingConfig { strength: s.smoothing });
                    let mut stroke = s.live_stroke();
                    stroke.points = pts;
                    stroke.id = 0;
                    s.doc.add_stroke(stroke);
                    ui.set_status("".into());
                }
            }
        }
        s.sync_ws();
    });
    {
        let w = weak.clone();
        ui.on_pointer_up(move || {
            up_inner();
            if let Some(ui) = w.upgrade() {
                ui.invoke_pen_lifted();
            }
        });
    }

    ui.on_zoom(with!(|_ui, s, f = f32, x = f32, y = f32| s.view.zoom_at(f, x, y)));
    ui.on_pan(with!(|_ui, s, dx = f32, dy = f32| {
        s.view.ox += dx;
        s.view.oy += dy;
    }));
    ui.on_set_tool(with!(|ui, s, t = SharedString| {
        s.tool = match t.as_str() {
            "highlighter" => Tool::Highlighter,
            "eraser" => Tool::Eraser,
            _ => Tool::Pen,
        };
        ui.set_tool(t);
    }));
    ui.on_set_mode(with!(|ui, s, m = SharedString| {
        s.mode = if m == "drawing" { InkMode::Drawing } else { InkMode::MathText };
        ui.set_mode(m);
    }));
    ui.on_toggle_pixel_erase(with!(|ui, s| {
        s.eraser_mode = if s.eraser_mode == EraserMode::Stroke { EraserMode::Pixel } else { EraserMode::Stroke };
        ui.set_pixel_erase(s.eraser_mode == EraserMode::Pixel);
    }));
    ui.on_undo(with!(|_ui, s| {
        s.doc.undo();
        s.sync_ws();
    }));
    ui.on_redo(with!(|_ui, s| {
        s.doc.redo();
        s.sync_ws();
    }));
    ui.on_clear_all(with!(|ui, s| {
        let ids: Vec<u64> = s.doc.strokes.iter().map(|x| x.id).collect();
        s.doc.remove_ids(&ids);
        s.session.symbols.clear();
        s.session.selected = None;
        s.editor = input::Editor::default();
        s.consumed.clear();
        s.typed_since_recog = false;
        s.sync_ws();
        update_strip(&ui, s);
    }));
    ui.on_set_smoothing(with!(|ui, s, v = f32| {
        s.smoothing = v;
        ui.set_smoothing(v);
    }));
    ui.on_set_width(with!(|ui, s, v = f32| {
        s.width = v;
        ui.set_stroke_width(v);
    }));
    ui.on_set_color(with!(|_ui, s, i = i32| {
        s.color = (i as usize).min(PALETTE.len() - 1);
    }));
    ui.on_cycle_paper(with!(|ui, s| {
        s.paper.kind = s.paper.kind.next();
        ui.set_paper_name(s.paper.kind.name().into());
        s.sync_ws();
    }));
    ui.on_set_spacing(with!(|ui, s, v = f32| {
        s.paper.spacing = v;
        ui.set_paper_spacing(v);
        s.sync_ws();
    }));
    ui.on_recognize_now(with!(|ui, s| run_recognition(&ui, s)));
    ui.on_set_delay(with!(|ui, s, v = f32| {
        s.recog_delay_ms = v as u64;
        ui.set_recog_delay(v);
    }));
    ui.on_choose(with!(|ui, s, i = i32| {
        if s.session.choose(i.max(0) as usize, &s.doc.strokes) {
            if !s.typed_since_recog {
                s.editor.set_expr(s.session.expression());
            }
            update_strip(&ui, s);
            ui.set_status("Correction saved to your handwriting profile".into());
            tablet::push_state(s);
        }
    }));
    ui.on_toggle_training(with!(|ui, s| {
        s.training = !s.training;
        ui.set_training(s.training);
        if s.training {
            update_training_ui(&ui, s);
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

    // ---- typing mode ------------------------------------------------------------------------
    ui.on_toggle_typing(with!(|ui, s| {
        s.typing = !s.typing;
        ui.set_typing(s.typing);
        update_expr(&ui, s);
    }));
    ui.on_type_text(with!(|ui, s, t = SharedString| {
        s.editor.insert_str(&t);
        s.typed_since_recog = true;
        update_expr(&ui, s);
        tablet::push_state(s);
    }));
    ui.on_type_nav(with!(|ui, s, k = SharedString| {
        tablet::nav(s, k.as_str());
        s.typed_since_recog = true;
        update_expr(&ui, s);
    }));

    // ---- notes: pages, search, export ----------------------------------------------------------
    ui.on_open_page(with!(|ui, s, id = SharedString| {
        s.sync_ws();
        let now = s.now_ms();
        let res = s.ws.as_mut().map(|w| w.open_page(&id, now));
        match res {
            Some(Ok(())) => {
                s.load_page_into_doc();
                ui.set_search_active(false);
            }
            Some(Err(e)) => ui.set_status(format!("Could not open page: {e}").into()),
            None => {}
        }
        refresh_sidebar(&ui, s);
        ui.set_paper_name(s.paper.kind.name().into());
        update_strip(&ui, s);
    }));
    ui.on_new_page(with!(|ui, s, t = SharedString| {
        s.sync_ws();
        let now = s.now_ms();
        let res = s.ws.as_mut().map(|w| w.new_page(&t, now));
        if let Some(Err(e)) = res {
            ui.set_status(format!("Could not create page: {e}").into());
        } else {
            s.load_page_into_doc();
        }
        refresh_sidebar(&ui, s);
        update_strip(&ui, s);
    }));
    ui.on_delete_page(with!(|ui, s| {
        let now = s.now_ms();
        let res = s.ws.as_mut().map(|w| w.delete_current(now));
        match res {
            Some(Ok(())) => {
                s.load_page_into_doc();
                ui.set_status("Page moved to the trash (kept 30 days)".into());
            }
            Some(Err(e)) => ui.set_status(format!("Could not delete: {e}").into()),
            None => {}
        }
        refresh_sidebar(&ui, s);
        update_strip(&ui, s);
    }));
    ui.on_rename_page(with!(|ui, s, t = SharedString| {
        if let Some(w) = &mut s.ws {
            w.rename(&t);
        }
        refresh_sidebar(&ui, s);
    }));
    ui.on_do_search(with!(|ui, s, q = SharedString| {
        if q.trim().is_empty() {
            ui.set_search_active(false);
            return;
        }
        let hits = s.ws.as_mut().map(|w| w.search(q.trim())).unwrap_or_default();
        let rows: Vec<PageRow> = hits.into_iter().map(|h| PageRow { id: h.page_id.into(), title: h.title.into(), detail: h.snippet.replace("<b>", "").replace("</b>", "").into(), current: false }).collect();
        ui.set_search_results(model(rows));
        ui.set_search_active(true);
    }));
    ui.on_export_page(with!(|ui, s, f = SharedString| {
        s.sync_ws();
        let fmt = match f.as_str() {
            "PNG" => export::Format::Png,
            "Markdown" => export::Format::Markdown,
            "Typst" => export::Format::Typst,
            "LaTeX" => export::Format::Latex,
            "HTML" => export::Format::Html,
            _ => export::Format::Pdf,
        };
        let now = s.now_ms();
        let dir = workspace::default_root().join("exports");
        let res = s.ws.as_mut().map(|w| w.export_current(fmt, &dir, now));
        match res {
            Some(Ok(p)) => ui.set_status(format!("Exported to {}", p.display()).into()),
            Some(Err(e)) => ui.set_status(format!("Export failed: {e}").into()),
            None => {}
        }
    }));

    // ---- graph -------------------------------------------------------------------------------
    ui.on_toggle_graph(quiet!(|ui, s| {
        let on = !ui.get_graph_mode();
        ui.set_graph_mode(on);
        if on {
            refresh_graph(&ui, s);
        }
    }));
    ui.on_graph_text_edited(quiet!(|ui, s, t = SharedString| {
        s.graph.set_text(&t);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_set_param(quiet!(|ui, s, n = SharedString, v = f32| {
        s.graph.graph.set_param(&n, v as f64);
        let (frame, info) = (s.graph.frame(), s.graph.info());
        let mut buf = SharedPixelBuffer::<slint::Rgba8Pixel>::new(frame.width, frame.height);
        buf.make_mut_bytes().copy_from_slice(&frame.rgba);
        ui.set_graph_frame(slint::Image::from_rgba8_premultiplied(buf));
        ui.set_graph_info(info.into());
    }));
    ui.on_graph_pointer(quiet!(|ui, s, kind = i32, x = f32, y = f32| {
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
    ui.on_graph_zoom(quiet!(|ui, s, f = f32, x = f32, y = f32| {
        let (w, h) = (s.graph.size.0 as f64, s.graph.size.1 as f64);
        s.graph.vp.zoom_at(f as f64, x as f64, y as f64, w, h);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_resized(quiet!(|ui, s, w = f32, h = f32| {
        s.graph.size = (w.max(50.0) as u32, h.max(50.0) as u32);
        refresh_graph(&ui, s);
    }));
    ui.on_graph_run_matlab(quiet!(|ui, s, script = SharedString| {
        let out = s.graph.run_matlab(&script);
        ui.set_graph_out(out.into());
        refresh_graph(&ui, s);
    }));
    ui.on_graph_load_mat(quiet!(|ui, s, path = SharedString| {
        let out = match std::fs::read(path.trim()) {
            Ok(b) => s.graph.load_mat(&b),
            Err(e) => format!("Could not read {}: {e}", path.trim()),
        };
        ui.set_graph_out(out.into());
    }));

    // ---- tablet --------------------------------------------------------------------------------
    tablet::wire(&ui, &st);

    // ---- auto-recognize (restart a single-shot timer on every pen-up) ----------------------------
    let timer = Rc::new(slint::Timer::default());
    {
        let (st2, weak2, timer2) = (st.clone(), weak.clone(), timer.clone());
        ui.on_pen_lifted(move || {
            let (st3, weak3) = (st2.clone(), weak2.clone());
            let delay = st2.borrow().recog_delay_ms;
            timer2.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(delay), move || {
                if let Some(ui) = weak3.upgrade() {
                    run_recognition(&ui, &mut st3.borrow_mut());
                    refresh(&ui, &st3.borrow());
                }
            });
        });
    }

    // ---- autosave: crash-recovery snapshot soon after a change, full save after 2 s idle --------
    let autosave = Rc::new(slint::Timer::default());
    {
        let (st2, weak2) = (st.clone(), weak.clone());
        autosave.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
            let mut s = st2.borrow_mut();
            let now = s.now_ms();
            if let Some(ws) = &mut s.ws {
                if let Ok(true) = ws.tick(now) {
                    if let Some(ui) = weak2.upgrade() {
                        refresh_sidebar(&ui, &s);
                    }
                }
            }
        });
    }

    // Save and stop the tablet server when the window closes.
    {
        let st2 = st.clone();
        ui.window().on_close_requested(move || {
            let mut s = st2.borrow_mut();
            s.sync_ws();
            let now = s.now_ms();
            if let Some(ws) = &mut s.ws {
                let _ = ws.save(now);
            }
            s.tablet.stop();
            slint::CloseRequestResponse::HideWindow
        });
    }

    if let (Some(path), Some(win)) = (shot_path, shot_window) {
        run_scene(&ui, &st, &std::env::var("SCRAWL_SHOT_SCENE").unwrap_or_default());
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
    ui.invoke_canvas_resized(1200.0, 640.0);
    for scene in scene.split(',') {
        match scene {
            "draw" => {
                // "+ =" drawn with real pointer callbacks, then recognized
                draw(&line((100.0, 300.0), (160.0, 300.0)));
                draw(&line((130.0, 270.0), (130.0, 330.0)));
                draw(&line((220.0, 285.0), (290.0, 285.0)));
                draw(&line((220.0, 315.0), (290.0, 315.0)));
                ui.invoke_recognize_now();
            }
            "typing" => {
                ui.invoke_toggle_typing();
                for c in "alpha + int".chars() {
                    ui.invoke_type_text(c.to_string().into());
                }
                ui.invoke_type_text(" ".into());
                for c in "_0^1 x^2/3".chars() {
                    ui.invoke_type_text(c.to_string().into());
                }
            }
            "graph" => {
                ui.invoke_toggle_graph();
                ui.invoke_graph_resized(1000.0, 560.0);
                ui.invoke_graph_text_edited("y = a sin(x)\ny = x^2/4 - 2\nx^2 + y^2 = 9\na = 2".replace("\\n", "\n").into());
            }
            "matlab" => {
                ui.invoke_toggle_graph();
                ui.invoke_graph_resized(1000.0, 560.0);
                ui.invoke_graph_text_edited("".into());
                ui.invoke_graph_run_matlab("x = linspace(0, 2*pi, 200);\ny = sin(x) .* exp(-x/4);\nplot(x, y); title('Damped sine')".replace("\\n", "\n").into());
            }
            "tablet" => ui.set_show_tablet(true),
            "notes" => {
                ui.invoke_new_page("problem set".into());
                ui.invoke_new_page("cornell notes".into());
            }
            _ => {}
        }
    }
    let _ = st;
}
