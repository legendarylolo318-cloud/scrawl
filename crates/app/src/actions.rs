//! Everything the UI can do, as plain functions over `State`. Commands (palette, toolbar buttons,
//! shortcuts) all funnel through `run`.

use crate::*;
use std::path::{Path, PathBuf};

fn open_overlay(ui: &MainWindow, s: &mut State, name: &str) {
    s.overlay = name.into();
    ui.set_overlay(name.into());
}

fn close_overlay(ui: &MainWindow, s: &mut State) {
    s.overlay.clear();
    s.prompt = None;
    // Do not call the Slint close-overlay() here: it invokes overlay-closed, which would borrow
    // State again while we hold it (a RefCell panic). Reset everything directly instead.
    s.rebinding = None;
    s.history_pick = None;
    ui.set_has_preview(false);
    ui.set_overlay("".into());
    ui.invoke_focus_canvas();
}

fn ask(ui: &MainWindow, s: &mut State, p: Prompt, title: &str, hint: &str, password: bool) {
    s.prompt = Some(p);
    ui.set_prompt_title(title.into());
    ui.set_prompt_hint(hint.into());
    ui.set_prompt_password(password);
    open_overlay(ui, s, "prompt");
}

/// Native file dialog when the desktop provides one; otherwise ask for a path in a prompt.
fn pick_file(ui: &MainWindow, s: &mut State, title: &str, exts: &[&str], fallback: Prompt) -> Option<PathBuf> {
    let has_portal = !cfg!(target_os = "linux") || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some();
    if has_portal && std::env::var_os("SCRAWL_SHOT").is_none() {
        return rfd::FileDialog::new().set_title(title).add_filter(title, exts).pick_file();
    }
    ask(ui, s, fallback, title, &format!("Full path to the file ({})", exts.join(", ")), false);
    None
}

fn clipboard_text(t: &str) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(t.to_string())).map_err(|e| format!("Clipboard unavailable: {e}"))
}

fn clipboard_png(png: &[u8]) -> Result<(), String> {
    let img = image::load_from_memory(png).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() }))
        .map_err(|e| format!("Clipboard unavailable: {e}"))
}

// ---- pointer ---------------------------------------------------------------------------------

pub fn set_tool(s: &mut State, t: &str) {
    s.tool = match t {
        "highlighter" => Tool::Highlighter,
        "eraser" => Tool::Eraser,
        "lasso" => Tool::Lasso,
        "textbox" => Tool::TextBox,
        _ => Tool::Pen,
    };
    s.textbox = None;
    if s.tool != Tool::Lasso {
        s.selection.clear();
        s.dirty |= dirty::CANVAS;
    }
    s.dirty |= dirty::TOOLS;
}

fn selection_bbox(s: &State) -> Option<ink::Rect> {
    let pts: Vec<Point> = s.doc.strokes.iter().filter(|x| s.selection.contains(&x.id)).flat_map(|x| x.points.iter().copied()).collect();
    ink::bbox(&pts)
}

pub fn pointer_down(s: &mut State, x: f32, y: f32) {
    let (px, py) = s.view.to_page(x, y);
    s.type_cursor = Some((px, py));
    s.down_at = Some(Instant::now());
    s.t0_ms = s.epoch.elapsed().as_millis() as u64;
    match s.tool {
        Tool::Eraser => s.eraser_path = vec![(px, py)],
        Tool::TextBox => {
            let hit = text_at(s, px, py);
            let (x, y) = hit.and_then(|id| s.ws.as_ref().and_then(|w| w.page.texts.iter().find(|t| t.id == id).map(|t| (t.x, t.y)))).unwrap_or((px, py));
            s.textbox = Some((hit, x, y));
            s.down_at = None;
            s.dirty |= dirty::TOOLS;
            return;
        }
        Tool::Lasso => {
            let inside = selection_bbox(s).is_some_and(|r| px >= r.x0 - 8.0 && px <= r.x1 + 8.0 && py >= r.y0 - 8.0 && py <= r.y1 + 8.0);
            if inside {
                s.drag_from = Some((px, py));
                s.drag_total = (0.0, 0.0);
            } else {
                s.selection.clear();
                s.lasso = vec![(px, py)];
            }
        }
        _ => s.live = vec![Point::new(px, py, 0)],
    }
    s.dirty |= dirty::CANVAS;
}

pub fn pointer_move(s: &mut State, x: f32, y: f32) {
    let Some(t0) = s.down_at else { return };
    let (px, py) = s.view.to_page(x, y);
    match s.tool {
        Tool::Eraser => s.eraser_path.push((px, py)),
        Tool::Lasso => {
            if let Some((lx, ly)) = s.drag_from {
                // live preview of the move: shift the selected strokes directly, commit on pen-up
                let (dx, dy) = (px - lx, py - ly);
                for st in s.doc.strokes.iter_mut().filter(|st| s.selection.contains(&st.id)) {
                    st.points.iter_mut().for_each(|p| {
                        p.x += dx;
                        p.y += dy;
                    });
                }
                s.drag_total.0 += dx;
                s.drag_total.1 += dy;
                s.drag_from = Some((px, py));
            } else {
                s.lasso.push((px, py));
            }
        }
        _ => {
            let t = t0.elapsed().as_millis() as u32;
            s.live.push(Point::new(px, py, t));
        }
    }
    s.dirty |= dirty::CANVAS;
}

/// Returns true when recognition should be scheduled.
pub fn pointer_up(ui: &MainWindow, s: &mut State) -> bool {
    if s.down_at.take().is_none() {
        return false;
    }
    s.dirty |= dirty::CANVAS;
    match s.tool {
        Tool::Eraser => {
            let path = std::mem::take(&mut s.eraser_path);
            let r = (s.settings.width * 2.0 + 4.0) / s.view.scale;
            let mode = s.eraser_mode;
            s.doc.erase_path(&path, r, mode);
            // the eraser also removes typed text it passes over
            let hit: Vec<u64> = path.iter().filter_map(|&(x, y)| text_at(s, x, y)).collect();
            if !hit.is_empty() {
                snapshot_texts(s);
                if let Some(w) = &mut s.ws {
                    w.page.texts.retain(|t| !hit.contains(&t.id));
                    w.touch();
                }
            }
            s.sync_ws();
            true
        }
        Tool::Lasso => {
            if s.drag_from.take().is_some() {
                let (dx, dy) = s.drag_total;
                // undo the preview shift, then apply it as one undoable edit
                for st in s.doc.strokes.iter_mut().filter(|st| s.selection.contains(&st.id)) {
                    st.points.iter_mut().for_each(|p| {
                        p.x -= dx;
                        p.y -= dy;
                    });
                }
                if dx.abs() + dy.abs() > 0.5 {
                    let ids = s.selection.clone();
                    s.doc.translate_ids(&ids, dx, dy);
                    s.sync_ws();
                    s.set_status("Moved selection (Undo puts it back)");
                    return true;
                }
                return false;
            }
            let poly = std::mem::take(&mut s.lasso);
            s.selection = if poly.len() < 6 {
                // a tap selects the stroke under the pen (used for "play from here")
                poly.first().map(|&(x, y)| s.doc.hit_test(x, y, 6.0 / s.view.scale)).unwrap_or_default()
            } else {
                ink::lasso_select(&s.doc.strokes, &poly)
            };
            if !s.selection.is_empty() {
                s.set_status(format!("{} stroke(s) selected: drag to move, Delete to remove", s.selection.len()));
            }
            false
        }
        Tool::TextBox => false,
        tool => {
            let raw = std::mem::take(&mut s.live);
            if raw.is_empty() {
                return false;
            }
            if tool == Tool::Pen && is_scribble(&raw) {
                let path: Vec<(f32, f32)> = raw.iter().map(|p| (p.x, p.y)).collect();
                let n = s.doc.erase_path(&path, s.settings.width + 3.0, EraserMode::Stroke);
                s.set_status(format!("Scribble deleted {n} stroke(s)"));
                s.sync_ws();
                return true;
            }
            let mut pts = smooth_stroke(&raw, SmoothingConfig { strength: s.settings.smoothing });
            // Shape snapping only in Drawing mode, so handwritten O, 0, ∘ are never altered.
            if s.mode == InkMode::Drawing && s.settings.snap_shapes {
                if let Some((shape, clean)) = snap_shape(&pts) {
                    pts = clean;
                    s.set_status(format!("Snapped to a {shape:?}").to_lowercase());
                }
            }
            let mut stroke = s.live_stroke();
            stroke.points = pts;
            stroke.id = 0;
            let id = s.doc.add_stroke(stroke);
            if let Some(rec) = &s.recorder {
                s.rec_marks.push((id, rec.elapsed_ms().saturating_sub(raw.last().map_or(0, |p| p.t_ms))));
            }
            if !s.status.starts_with("Snapped") {
                s.status.clear();
                s.dirty |= dirty::TOOLS;
            }
            s.sync_ws();
            let _ = ui;
            true
        }
    }
}

// ---- keyboard ---------------------------------------------------------------------------------

pub fn shortcut(ui: &MainWindow, s: &mut State, key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> bool {
    let Some(combo) = commands::combo(key, ctrl, alt, shift, meta) else { return false };
    if let Some(id) = s.rebinding.clone() {
        if matches!(combo.as_str(), "ctrl" | "alt" | "shift" | "meta") {
            return true;
        }
        if combo == "escape" {
            s.rebinding = None;
        } else if combo == "backspace" {
            s.settings.shortcuts.insert(id.clone(), String::new());
            s.rebinding = None;
            s.set_status(format!("Shortcut for {id} removed"));
        } else {
            // a combo can only belong to one command: unbind it elsewhere
            for c in commands::ALL {
                if c.id != id && s.settings.binding(c.id).is_some_and(|b| b.split('|').any(|k| k == combo)) {
                    let rest: Vec<String> = s.settings.binding(c.id).unwrap_or_default().split('|').filter(|k| *k != combo).map(String::from).collect();
                    s.settings.shortcuts.insert(c.id.to_string(), rest.join("|"));
                }
            }
            s.settings.shortcuts.insert(id.clone(), combo.clone());
            s.rebinding = None;
            s.set_status(format!("{} is now {}", commands::get(&id).map_or(id.as_str(), |c| c.title), commands::pretty(&combo)));
        }
        overlay_query(ui, s, "");
        s.dirty |= dirty::TOOLS;
        return true;
    }
    match s.settings.command_for(&combo) {
        Some(id) => {
            run(ui, s, id);
            true
        }
        None => false,
    }
}

// ---- commands ---------------------------------------------------------------------------------

pub fn run(ui: &MainWindow, s: &mut State, id: &str) {
    let now = s.now_ms();
    match id {
        "palette" | "shortcuts" => {
            s.rebinding = None;
            open_overlay(ui, s, id);
            overlay_query(ui, s, "");
        }
        "switcher" | "split" => {
            open_overlay(ui, s, if id == "split" { "split-pick" } else { "switcher" });
            overlay_query(ui, s, "");
        }
        "settings" => {
            backup_info(ui, s);
            open_overlay(ui, s, "settings");
        }
        "symbols" => {
            if s.symbol_rows.is_empty() {
                let labels = s.session.labels();
                s.symbol_rows = symbols::all(&labels);
            }
            open_overlay(ui, s, "symbols");
            overlay_query(ui, s, "");
        }
        "focus" => {
            ui.set_focus_mode(!ui.get_focus_mode());
            ui.invoke_focus_canvas();
        }
        "present" => {
            let on = !ui.get_present_mode();
            ui.set_present_mode(on);
            ui.window().set_fullscreen(on);
            if on {
                ui.set_show_doc(true);
            }
            s.dirty |= dirty::DOC;
        }
        "fullscreen" => ui.window().set_fullscreen(!ui.window().is_fullscreen()),
        "toggle-sidebar" => ui.set_show_sidebar(!ui.get_show_sidebar()),
        "toggle-doc" => {
            if ui.get_win_w() < 700.0 {
                ui.set_doc_on_narrow(!ui.get_doc_on_narrow());
            } else {
                ui.set_show_doc(!ui.get_show_doc());
            }
            s.dirty |= dirty::DOC;
        }
        "split-close" => {
            s.split = None;
            s.dirty |= dirty::DOC;
        }
        "zoom-in" | "zoom-out" | "zoom-reset" => {
            let (cx, cy) = (s.size.0 as f32 / 2.0, s.size.1 as f32 / 2.0);
            match id {
                "zoom-in" => s.view.zoom_at(1.25, cx, cy),
                "zoom-out" => s.view.zoom_at(0.8, cx, cy),
                _ => s.view = View::default(),
            }
            s.dirty |= dirty::CANVAS;
        }
        "theme-dark" | "theme-light" | "theme-contrast" => {
            s.settings.theme = id.trim_start_matches("theme-").into();
            theme_changed(ui, s);
        }
        "pen" | "highlighter" | "eraser" | "lasso" | "textbox" => set_tool(s, id),
        "mode-text" => set_mode(s, "text"),
        "pen1" | "pen2" | "pen3" => {
            let i = id[3..].parse::<usize>().unwrap_or(1) - 1;
            s.tool = Tool::Pen;
            s.width_from_pen(i);
            s.active_pen = Some(i);
            s.dirty |= dirty::TOOLS;
        }
        "mode-math" => set_mode(s, "math"),
        "mode-drawing" => set_mode(s, "drawing"),
        "undo" if undo_text(s) => s.set_status("Undid the typed text change (Redo puts it back)"),
        "redo" if redo_text(s) => s.set_status("Redid the typed text change"),
        "undo" | "redo" => {
            if s.typing && id == "undo" && !s.editor.expr().is_empty() {
                s.editor.backspace();
                s.dirty |= dirty::STRIP;
                return;
            }
            if id == "undo" {
                s.doc.undo();
            } else {
                s.doc.redo();
            }
            s.selection.retain(|i| s.doc.strokes.iter().any(|x| x.id == *i));
            s.sync_ws();
            ui.invoke_pen_lifted();
        }
        "clear" => {
            let ids: Vec<u64> = s.doc.strokes.iter().map(|x| x.id).collect();
            s.doc.remove_ids(&ids);
            clear_expression(s);
            s.sync_ws();
        }
        "delete-selection" => {
            let ids = std::mem::take(&mut s.selection);
            if !ids.is_empty() {
                let n = s.doc.remove_ids(&ids);
                s.set_status(format!("Deleted {n} stroke(s)"));
                s.sync_ws();
                ui.invoke_pen_lifted();
            }
        }
        "insert-axes" | "insert-ruler" => insert_guides(s, id == "insert-axes"),
        "snap-toggle" => {
            s.settings.snap_shapes = !s.settings.snap_shapes;
            s.set_status(if s.settings.snap_shapes { "Shape snapping on (Drawing mode)" } else { "Shape snapping off" });
            theme_changed(ui, s);
        }
        "recognize" => run_recognition(ui, s),
        "insert" => insert_expression(ui, s),
        "typing" => {
            s.typing = !s.typing;
            s.dirty |= dirty::TOOLS | dirty::STRIP;
            ui.invoke_focus_canvas();
        }
        "train" => {
            s.training = !s.training;
            if s.training {
                update_training_ui(ui, s);
            }
            s.dirty |= dirty::TOOLS;
        }
        "copy-latex" | "copy-typst" | "copy-unicode" | "copy-image" => {
            let e = current_expr(s);
            if e.is_empty() {
                s.set_status("Nothing to copy: draw or type an expression first");
                return;
            }
            copy_expr(s, &e, id);
        }
        t if t.starts_with("tmpl-") => {
            let out = match t {
                "tmpl-frac" => input::Out::Frac,
                "tmpl-sqrt" => input::Out::Sqrt,
                "tmpl-int" => input::Out::Big(layout::BigOp::Int),
                "tmpl-sum" => input::Out::Big(layout::BigOp::Sum),
                "tmpl-lim" => input::Out::Big(layout::BigOp::Lim),
                "tmpl-matrix" => input::Out::Matrix { kind: layout::MatrixKind::Bracket, rows: 2, cols: 2 },
                "tmpl-cases" => input::Out::Cases,
                _ => input::Out::Deriv { partial: false },
            };
            start_typed(s);
            s.editor.insert_template(out);
            s.dirty |= dirty::STRIP;
        }
        "new-page" => new_page(s, "blank"),
        "new-section" => ask(ui, s, Prompt::NewSection, "New section", "Name of the section", false),
        "new-notebook" => ask(ui, s, Prompt::NewNotebook, "New notebook", "Name of the notebook", false),
        "new-encrypted" => ask(ui, s, Prompt::NewEncryptedName, "New encrypted notebook", "Name of the notebook (it is hidden in the list until you unlock it)", false),
        "lock" => {
            if s.ws.as_ref().is_some_and(|w| w.is_encrypted()) {
                let plain = s.ws.as_ref().and_then(|w| w.notebooks().into_iter().find(|m| !m.encrypted)).map(|m| m.id);
                if let (Some(id), Some(w)) = (plain, &mut s.ws) {
                    match w.switch_notebook(&id, None, now) {
                        Ok(()) => s.status = "Encrypted notebook locked".into(),
                        Err(e) => s.status = e,
                    }
                    s.tabs.clear();
                    s.load_page_into_doc();
                }
            } else {
                s.set_status("This notebook is not encrypted");
            }
        }
        "duplicate" => {
            s.sync_ws();
            if let Some(w) = &mut s.ws {
                if let Err(e) = w.duplicate_current(now) {
                    s.status = format!("Could not duplicate: {e}");
                }
            }
            s.load_page_into_doc();
        }
        "delete-page" => {
            let res = s.ws.as_mut().map(|w| w.delete_current(now));
            match res {
                Some(Ok(())) => {
                    s.load_page_into_doc();
                    s.set_status("Page moved to the trash (kept 30 days). Trash in the sidebar restores it.");
                }
                Some(Err(e)) => s.set_status(format!("Could not delete: {e}")),
                None => {}
            }
        }
        "trash" | "history" | "backups" => {
            open_overlay(ui, s, id);
            if id == "backups" {
                backup_info(ui, s);
            }
            overlay_query(ui, s, "");
        }
        "history-restore" => {
            if let (Some(t), Some(w)) = (s.history_pick, &mut s.ws) {
                match w.restore_snapshot(t, now) {
                    Ok(()) => s.status = "Restored that version (the previous one is in the history too)".into(),
                    Err(e) => s.status = format!("Restore failed: {e}"),
                }
                s.load_page_into_doc();
                close_overlay(ui, s);
            }
        }
        "favorite" => {
            if let Some(w) = &mut s.ws {
                w.toggle_favorite();
            }
            s.dirty |= dirty::SIDEBAR;
        }
        "save-template" => ask(ui, s, Prompt::SaveTemplate, "Save page as template", "Template name (ink, audio and PDF are not included)", false),
        "back" | "forward" => {
            let (from, to) = if id == "back" { (&mut s.back, &mut s.fwd) } else { (&mut s.fwd, &mut s.back) };
            if let Some(target) = from.pop() {
                if let Some(cur) = s.ws.as_ref().map(|w| w.page.id.clone()) {
                    to.push(cur);
                }
                if let Err(e) = s.goto_page(&target, false) {
                    s.set_status(e);
                }
            }
        }
        "close-tab" => {
            if let Some(id) = s.ws.as_ref().map(|w| w.page.id.clone()) {
                close_tab(s, &id);
            }
        }
        "next-tab" => {
            let cur = s.ws.as_ref().map(|w| w.page.id.clone()).unwrap_or_default();
            if let Some(i) = s.tabs.iter().position(|t| *t == cur) {
                let next = s.tabs[(i + 1) % s.tabs.len()].clone();
                let _ = s.goto_page(&next, true);
            }
        }
        "search" => {
            ui.set_show_sidebar(true);
            s.set_status("Type in the search box at the top of the sidebar (tag:name, after:2026-01-01, before:...)");
        }
        "find" => {
            ui.set_show_doc(true);
            ui.set_show_find(true);
            s.dirty |= dirty::DOC;
        }
        "text-block" => {
            ui.set_show_doc(true);
            s.editing = None;
            ui.set_editor_text("".into());
            s.set_status("Type in the text box under the page, then press Add text block");
            s.dirty |= dirty::DOC;
        }
        "callout" => add_block(s, store::Block::Callout { kind: store::CalloutKind::Definition, markup: "Definition text".into() }),
        "rule" => add_block(s, store::Block::Rule),
        "image" => {
            if let Some(p) = pick_file(ui, s, "Insert image", &["png", "jpg", "jpeg", "gif", "webp"], Prompt::ImagePath) {
                open_file(ui, s, &p);
            }
        }
        "paste-image" => paste_image(s),
        "pdf" => {
            if let Some(p) = pick_file(ui, s, "Import PDF", &["pdf"], Prompt::PdfPath) {
                open_file(ui, s, &p);
            }
        }
        "plot" => ask(ui, s, Prompt::PlotFunctions, "Function plot", "Functions of x separated by ; for example: sin(x); x^2/4 - 1", false),
        "graph" => {
            let on = !ui.get_graph_mode();
            ui.set_graph_mode(on);
            if on {
                s.graph.dark = !s.light();
                refresh_graph(ui, s);
            }
        }
        "graph-add-expr" => {
            let e = current_expr(s);
            if e.is_empty() {
                s.set_status("Draw or type an expression first (y = ..., x^2 + y^2 = 4, ...)");
                return;
            }
            let line = expr_to_calc(&e);
            let mut t = ui.get_graph_text().to_string();
            if !t.is_empty() && !t.ends_with('\n') {
                t.push('\n');
            }
            t += &line;
            ui.set_graph_text(t.as_str().into());
            s.graph.set_text(&t);
            refresh_graph(ui, s);
        }
        "save-graph" => save_graph(ui, s),
        "record" => toggle_recording(s),
        "play" => play_audio(s),
        "paper" => {
            s.paper.kind = s.paper.kind.next();
            s.settings.paper_kind = s.paper.kind.name().to_lowercase();
            s.sync_ws();
            s.dirty |= dirty::TOOLS;
        }
        "page-mode" => {
            if let Some(w) = &mut s.ws {
                w.page.mode = match w.page.mode {
                    store::PageMode::Infinite => store::PageMode::Letter,
                    store::PageMode::Letter => store::PageMode::A4,
                    store::PageMode::A4 => store::PageMode::Infinite,
                };
                w.touch();
            }
            s.dirty |= dirty::CANVAS | dirty::DOC;
        }
        "export" => export(s, "PDF"),
        "export-notebook" => {
            s.sync_ws();
            let dir = workspace::default_root().join("exports");
            let res = s.ws.as_mut().map(|w| w.export_notebook(export::Format::Pdf, &dir, now));
            match res {
                Some(Ok(p)) => s.set_status(format!("Exported the notebook to {}", p.display())),
                Some(Err(e)) => s.set_status(format!("Export failed: {e}")),
                None => {}
            }
        }
        "print" => print(s),
        "backup" => {
            let (root, dest) = backup_dirs(s);
            match store::backup::backup_now(&root, &dest, now) {
                Ok(p) => s.set_status(format!("Backed up to {}", p.display())),
                Err(e) => s.set_status(format!("Backup failed: {e}")),
            }
            backup_info(ui, s);
            if s.overlay == "backups" {
                overlay_query(ui, s, "");
            }
        }
        "restore-backup" => run(ui, s, "backups"),
        "tablet" => ui.set_show_tablet(!ui.get_show_tablet()),
        other => s.set_status(format!("Unknown command {other}")),
    }
    s.dirty |= dirty::TOOLS;
}

fn start_typed(s: &mut State) {
    if !s.typed_since_recog && s.editor.expr().is_empty() {
        s.editor.set_expr(s.session.expression());
    }
    s.typed_since_recog = true;
}

pub fn clear_expression(s: &mut State) {
    s.session.symbols.clear();
    s.session.selected = None;
    s.editor = input::Editor::default();
    s.consumed.clear();
    s.typed_since_recog = false;
    s.dirty |= dirty::STRIP;
}

/// Insert the current (drawn and/or typed) expression into the page as a math block and remove the
/// Math/Text ink it came from, so the next expression starts fresh.
pub fn insert_expression(ui: &MainWindow, s: &mut State) {
    run_recognition(ui, s);
    if s.text_mode {
        insert_words(s);
        return;
    }
    let e = current_expr(s);
    if e.is_empty() {
        s.set_status("Nothing to insert yet: write math on the canvas or press Type");
        return;
    }
    let used: Vec<u64> = s.session.symbols.iter().flat_map(|x| x.stroke_ids.clone()).collect();
    add_block(s, store::Block::Math { expr: e, source: None });
    if !used.is_empty() {
        s.doc.remove_ids(&used);
        s.sync_ws();
    }
    clear_expression(s);
    s.set_status("Inserted into the page (Undo brings the ink back)");
    tablet::push_state(s);
}

pub fn add_block(s: &mut State, b: store::Block) {
    if let Some(w) = &mut s.ws {
        let at = s.editing.map(|i| i + 1).unwrap_or(w.page.blocks.len()).min(w.page.blocks.len());
        w.page.blocks.insert(at, b);
        w.touch();
    }
    s.dirty |= dirty::DOC;
}

fn copy_expr(s: &mut State, e: &layout::Expr, how: &str) {
    let r = match how {
        "copy-latex" => clipboard_text(&e.to_latex()),
        "copy-typst" => clipboard_text(&e.to_typst()),
        "copy-unicode" => clipboard_text(&e.to_unicode()),
        _ => export::copy_block(e, export::CopyAs::Image).and_then(|png| clipboard_png(&png)),
    };
    s.set_status(match r {
        Ok(()) => "Copied".to_string(),
        Err(e) => e,
    });
}

/// Expression tree -> the graph view's calculator syntax.
pub fn expr_to_calc(e: &layout::Expr) -> String {
    use layout::Expr::*;
    let g = |x: &layout::Expr| format!("({})", expr_to_calc(x));
    match e {
        Row(v) => v.iter().map(expr_to_calc).collect::<Vec<_>>().join(" "),
        Sym(s) => match s.as_str() {
            "π" => "pi".into(),
            "θ" => "t".into(),
            "·" | "×" => "*".into(),
            "−" => "-".into(),
            "÷" => "/".into(),
            "≤" => "<=".into(),
            "≥" => ">=".into(),
            other => other.into(),
        },
        Num(n) => n.clone(),
        Func(f) => f.clone(),
        Unit(u) | Text(u) | Raw(u) => u.clone(),
        Frac(a, b) => format!("{}/{}", g(a), g(b)),
        Sqrt { index: None, body } => format!("sqrt{}", g(body)),
        Sqrt { index: Some(i), body } => format!("{}^(1/{})", g(body), g(i)),
        Sup(a, b) => format!("{}^{}", g(a), g(b)),
        Sub(a, _) => expr_to_calc(a),
        SubSup(a, _, c) => format!("{}^{}", g(a), g(c)),
        Fence(layout::Fence::Abs, b) => format!("abs{}", g(b)),
        Fence(_, b) => g(b),
        Accent(_, b) => expr_to_calc(b),
        _ => e.to_unicode(),
    }
}

fn insert_guides(s: &mut State, axes: bool) {
    let (cx, cy) = s.view.to_page(s.size.0 as f32 / 2.0, s.size.1 as f32 / 2.0);
    let len = 240.0 / s.view.scale;
    let c = s.pen_color();
    let mk = |a: (f32, f32), b: (f32, f32)| Stroke { id: 0, points: vec![Point::new(a.0, a.1, 0), Point::new(b.0, b.1, 10)], color: c, width: 1.5, kind: PenKind::Pen, mode: InkMode::Drawing, t0_ms: 0 };
    let mut strokes = vec![mk((cx - len, cy), (cx + len, cy))];
    if axes {
        strokes.push(mk((cx, cy + len), (cx, cy - len)));
        strokes.push(mk((cx + len, cy), (cx + len - 10.0, cy - 6.0)));
        strokes.push(mk((cx + len, cy), (cx + len - 10.0, cy + 6.0)));
        strokes.push(mk((cx, cy - len), (cx - 6.0, cy - len + 10.0)));
        strokes.push(mk((cx, cy - len), (cx + 6.0, cy - len + 10.0)));
    }
    let step = 40.0 / s.view.scale;
    let mut k = -len + step;
    while k < len - 1.0 {
        strokes.push(mk((cx + k, cy - 4.0), (cx + k, cy + 4.0)));
        if axes {
            strokes.push(mk((cx - 4.0, cy + k), (cx + 4.0, cy + k)));
        }
        k += step;
    }
    for st in strokes {
        s.doc.add_stroke(st);
    }
    s.sync_ws();
    s.set_status(if axes { "Axes inserted (Drawing ink, not recognized)" } else { "Ruler inserted" });
}

pub fn type_paste(s: &mut State, t: &str) {
    let t = t.trim();
    if t.is_empty() {
        return;
    }
    if layout::looks_like_latex(t) || t.starts_with('$') {
        let src = if layout::looks_like_latex(t) { layout::latex_to_typst(t) } else { t.trim_matches('$').trim().to_string() };
        match render::compile(&render::math_source(&src, 14.0, [0, 0, 0]), &[]) {
            Ok(_) => {
                add_block(s, store::Block::Math { expr: layout::Expr::empty(), source: Some(src.clone()) });
                s.set_status(format!("Inserted as math: {src}"));
            }
            Err(e) => s.set_status(format!("Could not read that formula: {e}")),
        }
        return;
    }
    start_typed(s);
    s.editor.insert_str(t);
    s.editor.insert_char(' ');
    s.dirty |= dirty::STRIP;
}

// ---- pages, tabs, notebooks ---------------------------------------------------------------------

pub fn new_page(s: &mut State, template: &str) {
    s.sync_ws();
    let now = s.now_ms();
    let cur = s.ws.as_ref().map(|w| w.page.id.clone());
    let res = s.ws.as_mut().map(|w| w.new_page(template, now));
    match res {
        Some(Err(e)) => s.set_status(format!("Could not create page: {e}")),
        Some(Ok(())) => {
            if let Some(c) = cur {
                s.back.push(c);
                s.fwd.clear();
            }
            s.load_page_into_doc();
        }
        None => {}
    }
}

pub fn close_tab(s: &mut State, id: &str) {
    let Some(i) = s.tabs.iter().position(|t| t == id) else { return };
    s.tabs.remove(i);
    let cur = s.ws.as_ref().map(|w| w.page.id.clone()).unwrap_or_default();
    if cur == id {
        if let Some(next) = s.tabs.get(i.min(s.tabs.len().saturating_sub(1))).cloned() {
            let _ = s.goto_page(&next, true);
        } else {
            s.tabs.push(cur);
        }
    }
    s.dirty |= dirty::TABS;
}

pub fn set_notebook(ui: &MainWindow, s: &mut State, i: i32) {
    let Some(meta) = s.ws.as_ref().and_then(|w| w.notebooks().get(i.max(0) as usize).cloned()) else { return };
    if s.ws.as_ref().is_some_and(|w| w.nb.id == meta.id) {
        return;
    }
    if meta.encrypted {
        ask(ui, s, Prompt::Unlock(meta.id.clone()), "Unlock notebook", "Password for this encrypted notebook", true);
        s.dirty |= dirty::SIDEBAR;
        return;
    }
    s.sync_ws();
    let now = s.now_ms();
    if let Some(w) = &mut s.ws {
        if let Err(e) = w.switch_notebook(&meta.id, None, now) {
            s.status = e;
        }
    }
    s.tabs.clear();
    s.back.clear();
    s.fwd.clear();
    s.load_page_into_doc();
}

// ---- blocks and the text editor -------------------------------------------------------------------

pub fn block_action(ui: &MainWindow, s: &mut State, i: usize, a: &str) {
    let Some(w) = &mut s.ws else { return };
    let n = w.page.blocks.len();
    if i >= n {
        return;
    }
    match a {
        "up" if i > 0 => w.page.blocks.swap(i, i - 1),
        "down" if i + 1 < n => w.page.blocks.swap(i, i + 1),
        "delete" => {
            w.page.blocks.remove(i);
            s.editing = None;
        }
        "edit" => {
            s.editing = Some(i);
            ui.set_editor_text(pagedoc::edit_text(&w.page.blocks[i]).into());
            s.dirty |= dirty::DOC;
            spell(ui, s);
            return;
        }
        "copy-latex" | "copy-typst" => {
            if let store::Block::Math { expr, source } = &w.page.blocks[i] {
                let text = if expr.is_empty() { source.clone().unwrap_or_default() } else if a == "copy-latex" { expr.to_latex() } else { expr.to_typst() };
                let r = clipboard_text(&text);
                s.set_status(r.map(|_| "Copied".to_string()).unwrap_or_else(|e| e));
            }
            return;
        }
        "wider" | "narrower" | "crop" | "uncrop" => {
            if let store::Block::Image { w: iw, h: ih, crop, .. } = &mut w.page.blocks[i] {
                match a {
                    "wider" => {
                        *ih *= 1.2;
                        *iw *= 1.2;
                    }
                    "narrower" => {
                        *ih /= 1.2;
                        *iw /= 1.2;
                    }
                    "crop" => {
                        let c = crop.get_or_insert([0.0; 4]);
                        for v in c.iter_mut() {
                            *v = (*v + 0.05).min(0.4);
                        }
                    }
                    _ => *crop = None,
                }
            }
        }
        _ => return,
    }
    w.touch();
    s.dirty |= dirty::DOC;
}

pub fn editor_commit(ui: &MainWindow, s: &mut State) {
    let text = ui.get_editor_text().to_string();
    let Some(w) = &mut s.ws else { return };
    match s.editing.filter(|&i| i < w.page.blocks.len()) {
        Some(i) => {
            let b = &mut w.page.blocks[i];
            match b {
                store::Block::Text { markup } | store::Block::Callout { markup, .. } => *markup = text,
                store::Block::Math { expr, source } => {
                    let src = if layout::looks_like_latex(&text) { layout::latex_to_typst(&text) } else { text.trim().to_string() };
                    if src != expr.to_typst() {
                        *expr = layout::Expr::empty();
                        *source = Some(src);
                    }
                }
                _ => {}
            }
            s.editing = None;
        }
        None => {
            if text.trim().is_empty() {
                s.set_status("The text box is empty");
                return;
            }
            w.page.blocks.push(store::Block::Text { markup: text });
        }
    }
    w.touch();
    ui.set_editor_text("".into());
    ui.set_spelling(model(vec![]));
    s.dirty |= dirty::DOC;
}

/// Spell-check the editor text (lazy-loads the dictionary on first use).
pub fn spell(ui: &MainWindow, s: &mut State) {
    if !s.settings.spell_check {
        ui.set_spelling(model(vec![]));
        return;
    }
    if s.speller.is_none() {
        s.speller = pagedoc::Speller::new();
    }
    let rows = s.speller.as_ref().map(|sp| sp.check(&ui.get_editor_text(), 6)).unwrap_or_default();
    ui.set_spelling(model(rows.into_iter().map(|(w, sug)| ItemRow { id: w.as_str().into(), title: w.into(), detail: sug.into() }).collect()));
}

// ---- overlays ---------------------------------------------------------------------------------

fn item(id: impl Into<SharedString>, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> ItemRow {
    ItemRow { id: id.into(), title: title.into(), detail: detail.into() }
}

fn when(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{} {:02}:{:02} UTC", store::iso_date(ms), secs / 3600 % 24, secs / 60 % 60)
}

pub fn overlay_query(ui: &MainWindow, s: &mut State, q: &str) {
    match s.overlay.as_str() {
        "palette" | "shortcuts" => {
            let rows: Vec<CmdRow> = commands::ALL
                .iter()
                .filter(|c| commands::matches(c, q))
                .map(|c| CmdRow { id: c.id.into(), title: c.title.into(), key: s.settings.binding(c.id).map(|k| commands::pretty(&k)).unwrap_or_default().into(), group: c.group.into() })
                .collect();
            ui.set_commands(model(rows));
        }
        "switcher" | "split-pick" => {
            let ql = q.to_lowercase();
            let rows: Vec<ItemRow> = s.ws.as_ref().map(|w| w.rows()).unwrap_or_default().into_iter().filter(|r| !r.is_section && r.title.to_lowercase().contains(&ql)).map(|r| item(r.id, r.title, r.section)).collect();
            ui.set_items_hint("".into());
            ui.set_items(model(rows));
        }
        "symbols" => {
            let hits = symbols::filter(&s.symbol_rows, q);
            ui.set_symbols(model(hits.iter().map(|x| SymRow { sym: x.display.as_str().into(), name: x.name.as_str().into() }).collect()));
        }
        "trash" => {
            let rows: Vec<ItemRow> = s.ws.as_ref().map(|w| w.trash()).unwrap_or_default().into_iter().map(|t| item(t.page_id, t.title, format!("deleted {}  ·  click to restore", when(t.deleted_ms)))).collect();
            ui.set_items_hint("Click a page to put it back where it was.".into());
            ui.set_items(model(rows));
        }
        "history" => {
            let rows: Vec<ItemRow> = s.ws.as_ref().map(|w| w.history()).unwrap_or_default().into_iter().map(|t| item(t.to_string(), when(t), "click to preview")).collect();
            ui.set_items_hint("Snapshots are taken at most every 5 minutes while you work; older ones are thinned out. Pick one to preview it, then Restore.".into());
            ui.set_items(model(rows));
        }
        "backups" => {
            let (_, dest) = backup_dirs(s);
            let rows: Vec<ItemRow> = store::backup::list_backups(&dest).into_iter().rev().map(|p| item(p.display().to_string(), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), "click to restore (the current state is backed up first)")).collect();
            ui.set_items_hint(format!("Backups folder: {}", dest.display()).into());
            ui.set_items(model(rows));
        }
        _ => {}
    }
}

pub fn overlay_pick(ui: &MainWindow, s: &mut State, id: &str) {
    let now = s.now_ms();
    match s.overlay.clone().as_str() {
        "palette" => {
            close_overlay(ui, s);
            run(ui, s, id);
        }
        "switcher" => {
            close_overlay(ui, s);
            if let Err(e) = s.goto_page(id, true) {
                s.set_status(e);
            }
        }
        "split-pick" => {
            close_overlay(ui, s);
            s.split = Some(id.to_string());
            ui.set_show_doc(true);
            s.dirty |= dirty::DOC;
        }
        "trash" => {
            s.sync_ws();
            let r = s.ws.as_mut().map(|w| w.restore_from_trash(id, now));
            match r {
                Some(Ok(())) => {
                    s.load_page_into_doc();
                    s.set_status("Restored from the trash");
                    close_overlay(ui, s);
                }
                Some(Err(e)) => s.set_status(format!("Could not restore: {e}")),
                None => {}
            }
        }
        "history" => {
            let Ok(t) = id.parse::<u64>() else { return };
            let page = s.ws.as_ref().and_then(|w| w.snapshot(t));
            if let Some(p) = page {
                s.history_pick = Some(t);
                let media = s.ws.as_ref().map(|w| w.page_media(&p)).unwrap_or_default();
                match export::export_pages_with(&[&p], export::Format::Png, &media, &[]).ok().and_then(|png| image::load_from_memory(&png).ok()) {
                    Some(img) => {
                        let rgba = img.to_rgba8();
                        let (w, h) = rgba.dimensions();
                        ui.set_preview_image(slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), w, h)));
                        ui.set_has_preview(true);
                    }
                    None => ui.set_has_preview(false),
                }
            }
        }
        "backups" => {
            let path = PathBuf::from(id);
            ask(ui, s, Prompt::RestoreBackup(id.to_string()), "Restore backup?", &format!("Type RESTORE to replace your notes with {}. Your current notes are backed up first.", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), false);
        }
        _ => {}
    }
}

pub fn prompt_ok(ui: &MainWindow, s: &mut State, text: &str) {
    let Some(p) = s.prompt.take() else {
        close_overlay(ui, s);
        return;
    };
    let now = s.now_ms();
    let t = text.trim();
    close_overlay(ui, s);
    match p {
        Prompt::NewSection if !t.is_empty() => {
            if let Some(w) = &mut s.ws {
                let _ = w.new_section(t);
            }
            s.dirty |= dirty::SIDEBAR;
        }
        Prompt::NewNotebook if !t.is_empty() => {
            s.sync_ws();
            if let Some(w) = &mut s.ws {
                if let Err(e) = w.create_notebook(t, None, now) {
                    s.status = e;
                }
            }
            s.tabs.clear();
            s.load_page_into_doc();
        }
        Prompt::NewEncryptedName if !t.is_empty() => ask(ui, s, Prompt::NewEncryptedPassword(t.to_string()), "Choose a password", "At least 8 characters. There is no recovery: if you forget it, the notebook cannot be opened.", true),
        Prompt::NewEncryptedPassword(name) => {
            if text.chars().count() < 8 {
                ask(ui, s, Prompt::NewEncryptedPassword(name), "Choose a password", "Too short: use at least 8 characters.", true);
                return;
            }
            s.sync_ws();
            if let Some(w) = &mut s.ws {
                match w.create_notebook(&name, Some(text), now) {
                    Ok(()) => s.status = "Encrypted notebook created. It locks when you switch away or close Scrawl.".into(),
                    Err(e) => s.status = e,
                }
            }
            s.tabs.clear();
            s.load_page_into_doc();
        }
        Prompt::Unlock(id) => {
            s.sync_ws();
            let r = s.ws.as_mut().map(|w| w.switch_notebook(&id, Some(text), now));
            match r {
                Some(Ok(())) => {
                    s.tabs.clear();
                    s.back.clear();
                    s.fwd.clear();
                    s.load_page_into_doc();
                    s.set_status("Unlocked");
                }
                Some(Err(e)) => {
                    s.set_status(e.clone());
                    ask(ui, s, Prompt::Unlock(id), "Unlock notebook", &e, true);
                }
                None => {}
            }
        }
        Prompt::SaveTemplate if !t.is_empty() => {
            let safe: String = t.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect();
            let r = s.ws.as_ref().map(|w| w.save_as_template(&safe));
            s.set_status(match r {
                Some(Ok(())) => format!("Saved template \"{safe}\" (choose \"my: {safe}\" for new pages)"),
                Some(Err(e)) => format!("Could not save template: {e}"),
                None => String::new(),
            });
            s.dirty |= dirty::SIDEBAR;
        }
        Prompt::ImagePath | Prompt::PdfPath | Prompt::MatPath if !t.is_empty() => {
            let path = PathBuf::from(t.trim_matches('"'));
            open_file(ui, s, &path);
        }
        Prompt::PlotFunctions if !t.is_empty() => {
            let functions: Vec<String> = t.split(';').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
            let b = store::Block::Plot { functions, x_range: (-6.0, 6.0), y_range: None };
            if export::plot_svg(&b).is_none() {
                s.set_status("Could not plot that: use functions of x like sin(x); x^2 - 1");
                return;
            }
            add_block(s, b);
        }
        Prompt::RestoreBackup(path) => {
            if t != "RESTORE" {
                s.set_status("Restore cancelled");
                return;
            }
            s.sync_ws();
            if let Some(w) = &mut s.ws {
                let _ = w.save(now);
            }
            let (root, dest) = backup_dirs(s);
            match store::backup::restore_backup(Path::new(&path), &root, &dest, now) {
                Ok(safety) => {
                    let pref = s.ws.as_ref().map(|w| (w.nb.id.clone(), w.page.id.clone()));
                    s.ws = Workspace::open_with(&root, pref.as_ref().map(|p| p.0.as_str()), pref.as_ref().map(|p| p.1.as_str())).ok();
                    s.tabs.clear();
                    s.load_page_into_doc();
                    s.set_status(format!("Restored. Your previous notes were saved to {}", safety.display()));
                }
                Err(e) => s.set_status(format!("Restore failed: {e}")),
            }
        }
        Prompt::ExternalChange => {
            if t.eq_ignore_ascii_case("reload") {
                if let Some(w) = &mut s.ws {
                    let _ = w.reload_from_disk();
                }
                s.load_page_into_doc();
                s.set_status("Reloaded the version from disk");
            } else if let Some(w) = &mut s.ws {
                w.ignore_disk_change();
                s.set_status("Kept your version (it will overwrite the file on the next save)");
            }
        }
        _ => {}
    }
}

pub fn insert_symbol(ui: &MainWindow, s: &mut State, sym: &str) {
    let Some(entry) = s.symbol_rows.iter().find(|x| x.display == sym).cloned() else { return };
    start_typed(s);
    match entry.out {
        symbols::Insert::Expr(e) => s.editor.insert_expr(e),
        symbols::Insert::Template(o) => s.editor.insert_template(o),
    }
    s.set_status(format!("Inserted {sym}. Tab moves between slots, Insert ↵ puts it on the page."));
    s.dirty |= dirty::STRIP;
    let _ = ui;
}

pub fn theme_changed(ui: &MainWindow, s: &mut State) {
    s.settings.sanitize();
    apply_theme(ui, s);
    s.graph.dark = !s.light();
    if ui.get_graph_mode() {
        refresh_graph(ui, s);
    }
    s.dirty = dirty::ALL;
}

// ---- files ---------------------------------------------------------------------------------

/// Open a file by type: image -> image block, PDF -> pages to annotate, .mat -> graph data.
pub fn open_file(ui: &MainWindow, s: &mut State, path: &Path) {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            s.set_status(format!("Could not read {}: {e}", path.display()));
            return;
        }
    };
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" => add_image(s, &ext, &bytes),
        "pdf" => import_pdf(s, path, bytes),
        "mat" => {
            let out = s.graph.load_mat(&bytes);
            ui.set_graph_mode(true);
            ui.set_graph_out(out.into());
            refresh_graph(ui, s);
        }
        _ => s.set_status(format!("Scrawl cannot open .{ext} files (images, PDF and .mat work)")),
    }
}

fn add_image(s: &mut State, ext: &str, bytes: &[u8]) {
    let Ok(img) = media::images::Img::decode(bytes) else {
        s.set_status("That image could not be decoded");
        return;
    };
    // keep at most 2000 px wide in the notebook
    let img = if img.size().0 > 2000 { img.fit(2000, 4000) } else { img };
    let (w, h) = img.size();
    let (png, ext) = if ext == "png" && w <= 2000 { (bytes.to_vec(), "png") } else { (img.to_png(), "png") };
    let res = s.ws.as_ref().map(|ws| ws.add_media(ext, &png));
    match res {
        Some(Ok(name)) => {
            let disp_w = (w as f32).min(360.0);
            add_block(s, store::Block::Image { file: name, x: 0.0, y: 0.0, w: disp_w, h: disp_w * h as f32 / w.max(1) as f32, crop: None });
            s.set_status("Image added to the page: hover it for size and crop buttons");
        }
        Some(Err(e)) => s.set_status(format!("Could not store the image: {e}")),
        None => {}
    }
}

fn paste_image(s: &mut State) {
    let got = arboard::Clipboard::new().and_then(|mut c| c.get_image());
    match got {
        Ok(img) => {
            let Some(buf) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else {
                s.set_status("The clipboard image has an unexpected format");
                return;
            };
            let mut png = Vec::new();
            if image::DynamicImage::ImageRgba8(buf).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                add_image(s, "png", &png);
            }
        }
        Err(e) => s.set_status(format!("No image on the clipboard ({e})")),
    }
}

fn import_pdf(s: &mut State, path: &Path, bytes: Vec<u8>) {
    let doc = match media::pdf::PdfDoc::open(bytes) {
        Ok(d) => d,
        Err(e) => {
            s.set_status(e);
            return;
        }
    };
    let n = doc.page_count().min(200);
    let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "PDF".into());
    let text = doc.text();
    let now = s.now_ms();
    s.sync_ws();
    let mut first: Option<String> = None;
    for i in 0..n {
        let Some(img) = doc.render_page(i, PDF_RENDER_SCALE) else { continue };
        let Some(pm) = tiny_skia::Pixmap::from_vec(img.rgba, tiny_skia::IntSize::from_wh(img.width, img.height).unwrap_or(tiny_skia::IntSize::from_wh(1, 1).unwrap())) else { continue };
        let Ok(png) = pm.encode_png() else { continue };
        let Some(w) = &mut s.ws else { return };
        let Ok(media) = w.add_media("png", &png) else { continue };
        let mut p = store::Page::new(&store::new_id(), &format!("{name} – page {}", i + 1), &store::iso_date(now));
        p.pdf_background = Some(media);
        p.paper.kind = "blank".into();
        p.tags = vec!["pdf".into()];
        if i == 0 && !text.trim().is_empty() {
            // the PDF's text goes into the first page so search finds it
            let snippet: String = text.chars().take(20_000).collect();
            p.blocks.push(store::Block::Text { markup: format!("// PDF text (for search)\n{}", snippet.lines().map(|l| format!("// {l}")).collect::<Vec<_>>().join("\n")) });
        }
        if w.add_page(p, now).is_ok() && first.is_none() {
            first = Some(w.page.id.clone());
        }
    }
    if let Some(f) = first {
        let _ = s.goto_page(&f, true);
        s.load_page_into_doc();
        s.set_status(format!("Imported {n} page(s) from {name}: write on them with the pen"));
    } else {
        s.set_status("No pages could be rendered from that PDF");
    }
}

pub fn load_mat(ui: &MainWindow, s: &mut State) {
    if let Some(p) = pick_file(ui, s, "Load MATLAB data", &["mat"], Prompt::MatPath) {
        open_file(ui, s, &p);
    }
}

fn save_graph(ui: &MainWindow, s: &mut State) {
    // render light, at a fixed size, then put the tick labels on with Typst
    let (dark, size) = (s.graph.dark, s.graph.size);
    s.graph.dark = false;
    s.graph.size = (900, 560);
    let f = s.graph.frame();
    s.graph.dark = dark;
    s.graph.size = size;
    let Some(pm) = tiny_skia::Pixmap::from_vec(f.rgba.clone(), tiny_skia::IntSize::from_wh(f.width, f.height).unwrap_or(tiny_skia::IntSize::from_wh(1, 1).unwrap())) else { return };
    let Ok(plain) = pm.encode_png() else { return };
    let mut src = format!("#set page(width: {}pt, height: {}pt, margin: 0pt)\n#set text(size: 8pt, fill: rgb(\"#3b4261\"))\n#place(image(\"g.png\", width: 100%))\n", f.width, f.height);
    for (x, y, t, _) in &f.labels {
        src += &format!("#place(dx: {x:.1}pt, dy: {y:.1}pt)[{}]\n", store::typst_escape(t));
    }
    let png = render::compile(&src, &[("g.png", plain.clone())]).ok().and_then(|d| render::png(&d, 0, 1.0)).unwrap_or(plain);
    let res = s.ws.as_ref().map(|w| w.add_media("png", &png));
    if let Some(Ok(name)) = res {
        add_block(s, store::Block::Image { file: name, x: 0.0, y: 0.0, w: 360.0, h: 360.0 * f.height as f32 / f.width as f32, crop: None });
        let eqs: Vec<String> = ui.get_graph_text().lines().filter(|l| !l.trim().is_empty()).map(|l| format!("`{}`", l.trim().replace('`', ""))).collect();
        add_block(s, store::Block::Text { markup: format!("_Graph:_ {}", eqs.join(", ")) });
        s.set_status("Graph saved into the page (image plus its equations)");
    }
}

// ---- audio ---------------------------------------------------------------------------------

fn toggle_recording(s: &mut State) {
    if let Some(rec) = s.recorder.take() {
        let dur = rec.elapsed_ms();
        match rec.finish() {
            Ok(ogg) => {
                let res = s.ws.as_ref().map(|w| w.add_media("ogg", &ogg));
                if let (Some(Ok(name)), Some(w)) = (res, &mut s.ws) {
                    w.page.audio = Some(store::AudioClip { file: name, marks: std::mem::take(&mut s.rec_marks), duration_ms: dur });
                    w.touch();
                    s.status = format!("Saved {:.0} s of audio. Lasso or tap ink, then Play, to hear what was said then.", dur as f32 / 1000.0);
                }
            }
            Err(e) => s.status = format!("Recording failed: {e}"),
        }
        s.dirty |= dirty::TOOLS | dirty::DOC;
        return;
    }
    match media::audio::Recorder::start() {
        Ok(r) => {
            s.recorder = Some(r);
            s.rec_marks.clear();
            s.set_status("Recording… ink you write now is linked to the audio");
        }
        Err(e) => s.set_status(format!("No microphone available: {e}")),
    }
}

fn play_audio(s: &mut State) {
    if s.playing.take().is_some() {
        s.set_status("Stopped");
        return;
    }
    let Some(clip) = s.ws.as_ref().and_then(|w| w.page.audio.clone()) else {
        s.set_status("This page has no recording (press Record first)");
        return;
    };
    let start = clip.marks.iter().filter(|(id, _)| s.selection.contains(id)).map(|(_, t)| *t).min().unwrap_or(0);
    let Some(bytes) = s.ws.as_ref().and_then(|w| w.media(&clip.file)) else {
        s.set_status("The recording file is missing");
        return;
    };
    match media::audio::decode_ogg_opus(&bytes).and_then(|pcm| media::audio::play(pcm, start)) {
        Ok(stream) => {
            s.playing = Some(Box::new(stream));
            s.set_status(format!("Playing from {:.1} s (Play again stops)", start as f32 / 1000.0));
        }
        Err(e) => s.set_status(format!("Playback failed: {e}")),
    }
}

// ---- export, print, backups ---------------------------------------------------------------------

fn background_png(s: &State) -> Option<Vec<u8>> {
    let bg = s.background.as_ref()?;
    raster::composite_png(&s.doc.strokes, bg)
}

pub fn export(s: &mut State, f: &str) {
    s.sync_ws();
    let fmt = match f {
        "PNG" => export::Format::Png,
        "Markdown" => export::Format::Markdown,
        "Typst" => export::Format::Typst,
        "LaTeX" => export::Format::Latex,
        "HTML" => export::Format::Html,
        _ => export::Format::Pdf,
    };
    let now = s.now_ms();
    let dir = workspace::default_root().join("exports");
    let bg = background_png(s);
    let res = s.ws.as_mut().map(|w| w.export_current(fmt, &dir, now, bg));
    match res {
        Some(Ok(p)) => s.set_status(format!("Exported to {}", p.display())),
        Some(Err(e)) => s.set_status(format!("Export failed: {e}")),
        None => {}
    }
}

fn print(s: &mut State) {
    s.sync_ws();
    let bg = background_png(s);
    let Some(w) = &mut s.ws else { return };
    let media = w.page_media(&w.page.clone());
    let pdf = export::export_pages_with(&[&w.page], export::Format::Pdf, &media, &[bg]);
    let encrypted = w.is_encrypted();
    match pdf {
        Ok(pdf) => {
            // the OS print dialog needs a file: keep it in our own folder and delete it after an hour
            let dir = workspace::default_root().join(".print");
            let path = dir.join(format!("print-{}.pdf", store::new_id()));
            let r = store::fsio::atomic_write(&path, &pdf).map_err(|e| e.to_string()).and_then(|_| open_with_os(&path));
            s.set_status(match r {
                Ok(()) if encrypted => "Opened in your PDF viewer for printing. Note: this temporary copy is not encrypted; it is deleted within an hour.".to_string(),
                Ok(()) => "Opened in your PDF viewer: print from there".to_string(),
                Err(e) => format!("Could not open a PDF viewer: {e}"),
            });
        }
        Err(e) => s.set_status(format!("Print failed: {e}")),
    }
}

fn open_with_os(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(target_os = "windows"))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn backup_dirs(s: &State) -> (PathBuf, PathBuf) {
    let root = s.ws.as_ref().map(|w| w.store.root.clone()).unwrap_or_else(workspace::default_root);
    let dest = s.settings.backup_dir.clone().unwrap_or_else(|| {
        let parent = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
        parent.join("Scrawl backups")
    });
    (root, dest)
}

fn backup_info(ui: &MainWindow, s: &State) {
    let (_, dest) = backup_dirs(s);
    let list = store::backup::list_backups(&dest);
    let last = list.last().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "none yet".into());
    ui.set_backup_info(format!("{} backup(s) in {} (latest: {last}). Keeps the newest {}.", list.len(), dest.display(), s.settings.backup_keep).into());
}

thread_local! {
    static SAVED_SETTINGS: std::cell::RefCell<Option<Settings>> = const { std::cell::RefCell::new(None) };
}

/// Once a second: autosave, settings persistence, external-change check, daily backups,
/// trash purge and temp-file cleanup. Cheap when nothing changed.
pub fn housekeeping(ui: &MainWindow, s: &mut State) {
    let now = s.now_ms();
    if let Some(ws) = &mut s.ws {
        match ws.tick(now) {
            Ok(true) => s.dirty |= dirty::SIDEBAR | dirty::TABS,
            Ok(false) => {}
            Err(e) => {
                s.status = format!("Autosave failed: {e}. Your work is kept in memory; check disk space and permissions.");
                s.dirty |= dirty::TOOLS;
            }
        }
    }
    let changed = SAVED_SETTINGS.with(|c| c.borrow().as_ref() != Some(&s.settings));
    if changed {
        s.settings.save();
        SAVED_SETTINGS.with(|c| *c.borrow_mut() = Some(s.settings.clone()));
    }
    if now.saturating_sub(s.last_disk_check) > 3000 && s.overlay.is_empty() {
        s.last_disk_check = now;
        if s.ws.as_ref().is_some_and(|w| w.changed_on_disk()) {
            ask(ui, s, Prompt::ExternalChange, "This page changed on disk", "Another program (sync, git, an editor) changed this page. Type RELOAD to load that version, or press OK to keep yours.", false);
        }
    }
    if now.saturating_sub(s.last_backup_check) > 10 * 60_000 {
        let first = s.last_backup_check == 0;
        s.last_backup_check = now;
        if s.settings.backup_daily {
            let (root, dest) = backup_dirs(s);
            if let Ok(Some(p)) = store::backup::backup_if_due(&root, &dest, now, s.settings.backup_keep) {
                s.status = format!("Daily backup saved to {}", p.display());
                s.dirty |= dirty::TOOLS;
            }
        }
        if first {
            if let Some(w) = &s.ws {
                let _ = w.store.purge_trash(&w.nb.id, &w.vault, now);
            }
            let dir = workspace::default_root().join(".print");
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

// ---- ink modes and canvas text ------------------------------------------------------------------

/// Math / Text / Drawing. Text and Math both recognize ink; Text turns it into words.
pub fn set_mode(s: &mut State, m: &str) {
    s.mode = if m == "drawing" { InkMode::Drawing } else { InkMode::MathText };
    s.text_mode = m == "text";
    s.text_result.clear();
    clear_expression(s);
    if let Some(w) = &mut s.ws {
        let want = Some(m.to_string()).filter(|m| m != "math");
        if w.page.ink_mode != want {
            w.page.ink_mode = want;
            w.touch();
        }
    }
    if s.text_mode {
        s.set_status("Text mode: print letters separately, pause, then Insert ↵ adds the words to the page");
    }
    s.dirty |= dirty::TOOLS | dirty::STRIP;
}

fn text_at(s: &State, px: f32, py: f32) -> Option<u64> {
    s.ws.as_ref()?.page.texts.iter().rev().find(|t| {
        let (x0, y0, x1, y1) = t.bbox();
        px >= x0 - 4.0 && px <= x1 + 4.0 && py >= y0 - 4.0 && py <= y1 + 4.0
    }).map(|t| t.id)
}

fn snapshot_texts(s: &mut State) {
    s.text_redo.clear();
    if let Some(w) = &s.ws {
        s.text_history.push(w.page.texts.clone());
        if s.text_history.len() > 50 {
            s.text_history.remove(0);
        }
    }
    s.last_was_text = true;
}

/// Enter in the on-canvas text box: add, change or (when empty) delete the text.
pub fn textbox_commit(s: &mut State, text: &str) {
    let Some((id, x, y)) = s.textbox.take() else { return };
    snapshot_texts(s);
    let color = s.pen_color();
    let size = 18.0 / s.view.scale.max(0.1);
    if let Some(w) = &mut s.ws {
        let text = text.trim_end().to_string();
        match id {
            Some(id) => {
                if text.is_empty() {
                    w.page.texts.retain(|t| t.id != id);
                } else if let Some(t) = w.page.texts.iter_mut().find(|t| t.id == id) {
                    t.text = text;
                }
            }
            None if !text.is_empty() => {
                let id = w.page.texts.iter().map(|t| t.id).max().unwrap_or(0) + 1;
                w.page.texts.push(store::CanvasText { id, x, y, text, size, color });
            }
            None => {}
        }
        w.touch();
    }
    s.dirty |= dirty::CANVAS | dirty::TOOLS;
}

/// Undo the last canvas-text change if that was the most recent edit.
pub fn undo_text(s: &mut State) -> bool {
    if !s.last_was_text {
        return false;
    }
    let Some(prev) = s.text_history.pop() else {
        s.last_was_text = false;
        return false;
    };
    if let Some(w) = &mut s.ws {
        s.text_redo.push(std::mem::replace(&mut w.page.texts, prev));
        w.touch();
    }
    if s.text_history.is_empty() {
        s.last_was_text = false;
    }
    s.dirty |= dirty::CANVAS;
    true
}

/// Redo a canvas-text change that Undo reverted (until new ink or text edits clear it).
pub fn redo_text(s: &mut State) -> bool {
    let Some(next) = s.text_redo.pop() else { return false };
    if let Some(w) = &mut s.ws {
        s.text_history.push(std::mem::replace(&mut w.page.texts, next));
        w.touch();
    }
    s.last_was_text = true;
    s.dirty |= dirty::CANVAS;
    true
}

/// Add recognized words to the page: appended to the last text block, or a new one.
fn insert_words(s: &mut State) {
    let words = s.text_result.trim().to_string();
    if words.is_empty() {
        s.set_status("Nothing recognized yet: write some words and pause");
        return;
    }
    let used: Vec<u64> = s.session.symbols.iter().flat_map(|x| x.stroke_ids.clone()).collect();
    if let Some(w) = &mut s.ws {
        match w.page.blocks.last_mut() {
            Some(store::Block::Text { markup }) if s.editing.is_none() => {
                if !markup.ends_with('\n') && !markup.is_empty() {
                    markup.push(' ');
                }
                markup.push_str(&words);
            }
            _ => w.page.blocks.push(store::Block::Text { markup: words.clone() }),
        }
        w.touch();
    }
    if !used.is_empty() {
        s.doc.remove_ids(&used);
        s.sync_ws();
    }
    clear_expression(s);
    s.text_result.clear();
    s.set_status(format!("Added “{words}” to the page (Undo brings the ink back)"));
    s.dirty |= dirty::DOC;
}

/// A key pressed while the page has focus and no text box is open. Printable keys start typing
/// right away (seamless notes); other keys (Delete, F-keys...) go to the shortcut table.
pub fn plain_key(ui: &MainWindow, s: &mut State, key: &str) -> bool {
    let printable = key.chars().count() == 1 && key.chars().all(|c| !c.is_control() && !('\u{F700}'..='\u{F8FF}').contains(&c));
    if !printable || s.typing || s.rebinding.is_some() {
        return shortcut(ui, s, key, false, false, false, false);
    }
    // 1-5 pick a recognition candidate when there are candidates to pick
    if key.chars().all(|c| ('1'..='5').contains(&c)) && s.session.selected.is_some() && !s.text_mode {
        return false;
    }
    // the click that placed the cursor left a dot of ink: typing right after it removes the dot
    if let (Some((cx, cy)), Some(last)) = (s.type_cursor, s.doc.strokes.last()) {
        let tiny = ink::bbox(&last.points).is_some_and(|r| r.diagonal() < 3.0 / s.view.scale.max(0.1));
        let here = last.points.first().is_some_and(|p| (p.x - cx).abs() < 0.5 && (p.y - cy).abs() < 0.5);
        if tiny && here {
            let id = last.id;
            s.doc.remove_ids(&[id]);
            s.sync_ws();
            clear_expression(s);
            s.text_result.clear();
        }
    }
    let (x, y) = s.type_cursor.unwrap_or_else(|| s.view.to_page(60.0, 60.0));
    s.textbox = Some((None, x, y));
    s.textbox_typed = true;
    s.textbox_first = key.to_string();
    s.dirty |= dirty::TOOLS;
    true
}

/// Enter (`next_line`) or Esc in the canvas text box: place the text; Enter on new text opens a
/// fresh box on the next line, like a notepad. An empty line ends typing.
pub fn textbox_finish(s: &mut State, text: &str, next_line: bool) {
    let Some((id, x, y)) = s.textbox else { return };
    let was_new = id.is_none();
    textbox_commit(s, text);
    s.textbox_typed = false;
    s.textbox_first.clear();
    let line = 18.0 / s.view.scale.max(0.1) * 1.5;
    s.type_cursor = Some((x, y + line));
    if next_line && was_new && !text.trim().is_empty() {
        s.textbox = Some((None, x, y + line));
    }
    s.dirty |= dirty::TOOLS;
}
