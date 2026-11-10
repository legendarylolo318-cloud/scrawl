//! Glue between the tablet server (`remote`) and the app: starting/stopping it, pairing UI,
//! and applying strokes / text / commands that arrive from the device.

use crate::{dirty, flush, run_recognition, update_expr, update_strip, DeviceRow, MainWindow, State, Tool};
use ink::{smooth_stroke, EraserMode, InkMode, PenKind, Point, SmoothingConfig, Stroke};
use input::Out;
use layout::{Accent, BigOp, Fence, MatrixKind};
use remote::protocol::{Command, NavKey, Pen, ServerMsg, Template};
use remote::server::{RemoteEvent, RemoteHandle, ServerConfig, StopReason};
use slint::{ComponentHandle, SharedPixelBuffer, SharedString};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Default)]
pub struct Tablet {
    handle: Option<RemoteHandle>,
    inflight: HashMap<u32, Vec<Point>>,
    pending: Vec<(String, String, String)>,
    pump: Option<slint::Timer>,
}

impl Tablet {
    pub fn running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.is_running())
    }

    /// Stop the server and release everything (also called when the window closes).
    pub fn stop(&mut self) {
        self.pump = None;
        self.pending.clear();
        self.inflight.clear();
        if let Some(h) = self.handle.take() {
            h.stop();
        }
    }
}

pub fn nav(st: &mut State, key: &str) {
    match key {
        "backspace" => st.editor.backspace(),
        "left" => st.editor.left(),
        "right" => st.editor.right(),
        "tab" => {
            st.editor.tab();
        }
        "shift_tab" => {
            st.editor.shift_tab();
        }
        _ => st.editor.insert_char(' '), // Enter: finish the pending shorthand word
    }
}

/// Send the current state, candidates and expression to the device (only while connected).
pub fn push_state(st: &State) {
    let Some(h) = &st.tablet.handle else { return };
    if h.connected_devices() == 0 {
        return;
    }
    let e = if st.typed_since_recog || !st.editor.expr().is_empty() { st.editor.expr() } else { st.session.expression() };
    h.send(ServerMsg::State { drawing: st.mode == InkMode::Drawing, can_undo: st.doc.can_undo(), can_redo: st.doc.can_redo(), expr: e.to_unicode() });
    // Render of the active block only (never other notes), so the device shows typeset math.
    let png = if e.is_empty() {
        Vec::new()
    } else {
        render::compile(&render::math_source(&e.to_typst(), 18.0, [0xc0, 0xca, 0xf5]), &[]).ok().and_then(|d| render::png(&d, 0, 2.0)).unwrap_or_default()
    };
    if png.len() < 512 * 1024 {
        h.send(ServerMsg::frame(&png));
    }
    let labels: Vec<String> = st.session.selected.and_then(|i| st.session.symbols.get(i)).map(|s| s.cands.iter().map(|c| c.label.clone()).collect()).unwrap_or_default();
    h.send(ServerMsg::Candidates { labels });
}

fn age(ms: u64) -> String {
    let d = store::now_ms().saturating_sub(ms) / 1000;
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86_400),
    }
}

fn refresh_ui(ui: &MainWindow, st: &State) {
    ui.set_tablet_running(st.tablet.running());
    let devices: Vec<DeviceRow> = st.tablet.handle.as_ref().map(|h| h.devices()).unwrap_or_default().into_iter().map(|d| DeviceRow { id: d.id.into(), name: d.name.into(), detail: format!("last seen {}", age(d.last_seen_ms)).into() }).collect();
    ui.set_tablet_devices(crate::model(devices));
    let pending: Vec<DeviceRow> = st.tablet.pending.iter().map(|(id, name, code)| DeviceRow { id: id.as_str().into(), name: name.as_str().into(), detail: code.as_str().into() }).collect();
    ui.set_tablet_pending(crate::model(pending));
}

fn qr_image(modules: &[Vec<bool>]) -> slint::Image {
    let n = modules.len();
    let (scale, quiet) = (6usize, 4usize);
    let px = (n + 2 * quiet) * scale;
    let mut buf = SharedPixelBuffer::<slint::Rgb8Pixel>::new(px as u32, px as u32);
    let bytes = buf.make_mut_bytes();
    bytes.fill(255);
    for (y, row) in modules.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (ix, iy) = ((x + quiet) * scale + dx, (y + quiet) * scale + dy);
                        let o = (iy * px + ix) * 3;
                        bytes[o..o + 3].copy_from_slice(&[0, 0, 0]);
                    }
                }
            }
        }
    }
    slint::Image::from_rgb8(buf)
}

fn template_out(t: &Template) -> Out {
    match t {
        Template::Fraction {} => Out::Frac,
        Template::Sqrt {} => Out::Sqrt,
        Template::Matrix { rows, cols } => Out::Matrix { kind: MatrixKind::Bracket, rows: *rows as usize, cols: *cols as usize },
        Template::Integral {} => Out::Big(BigOp::Int),
        Template::Sum {} => Out::Big(BigOp::Sum),
        Template::Limit {} => Out::Big(BigOp::Lim),
        Template::Cases {} => Out::Cases,
        Template::Abs {} => Out::Fence(Fence::Abs),
        Template::Norm {} => Out::Fence(Fence::Norm),
        Template::Deriv {} => Out::Deriv { partial: false },
        Template::Vector {} => Out::Accent(Accent::Arrow),
    }
}

/// Apply one event from the device. Returns true if ink changed (so recognition should be scheduled).
fn apply(ui: &MainWindow, st: &mut State, ev: RemoteEvent) -> bool {
    match ev {
        RemoteEvent::Strokes { id, pen, width, pts, done } => {
            let entry = st.tablet.inflight.entry(id).or_default();
            let mut t = entry.last().map_or(0, |p| p.t_ms);
            for p in &pts {
                t += p[5] as u32;
                entry.push(Point { x: p[0], y: p[1], pressure: p[2], tilt_x: p[3], tilt_y: p[4], t_ms: t });
            }
            if !done {
                return false;
            }
            let raw = st.tablet.inflight.remove(&id).unwrap_or_default();
            let page: Vec<Point> = raw.iter().map(|p| { let (x, y) = st.view.to_page(p.x, p.y); Point { x, y, ..*p } }).collect();
            match pen {
                Pen::Eraser => {
                    let path: Vec<(f32, f32)> = page.iter().map(|p| (p.x, p.y)).collect();
                    let r = (st.settings.width * 2.0 + 4.0) / st.view.scale;
                    let mode = st.eraser_mode;
                    st.doc.erase_path(&path, r, mode);
                }
                other => {
                    let kind = if other == Pen::Highlighter { PenKind::Highlighter } else { PenKind::Pen };
                    let c = st.color;
                    let stroke = Stroke {
                        id: 0,
                        points: smooth_stroke(&page, SmoothingConfig { strength: st.settings.smoothing }),
                        color: [c[0], c[1], c[2], if kind == PenKind::Highlighter { 0x66 } else { 0xff }],
                        width: width.max(0.5),
                        kind,
                        mode: st.mode,
                        t0_ms: st.epoch.elapsed().as_millis() as u64,
                    };
                    st.doc.add_stroke(stroke);
                }
            }
            st.sync_ws();
            true
        }
        RemoteEvent::Text(s) => {
            st.editor.insert_str(&s);
            st.typed_since_recog = true;
            update_expr(ui, st);
            push_state(st);
            false
        }
        RemoteEvent::Cmd(c) => {
            match c {
                Command::Undo {} => {
                    st.doc.undo();
                    st.sync_ws();
                    return true;
                }
                Command::Redo {} => {
                    st.doc.redo();
                    st.sync_ws();
                    return true;
                }
                Command::RecognizeNow {} => run_recognition(ui, st),
                Command::Mode { drawing } => st.mode = if drawing { InkMode::Drawing } else { InkMode::MathText },
                Command::Tool { pen } => st.tool = match pen { Pen::Pen => Tool::Pen, Pen::Highlighter => Tool::Highlighter, Pen::Eraser => Tool::Eraser },
                Command::Color { rgb } => {
                    let (r, g, b) = ((rgb >> 16) as i32 & 255, (rgb >> 8) as i32 & 255, rgb as i32 & 255);
                    st.color = [r as u8, g as u8, b as u8];
                    st.active_pen = None;
                }
                Command::Choose { i } => {
                    if st.session.choose(i as usize, &st.doc.strokes) {
                        if !st.typed_since_recog {
                            st.editor.set_expr(st.session.expression());
                        }
                        update_strip(ui, st);
                    }
                }
                Command::Template { t } => {
                    st.editor.insert_template(template_out(&t));
                    st.typed_since_recog = true;
                    update_expr(ui, st);
                }
                Command::Nav { key } => {
                    nav(st, match key { NavKey::Backspace => "backspace", NavKey::Left => "left", NavKey::Right => "right", NavKey::Tab => "tab", NavKey::ShiftTab => "shift_tab", NavKey::Enter => "enter" });
                    st.typed_since_recog = true;
                    update_expr(ui, st);
                }
            }
            push_state(st);
            false
        }
        RemoteEvent::PendingApproval { id, name, code } => {
            st.tablet.pending.push((id, name, code));
            ui.set_tablet_status("A device wants to pair. Check that the 6-digit code matches, then Approve.".into());
            ui.set_show_tablet(true);
            false
        }
        RemoteEvent::DeviceConnected(name) => {
            ui.set_status(format!("Tablet connected: {name}").into());
            push_state(st);
            false
        }
        RemoteEvent::DeviceDisconnected => {
            ui.set_status("Tablet disconnected".into());
            false
        }
        RemoteEvent::Stopped(reason) => {
            st.tablet.stop();
            ui.set_tablet_status(if reason == StopReason::Idle { "Tablet server stopped after being idle." } else { "Tablet server stopped." }.into());
            false
        }
    }
}

pub fn wire(ui: &MainWindow, st: &Rc<RefCell<State>>) {
    let weak = ui.as_weak();

    // start / stop
    {
        let (st, weak) = (st.clone(), weak.clone());
        ui.on_tablet_toggle(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            if s.tablet.running() {
                s.tablet.stop();
                ui.set_tablet_status("Tablet server stopped.".into());
                ui.set_has_qr(false);
                ui.set_tablet_fp("".into());
                refresh_ui(&ui, &s);
                return;
            }
            let dir = crate::session::config_dir().unwrap_or_else(std::env::temp_dir).join("tablet");
            match remote::server::start(ServerConfig::new(dir), Arc::new(remote::ca::KeyringStore)) {
                Ok(h) => {
                    ui.set_tablet_fp(h.ca_fingerprint.clone().into());
                    ui.set_tablet_status(format!("Listening on {}. First time? Press \"Setup\" to install the CA on your device, then \"Pair device\".", h.addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")).into());
                    s.tablet.handle = Some(h);
                    // poll events only while the server runs (no timers when it is off)
                    let (st2, weak2) = (st.clone(), weak.clone());
                    let timer = slint::Timer::default();
                    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(15), move || {
                        let mut events = Vec::new();
                        let changed_ui;
                        {
                            let s = st2.borrow();
                            if let Some(h) = &s.tablet.handle {
                                while let Ok(e) = h.events.try_recv() {
                                    events.push(e);
                                }
                            }
                        }
                        if events.is_empty() {
                            return;
                        }
                        let Some(ui) = weak2.upgrade() else { return };
                        let mut ink_changed = false;
                        {
                            let mut s = st2.borrow_mut();
                            for e in events {
                                ink_changed |= apply(&ui, &mut s, e);
                            }
                            changed_ui = true;
                            s.dirty |= dirty::CANVAS | dirty::TOOLS;
                            flush(&ui, &mut s);
                            refresh_ui(&ui, &s);
                        }
                        let _ = changed_ui;
                        if ink_changed {
                            ui.invoke_pen_lifted();
                        }
                    });
                    s.tablet.pump = Some(timer);
                }
                Err(e) => ui.set_tablet_status(format!("Could not start: {e}").into()),
            }
            refresh_ui(&ui, &s);
        });
    }

    // one-time CA setup screen (bootstrap endpoint)
    {
        let (st, weak) = (st.clone(), weak.clone());
        ui.on_tablet_setup(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            let Some(h) = &s.tablet.handle else { return };
            match h.open_bootstrap() {
                Ok(()) => ui.set_tablet_status(format!(
                    "CA setup open for 5 minutes (or one download). On the device open {}\n\niPhone/iPad: install the profile, then Settings > General > About > Certificate Trust Settings.\nAndroid: Settings > Security & privacy > More security & privacy > Encryption & credentials > Install a certificate > CA certificate.\n\nCompare the fingerprint below with the one on the device.",
                        h.bootstrap_urls.first().cloned().unwrap_or_default()).into()),
                Err(e) => ui.set_tablet_status(e.into()),
            }
        });
    }

    // new pairing QR
    {
        let (st, weak) = (st.clone(), weak.clone());
        ui.on_tablet_new_pair(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            let Some(h) = &s.tablet.handle else { return };
            let info = h.begin_pairing();
            ui.set_tablet_qr(qr_image(&info.qr));
            ui.set_has_qr(true);
            ui.set_tablet_status(format!("Scan this QR code with the tablet (valid {} minutes, single use):\n{}", info.expires_in.as_secs() / 60, info.url.split('#').next().unwrap_or("")).into());
        });
    }

    for kind in ["approve", "reject", "revoke"] {
        let (st, weak) = (st.clone(), weak.clone());
        let f = move |id: SharedString| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            let id = id.to_string();
            match kind {
                "approve" => {
                    if let Some(h) = &s.tablet.handle {
                        h.approve(&id);
                    }
                    s.tablet.pending.retain(|p| p.0 != id);
                    ui.set_has_qr(false);
                    ui.set_tablet_status("Device approved.".into());
                }
                "reject" => {
                    if let Some(h) = &s.tablet.handle {
                        h.reject(&id);
                    }
                    s.tablet.pending.retain(|p| p.0 != id);
                }
                _ => {
                    if let Some(h) = &s.tablet.handle {
                        h.revoke(&id);
                    }
                }
            }
            refresh_ui(&ui, &s);
        };
        match kind {
            "approve" => ui.on_tablet_approve(f),
            "reject" => ui.on_tablet_reject(f),
            _ => ui.on_tablet_revoke(f),
        }
    }
    {
        let (st, weak) = (st.clone(), weak.clone());
        ui.on_tablet_revoke_all(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = st.borrow();
            if let Some(h) = &s.tablet.handle {
                h.revoke_all();
            }
            refresh_ui(&ui, &s);
        });
    }
    let _ = EraserMode::Stroke;
}
