mod raster;

use ink::{is_scribble, smooth_stroke, Document, EraserMode, InkMode, PenKind, Point, SmoothingConfig, Stroke};
use raster::View;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

slint::include_modules!();

const PALETTE: [[u8; 3]; 6] = [
    [0xc0, 0xca, 0xf5],
    [0xf7, 0x76, 0x8e],
    [0x9e, 0xce, 0x6a],
    [0x7a, 0xa2, 0xf7],
    [0xe0, 0xaf, 0x68],
    [0xbb, 0x9a, 0xf7],
];

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Pen,
    Highlighter,
    Eraser,
}

struct State {
    doc: Document,
    view: View,
    size: (u32, u32),
    tool: Tool,
    mode: InkMode,
    eraser_mode: EraserMode,
    smoothing: f32,
    width: f32,
    color: usize,
    live: Vec<Point>,
    eraser_path: Vec<(f32, f32)>,
    down_at: Option<Instant>,
}

impl State {
    fn new() -> Self {
        Self {
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
        }
    }

    fn pen_color(&self) -> [u8; 4] {
        let c = PALETTE[self.color];
        let a = if self.tool == Tool::Highlighter { 0x66 } else { 0xff };
        [c[0], c[1], c[2], a]
    }

    fn live_stroke(&self) -> Stroke {
        Stroke {
            id: u64::MAX,
            points: self.live.clone(),
            color: self.pen_color(),
            width: self.width,
            kind: if self.tool == Tool::Highlighter { PenKind::Highlighter } else { PenKind::Pen },
            mode: self.mode,
        }
    }
}

fn refresh(ui: &MainWindow, st: &State) {
    let scale = ui.window().scale_factor();
    let (w, h) = st.size;
    let live = (!st.live.is_empty() && st.tool != Tool::Eraser).then(|| st.live_stroke());
    let img = raster::render(
        st.doc.strokes.iter().chain(live.iter()),
        st.view,
        (w as f32 * scale) as u32,
        (h as f32 * scale) as u32,
        scale,
        [0x1a, 0x1b, 0x26],
    );
    ui.set_frame(img);
    ui.set_can_undo(st.doc.can_undo());
    ui.set_can_redo(st.doc.can_redo());
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let st = Rc::new(RefCell::new(State::new()));
    let weak = ui.as_weak();

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

    ui.on_canvas_resized(with!(|_ui, s, w = f32, h = f32| {
        s.size = (w.max(1.0) as u32, h.max(1.0) as u32);
    }));

    ui.on_pointer_down(with!(|_ui, s, x = f32, y = f32| {
        let (px, py) = s.view.to_page(x, y);
        s.down_at = Some(Instant::now());
        match s.tool {
            Tool::Eraser => s.eraser_path = vec![(px, py)],
            _ => s.live = vec![Point::new(px, py, 0)],
        }
    }));

    ui.on_pointer_move(with!(|_ui, s, x = f32, y = f32| {
        let Some(t0) = s.down_at else { return };
        let (px, py) = s.view.to_page(x, y);
        match s.tool {
            Tool::Eraser => {
                s.eraser_path.push((px, py));
                // Live feedback only for stroke mode; pixel mode commits on release.
            }
            _ => {
                let t = t0.elapsed().as_millis() as u32;
                s.live.push(Point::new(px, py, t));
            }
        }
    }));

    ui.on_pointer_up(with!(|ui, s| {
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
                    // Scribble-over-to-delete: the scribble itself is not kept.
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
    }));

    ui.on_zoom(with!(|_ui, s, f = f32, x = f32, y = f32| s.view.zoom_at(f, x, y)));
    ui.on_pan(with!(|_ui, s, dx = f32, dy = f32| {
        s.view.ox += dx;
        s.view.oy += dy;
    }));

    ui.on_set_tool(with!(|ui, s, t = slint::SharedString| {
        s.tool = match t.as_str() {
            "highlighter" => Tool::Highlighter,
            "eraser" => Tool::Eraser,
            _ => Tool::Pen,
        };
        ui.set_tool(t);
    }));
    ui.on_set_mode(with!(|ui, s, m = slint::SharedString| {
        s.mode = if m == "drawing" { InkMode::Drawing } else { InkMode::MathText };
        ui.set_mode(m);
    }));
    ui.on_toggle_pixel_erase(with!(|ui, s| {
        s.eraser_mode = if s.eraser_mode == EraserMode::Stroke { EraserMode::Pixel } else { EraserMode::Stroke };
        ui.set_pixel_erase(s.eraser_mode == EraserMode::Pixel);
    }));
    ui.on_undo(with!(|_ui, s| {
        s.doc.undo();
    }));
    ui.on_redo(with!(|_ui, s| {
        s.doc.redo();
    }));
    ui.on_clear_all(with!(|_ui, s| {
        let ids: Vec<u64> = s.doc.strokes.iter().map(|x| x.id).collect();
        s.doc.remove_ids(&ids); // undoable
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
    ui.on_recognize_now(with!(|ui, _s| {
        ui.set_status("Recognition is not implemented yet (phase 2)".into());
    }));

    ui.run()
}
