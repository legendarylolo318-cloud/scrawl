//! CPU rasterizer for ink strokes (tiny-skia).

use ink::{PenKind, Point, Stroke};
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaperKind {
    Blank,
    Lined,
    Grid,
    Dots,
    Engineering,
    Polar,
}

impl PaperKind {
    pub const ALL: [PaperKind; 6] = [Self::Blank, Self::Lined, Self::Grid, Self::Dots, Self::Engineering, Self::Polar];
    pub fn name(self) -> &'static str {
        match self {
            Self::Blank => "Blank",
            Self::Lined => "Lined",
            Self::Grid => "Grid",
            Self::Dots => "Dots",
            Self::Engineering => "Engineering",
            Self::Polar => "Polar",
        }
    }
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Page background: kind, spacing in page units, line colour (RGBA).
#[derive(Clone, Copy, Debug)]
pub struct Paper {
    pub kind: PaperKind,
    pub spacing: f32,
    pub color: [u8; 4],
}

impl Default for Paper {
    fn default() -> Self {
        Self { kind: PaperKind::Lined, spacing: 32.0, color: [0x41, 0x48, 0x68, 0xff] }
    }
}

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

fn draw_stroke(pm: &mut Pixmap, s: &Stroke, view: View, px_scale: f32, light: bool) {
    let Some(first) = s.points.first() else { return };
    let mut paint = Paint::default();
    let [r, g, b, a] = s.color;
    // Near-white ink (the dark theme's default pen) is drawn dark on light paper, and vice versa.
    let (r, g, b) = if light && r > 200 && g > 200 && b > 200 { (0x1f, 0x23, 0x35) } else if !light && r < 50 && g < 50 && b < 60 { (0xc0, 0xca, 0xf5) } else { (r, g, b) };
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

fn line(pm: &mut Pixmap, paint: &Paint, w: f32, a: (f32, f32), b: (f32, f32)) {
    let mut pb = PathBuilder::new();
    pb.move_to(a.0, a.1);
    pb.line_to(b.0, b.1);
    if let Some(p) = pb.finish() {
        let sk = SkStroke { width: w, ..SkStroke::default() };
        pm.stroke_path(&p, paint, &sk, Transform::identity(), None);
    }
}

/// Draw the page background in page space so it zooms and pans with the ink.
pub fn draw_paper(pm: &mut Pixmap, paper: Paper, view: View, px_scale: f32) {
    if paper.kind == PaperKind::Blank {
        return;
    }
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    let step = paper.spacing.max(4.0);
    // Page-space range that is visible.
    let to_px = |x: f32, y: f32| ((x * view.scale + view.ox) * px_scale, (y * view.scale + view.oy) * px_scale);
    let (px0, py0) = (((0.0 / px_scale) - view.ox) / view.scale, ((0.0 / px_scale) - view.oy) / view.scale);
    let (px1, py1) = (((w / px_scale) - view.ox) / view.scale, ((h / px_scale) - view.oy) / view.scale);
    let c = paper.color;
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(c[0], c[1], c[2], c[3]));
    paint.anti_alias = true;
    let hair = (px_scale).max(1.0);
    // Skip when lines would be denser than ~5 px.
    if step * view.scale * px_scale < 5.0 {
        return;
    }
    let first = |lo: f32| (lo / step).floor() as i32 - 1;
    let last = |hi: f32| (hi / step).ceil() as i32 + 1;
    match paper.kind {
        PaperKind::Blank => {}
        PaperKind::Lined => {
            for i in first(py0)..=last(py1) {
                let (_, y) = to_px(0.0, i as f32 * step);
                line(pm, &paint, hair, (0.0, y), (w, y));
            }
            // left margin rule
            let (mx, _) = to_px(step * 2.5, 0.0);
            let mut m = Paint::default();
            m.set_color(Color::from_rgba8(0xf7, 0x76, 0x8e, 0x90));
            line(pm, &m, hair, (mx, 0.0), (mx, h));
        }
        PaperKind::Grid | PaperKind::Engineering => {
            let eng = paper.kind == PaperKind::Engineering;
            let mut major = Paint::default();
            major.set_color(if eng { Color::from_rgba8(0x73, 0xda, 0xca, 0x90) } else { Color::from_rgba8(c[0], c[1], c[2], c[3]) });
            for i in first(px0)..=last(px1) {
                let (x, _) = to_px(i as f32 * step, 0.0);
                let is_major = eng && i % 5 == 0;
                line(pm, if is_major { &major } else { &paint }, if is_major { hair * 1.6 } else { hair }, (x, 0.0), (x, h));
            }
            for i in first(py0)..=last(py1) {
                let (_, y) = to_px(0.0, i as f32 * step);
                let is_major = eng && i % 5 == 0;
                line(pm, if is_major { &major } else { &paint }, if is_major { hair * 1.6 } else { hair }, (0.0, y), (w, y));
            }
        }
        PaperKind::Dots => {
            let r = (1.2 * px_scale * view.scale.sqrt()).max(1.0);
            let mut pb = PathBuilder::new();
            for i in first(px0)..=last(px1) {
                for j in first(py0)..=last(py1) {
                    let (x, y) = to_px(i as f32 * step, j as f32 * step);
                    pb.push_circle(x, y, r);
                }
            }
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }
        PaperKind::Polar => {
            let (cx, cy) = to_px(0.0, 0.0);
            let max_r = {
                let far = |x: f32, y: f32| (x - cx).hypot(y - cy);
                far(0.0, 0.0).max(far(w, 0.0)).max(far(0.0, h)).max(far(w, h))
            };
            let ring = step * view.scale * px_scale;
            let mut r = ring;
            while r < max_r {
                let mut pb = PathBuilder::new();
                pb.push_circle(cx, cy, r);
                if let Some(p) = pb.finish() {
                    pm.stroke_path(&p, &paint, &SkStroke { width: hair, ..SkStroke::default() }, Transform::identity(), None);
                }
                r += ring;
            }
            for k in 0..12 {
                let a = k as f32 * std::f32::consts::TAU / 12.0;
                line(pm, &paint, hair, (cx, cy), (cx + max_r * a.cos(), cy + max_r * a.sin()));
            }
        }
    }
}

/// Size of the fixed page modes in page units (1 unit = 1 pt): Letter and A4.
pub fn page_size(mode: &str) -> Option<(f32, f32)> {
    match mode {
        "letter" => Some((612.0, 792.0)),
        "a4" => Some((595.0, 842.0)),
        _ => None,
    }
}

/// A raster background for the page (an imported PDF page), placed at page (0, 0).
pub struct Background {
    pub pm: Pixmap,
    /// Pixels per page unit the image was rendered at.
    pub scale: f32,
}

/// Overlays drawn on top of the ink (never cached).
#[derive(Default)]
pub struct Overlay<'a> {
    /// Typed canvas text: (page x, page y, rendered pixmap). Part of the cached layer.
    pub texts: &'a [(f32, f32, std::sync::Arc<Pixmap>)],
    /// Changes whenever `texts` changes (content, position or zoom).
    pub texts_key: u64,
    pub lasso: &'a [(f32, f32)],
    pub selection: Option<ink::Rect>,
    pub light: bool,
    /// Document editor: selection highlight rectangles and the caret (x, y, height), page units.
    pub doc_sel: &'a [(f32, f32, f32, f32)],
    pub caret: Option<(f32, f32, f32)>,
    pub accent: [u8; 3],
}

#[allow(clippy::too_many_arguments)]
fn fingerprint(strokes: &[Stroke], view: View, w: u32, h: u32, px_scale: f32, bg: [u8; 3], paper: Paper, extra: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (w, h, bg, paper.kind as u8, paper.color, extra).hash(&mut hs);
    for v in [view.scale, view.ox, view.oy, px_scale, paper.spacing] {
        v.to_bits().hash(&mut hs);
    }
    strokes.len().hash(&mut hs);
    for s in strokes {
        (s.id, s.points.len(), s.color).hash(&mut hs);
        if let (Some(a), Some(b)) = (s.points.first(), s.points.last()) {
            (a.x.to_bits(), a.y.to_bits(), b.x.to_bits(), b.y.to_bits(), s.width.to_bits()).hash(&mut hs);
        }
    }
    hs.finish()
}

/// Cached canvas renderer: paper, background and finished ink are rasterized once; while the pen
/// moves only the live stroke is drawn on top of a copy of that cache.
#[derive(Default)]
pub struct Canvas {
    key: u64,
    base: Option<Pixmap>,
    pub base_renders: u64,
}

impl Canvas {
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        strokes: &[Stroke],
        live: Option<&Stroke>,
        view: View,
        (w, h): (u32, u32),
        px_scale: f32,
        bg: [u8; 3],
        paper: Paper,
        page: Option<(f32, f32)>,
        background: Option<&Background>,
        overlay: &Overlay,
    ) -> Image {
        let (w, h) = (w.max(1), h.max(1));
        let extra = page.map_or(0, |(a, b)| (a as u64) << 32 | b as u64) ^ background.map_or(0, |b| b.pm.width() as u64 * 7919 + b.pm.height() as u64) ^ overlay.light as u64 ^ overlay.texts_key.rotate_left(17);
        let key = fingerprint(strokes, view, w, h, px_scale, bg, paper, extra);
        if self.key != key || self.base.is_none() {
            self.base = Pixmap::new(w, h).map(|mut pm| {
                pm.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
                if let Some(b) = background {
                    let s = view.scale * px_scale / b.scale;
                    let t = Transform::from_scale(s, s).post_translate(view.ox * px_scale, view.oy * px_scale);
                    pm.draw_pixmap(0, 0, b.pm.as_ref(), &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() }, t, None);
                }
                draw_paper(&mut pm, paper, view, px_scale);
                if let Some(size) = page {
                    draw_page_frames(&mut pm, size, view, px_scale, overlay.light);
                }
                for (x, y, tp) in overlay.texts {
                    let (sx, sy) = ((x * view.scale + view.ox) * px_scale, (y * view.scale + view.oy) * px_scale);
                    pm.draw_pixmap(sx as i32, sy as i32, tp.as_ref().as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
                }
                for s in strokes.iter().filter(|s| s.kind == PenKind::Highlighter) {
                    draw_stroke(&mut pm, s, view, px_scale, overlay.light);
                }
                for s in strokes.iter().filter(|s| s.kind == PenKind::Pen) {
                    draw_stroke(&mut pm, s, view, px_scale, overlay.light);
                }
                pm
            });
            self.key = key;
            self.base_renders += 1;
        }
        let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
        if let Some(base) = &self.base {
            let needs_overlay = live.is_some() || !overlay.lasso.is_empty() || overlay.selection.is_some() || overlay.caret.is_some() || !overlay.doc_sel.is_empty();
            if !needs_overlay {
                buf.make_mut_bytes().copy_from_slice(base.data());
            } else {
                let mut pm = base.clone();
                if let Some(s) = live {
                    draw_stroke(&mut pm, s, view, px_scale, overlay.light);
                }
                draw_overlay(&mut pm, overlay, view, px_scale);
                buf.make_mut_bytes().copy_from_slice(pm.data());
            }
        }
        Image::from_rgba8_premultiplied(buf)
    }
}

fn draw_page_frames(pm: &mut Pixmap, (pw, ph): (f32, f32), view: View, px_scale: f32, light: bool) {
    let mut paint = Paint::default();
    paint.set_color(if light { Color::from_rgba8(0x40, 0x48, 0x68, 0xb0) } else { Color::from_rgba8(0x7a, 0xa2, 0xf7, 0xa0) });
    paint.anti_alias = true;
    let to_px = |x: f32, y: f32| ((x * view.scale + view.ox) * px_scale, (y * view.scale + view.oy) * px_scale);
    let visible_bottom = ((pm.height() as f32 / px_scale) - view.oy) / view.scale;
    let pages = (visible_bottom / ph).ceil().clamp(1.0, 500.0) as i32;
    for i in 0..pages {
        let (x0, y0) = to_px(0.0, i as f32 * ph);
        let (x1, y1) = to_px(pw, (i + 1) as f32 * ph);
        if let Some(r) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
            let path = PathBuilder::from_rect(r);
            pm.stroke_path(&path, &paint, &SkStroke { width: 1.5 * px_scale, dash: tiny_skia::StrokeDash::new(vec![8.0, 6.0], 0.0), ..SkStroke::default() }, Transform::identity(), None);
        }
    }
}

fn draw_overlay(pm: &mut Pixmap, o: &Overlay, view: View, px_scale: f32) {
    let to_px = |x: f32, y: f32| ((x * view.scale + view.ox) * px_scale, (y * view.scale + view.oy) * px_scale);
    {
        let [r, g, b] = o.accent;
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(r, g, b, 0x55));
        for (x, y, w, h) in o.doc_sel {
            let (x0, y0) = to_px(*x, *y);
            let (x1, y1) = to_px(x + w, y + h);
            if let Some(rr) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
                pm.fill_path(&PathBuilder::from_rect(rr), &p, FillRule::Winding, Transform::identity(), None);
            }
        }
        if let Some((x, y, h)) = o.caret {
            let (x0, y0) = to_px(x, y);
            let (_, y1) = to_px(x, y + h);
            p.set_color(Color::from_rgba8(r, g, b, 0xff));
            if let Some(rr) = tiny_skia::Rect::from_ltrb(x0 - px_scale, y0, x0 + 1.5 * px_scale, y1) {
                pm.fill_path(&PathBuilder::from_rect(rr), &p, FillRule::Winding, Transform::identity(), None);
            }
        }
    }
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(0xe0, 0xaf, 0x68, 0xe0));
    paint.anti_alias = true;
    let dash = || SkStroke { width: 1.5 * px_scale, dash: tiny_skia::StrokeDash::new(vec![6.0 * px_scale, 4.0 * px_scale], 0.0), ..SkStroke::default() };
    if o.lasso.len() > 1 {
        let mut pb = PathBuilder::new();
        let (x, y) = to_px(o.lasso[0].0, o.lasso[0].1);
        pb.move_to(x, y);
        for p in &o.lasso[1..] {
            let (x, y) = to_px(p.0, p.1);
            pb.line_to(x, y);
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint, &dash(), Transform::identity(), None);
        }
    }
    if let Some(r) = o.selection {
        let (x0, y0) = to_px(r.x0 - 6.0, r.y0 - 6.0);
        let (x1, y1) = to_px(r.x1 + 6.0, r.y1 + 6.0);
        if let Some(rr) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
            let path = PathBuilder::from_rect(rr);
            let mut fill = Paint::default();
            fill.set_color(Color::from_rgba8(0xe0, 0xaf, 0x68, 0x22));
            pm.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
            pm.stroke_path(&path, &paint, &dash(), Transform::identity(), None);
        }
    }
}

/// Render all strokes without the cache (tests). Highlighters are drawn under ink.
#[cfg(test)]
#[allow(dead_code)] // kept for tests and debugging
pub fn render<'a>(strokes: impl Iterator<Item = &'a Stroke> + Clone, view: View, w: u32, h: u32, px_scale: f32, bg: [u8; 3], paper: Paper) -> Image {
    let all: Vec<Stroke> = strokes.cloned().collect();
    Canvas::default().frame(&all, None, view, (w, h), px_scale, bg, paper, None, None, &Overlay::default())
}

/// Composite a PDF background with the page's ink for export (white paper, dark ink).
pub fn composite_png(strokes: &[Stroke], bg: &Background) -> Option<Vec<u8>> {
    let (w, h) = (bg.pm.width(), bg.pm.height());
    let mut pm = bg.pm.clone();
    let view = View { scale: bg.scale, ox: 0.0, oy: 0.0 };
    for s in strokes.iter().filter(|s| s.kind == PenKind::Highlighter).chain(strokes.iter().filter(|s| s.kind == PenKind::Pen)) {
        draw_stroke(&mut pm, s, view, 1.0, true);
    }
    let _ = (w, h);
    pm.encode_png().ok()
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

    #[test]
    fn every_paper_kind_draws_and_blank_does_not() {
        for kind in PaperKind::ALL {
            let mut pm = Pixmap::new(200, 200).unwrap();
            pm.fill(Color::from_rgba8(0, 0, 0, 255));
            draw_paper(&mut pm, Paper { kind, spacing: 20.0, ..Paper::default() }, View::default(), 1.0);
            let inked = pm.data().chunks(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count();
            if kind == PaperKind::Blank {
                assert_eq!(inked, 0);
            } else {
                assert!(inked > 100, "{kind:?} drew only {inked} px");
            }
        }
    }

    #[test]
    fn lined_paper_scrolls_with_the_view() {
        let draw = |oy: f32| {
            let mut pm = Pixmap::new(100, 100).unwrap();
            pm.fill(Color::from_rgba8(0, 0, 0, 255));
            draw_paper(&mut pm, Paper { kind: PaperKind::Lined, spacing: 20.0, ..Paper::default() }, View { scale: 1.0, ox: 0.0, oy }, 1.0);
            pm.data().to_vec()
        };
        assert_ne!(draw(0.0), draw(7.0));
        assert_eq!(draw(0.0), draw(20.0), "pattern repeats every spacing");
    }
}
