//! CPU rasterizer for ink strokes (tiny-skia).

use ink::{PenKind, Point, Stroke};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform};

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { scale: 1.0, ox: 0.0, oy: 0.0 }
    }
}

impl View {
    pub fn to_page(self, sx: f32, sy: f32) -> (f32, f32) {
        ((sx - self.ox) / self.scale, (sy - self.oy) / self.scale)
    }
    /// Zoom by `factor` keeping the screen point (sx, sy) fixed.
    pub fn zoom_at(&mut self, factor: f32, sx: f32, sy: f32) {
        let ns = (self.scale * factor).clamp(0.1, 16.0);
        let f = ns / self.scale;
        self.ox = sx - (sx - self.ox) * f;
        self.oy = sy - (sy - self.oy) * f;
        self.scale = ns;
    }
}

fn draw_stroke(pm: &mut Pixmap, s: &Stroke, view: View, px_scale: f32) {
    let Some(first) = s.points.first() else { return };
    let mut paint = Paint::default();
    let [r, g, b, a] = s.color;
    paint.set_color(Color::from_rgba8(r, g, b, a));
    paint.anti_alias = true;
    let width = match s.kind {
        PenKind::Pen => s.width,
        PenKind::Highlighter => s.width * 4.0,
    } * view.scale
        * px_scale;
    let sk = SkStroke {
        width: width.max(1.0),
        line_cap: if s.kind == PenKind::Highlighter { LineCap::Butt } else { LineCap::Round },
        line_join: LineJoin::Round,
        ..SkStroke::default()
    };
    let to_px = |p: &Point| ((p.x * view.scale + view.ox) * px_scale, (p.y * view.scale + view.oy) * px_scale);
    let mut pb = PathBuilder::new();
    let (x, y) = to_px(first);
    pb.move_to(x, y);
    if s.points.len() == 1 {
        pb.line_to(x + 0.01, y);
    }
    for p in &s.points[1..] {
        let (x, y) = to_px(p);
        pb.line_to(x, y);
    }
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint, &sk, Transform::identity(), None);
    }
}

/// Render all strokes. Highlighters are drawn first so they sit under ink.
pub fn render<'a>(
    strokes: impl Iterator<Item = &'a Stroke> + Clone,
    view: View,
    w: u32,
    h: u32,
    px_scale: f32,
    bg: [u8; 3],
) -> Image {
    let (w, h) = (w.max(1), h.max(1));
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    if let Some(mut pm) = Pixmap::new(w, h) {
        pm.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
        for s in strokes.clone().filter(|s| s.kind == PenKind::Highlighter) {
            draw_stroke(&mut pm, s, view, px_scale);
        }
        for s in strokes.filter(|s| s.kind == PenKind::Pen) {
            draw_stroke(&mut pm, s, view, px_scale);
        }
        // tiny-skia pixmaps are premultiplied RGBA8, identical layout to the buffer.
        buf.make_mut_bytes().copy_from_slice(pm.data());
    }
    Image::from_rgba8_premultiplied(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut v = View::default();
        let before = v.to_page(200.0, 150.0);
        v.zoom_at(2.0, 200.0, 150.0);
        let after = v.to_page(200.0, 150.0);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        assert!((v.scale - 2.0).abs() < 1e-6);
    }
}
