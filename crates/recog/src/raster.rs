//! Rasterize a symbol (one or more strokes) to the 32x32 CNN input.
//! Must stay in sync with `training/common.py::normalize`: fit the longest side
//! to 28 px preserving aspect, centre in 32x32, ink = 1.0.

use ink::Stroke;
use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform};

pub const SIZE: usize = 32;
const SS: usize = 4; // supersampling
const FIT: f32 = 28.0;
/// Pen thickness in output pixels (HASYv2 strokes are about this thick).
const PEN_PX: f32 = 2.2;

pub type Img = [f32; SIZE * SIZE];

pub fn rasterize(strokes: &[&Stroke]) -> Img {
    let mut out = [0.0f32; SIZE * SIZE];
    let pts: Vec<&ink::Point> = strokes.iter().flat_map(|s| s.points.iter()).collect();
    if pts.is_empty() {
        return out;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let side = w.max(h).max(1e-3);
    // Leave room so the stroke thickness stays inside the 28 px box.
    let scale = (FIT - PEN_PX) / side;
    let big = SIZE * SS;
    let Some(mut pm) = Pixmap::new(big as u32, big as u32) else { return out };
    let ox = (big as f32 - w * scale * SS as f32) / 2.0 - x0 * scale * SS as f32;
    let oy = (big as f32 - h * scale * SS as f32) / 2.0 - y0 * scale * SS as f32;
    let mut paint = Paint::default();
    paint.set_color(Color::WHITE);
    paint.anti_alias = true;
    let sk = SkStroke {
        width: PEN_PX * SS as f32,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..SkStroke::default()
    };
    for s in strokes {
        let mut pb = PathBuilder::new();
        for (i, p) in s.points.iter().enumerate() {
            let (x, y) = (p.x * scale * SS as f32 + ox, p.y * scale * SS as f32 + oy);
            if i == 0 {
                pb.move_to(x, y);
                if s.points.len() == 1 {
                    pb.line_to(x + 0.01, y);
                }
            } else {
                pb.line_to(x, y);
            }
        }
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &paint, &sk, Transform::identity(), None);
        }
    }
    let data = pm.data(); // premultiplied RGBA; white ink => alpha channel is coverage
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut acc = 0u32;
            for dy in 0..SS {
                for dx in 0..SS {
                    acc += data[((y * SS + dy) * big + x * SS + dx) * 4 + 3] as u32;
                }
            }
            out[y * SIZE + x] = acc as f32 / (255.0 * (SS * SS) as f32);
        }
    }
    out
}
