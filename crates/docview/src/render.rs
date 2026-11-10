use crate::layout::{Fonts, Item, ItemKind, META_CODE, META_LINK, META_MATH, META_UNDERLINE};
use crate::Objects;
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, SwashContent};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

#[derive(Clone, Copy, Debug)]
pub struct Colors {
    pub text: [u8; 3],
    pub link: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
}

fn fill(pm: &mut Pixmap, r: (f32, f32, f32, f32), c: [u8; 3], a: u8) {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(c[0], c[1], c[2], a));
    if let Some(rect) = Rect::from_xywh(r.0, r.1, r.2.max(0.5), r.3.max(0.5)) {
        pm.fill_path(&PathBuilder::from_rect(rect), &p, FillRule::Winding, Transform::identity(), None);
    }
}

/// Blend a glyph bitmap (alpha mask or colour) onto the premultiplied pixmap.
#[allow(clippy::too_many_arguments)]
fn blit(pm: &mut Pixmap, left: i32, top: i32, w: u32, h: u32, data: &[u8], content: SwashContent, color: [u8; 3]) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let stride = pw as usize * 4;
    let buf = pm.data_mut();
    for gy in 0..h as i32 {
        let y = top + gy;
        if y < 0 || y >= ph {
            continue;
        }
        for gx in 0..w as i32 {
            let x = left + gx;
            if x < 0 || x >= pw {
                continue;
            }
            let i = (gy as usize * w as usize + gx as usize) * if content == SwashContent::Mask { 1 } else { 4 };
            let (sr, sg, sb, sa) = match content {
                SwashContent::Mask => (color[0] as u32, color[1] as u32, color[2] as u32, data[i] as u32),
                SwashContent::Color => (data[i] as u32, data[i + 1] as u32, data[i + 2] as u32, data[i + 3] as u32),
                SwashContent::SubpixelMask => (color[0] as u32, color[1] as u32, color[2] as u32, ((data[i] as u32 + data[i + 1] as u32 + data[i + 2] as u32) / 3)),
            };
            if sa == 0 {
                continue;
            }
            let o = y as usize * stride + x as usize * 4;
            let inv = 255 - sa;
            let (pr, pg, pb) = if content == SwashContent::Color { (sr, sg, sb) } else { (sr * sa / 255, sg * sa / 255, sb * sa / 255) };
            buf[o] = (pr + buf[o] as u32 * inv / 255).min(255) as u8;
            buf[o + 1] = (pg + buf[o + 1] as u32 * inv / 255).min(255) as u8;
            buf[o + 2] = (pb + buf[o + 2] as u32 * inv / 255).min(255) as u8;
            buf[o + 3] = (sa + buf[o + 3] as u32 * inv / 255).min(255) as u8;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_buffer(fonts: &mut Fonts, buf: &Buffer, pm: &mut Pixmap, ox: f32, scale: f32, colors: &Colors, math: &[(usize, String, (f32, f32))], objs: &mut dyn Objects) {
    for run in buf.layout_runs() {
        for g in run.glyphs {
            if g.metadata & META_MATH != 0 {
                // inline math: its image replaces the placeholder
                if let Some((_, typst, (w, h))) = math.iter().find(|m| m.0 == g.start) {
                    if let Some(img) = objs.inline_pixmap(typst, scale, colors.text) {
                        let x = ((ox + g.x) * scale).round() as i32;
                        let y = ((run.line_top + (run.line_height - h) / 2.0) * scale).round() as i32;
                        pm.draw_pixmap(x, y, img.as_ref().as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
                    } else {
                        fill(pm, ((ox + g.x) * scale, (run.line_top + 2.0) * scale, w * scale, (run.line_height - 4.0) * scale), colors.muted, 60);
                    }
                }
                continue;
            }
            let color = if g.metadata & META_LINK != 0 { colors.link } else { colors.text };
            let p = g.physical((ox * scale, run.line_y * scale), scale);
            if let Some(img) = fonts.swash.get_image(&mut fonts.fs, p.cache_key).as_ref() {
                blit(pm, p.x + img.placement.left, p.y - img.placement.top, img.placement.width, img.placement.height, &img.data, img.content, color);
            }
            if g.metadata & META_CODE != 0 {
                fill(pm, ((ox + g.x) * scale, run.line_top * scale, g.w * scale, run.line_height * scale), colors.muted, 28);
            }
        }
        // underlines for links and underlined runs, one segment per consecutive run of glyphs
        let mut seg: Option<(f32, f32, usize)> = None;
        let flush = |pm: &mut Pixmap, seg: &mut Option<(f32, f32, usize)>| {
            if let Some((x0, x1, meta)) = seg.take() {
                let c = if meta & META_LINK != 0 { colors.link } else { colors.text };
                fill(pm, ((ox + x0) * scale, (run.line_y + 2.0) * scale, (x1 - x0) * scale, scale.max(1.0)), c, 255);
            }
        };
        for g in run.glyphs {
            if g.metadata & META_UNDERLINE != 0 {
                match &mut seg {
                    Some((_, x1, _)) if (g.x - *x1).abs() < 1.0 => *x1 = g.x + g.w,
                    _ => {
                        flush(pm, &mut seg);
                        seg = Some((g.x, g.x + g.w, g.metadata));
                    }
                }
            } else {
                flush(pm, &mut seg);
            }
        }
        flush(pm, &mut seg);
    }
}

/// Draw one paragraph into its own transparent pixmap (`col_w` x item height, times `scale`).
/// The caller places it at the item's position.
#[allow(clippy::too_many_arguments)]
pub fn render_item(fonts: &mut Fonts, it: &Item, kind: &notedoc::ParaKind, col_w: f32, scale: f32, colors: &Colors, objs: &mut dyn Objects) -> Option<Pixmap> {
    let w = (col_w * scale).ceil().max(1.0) as u32;
    let h = (it.height * scale).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h)?;
    match it.kind {
        ItemKind::Object => {
            if let Some(img) = objs.block_pixmap(kind, col_w, scale, colors.text) {
                // centred display math, left-aligned everything else
                let x = if matches!(kind, notedoc::ParaKind::MathDisplay { .. }) { ((w as i32 - img.width() as i32) / 2).max(0) } else { 0 };
                pm.draw_pixmap(x, 0, img.as_ref().as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
            } else if matches!(kind, notedoc::ParaKind::Rule) {
                fill(&mut pm, (0.0, (it.height / 2.0) * scale, w as f32, scale.max(1.0)), colors.muted, 255);
            }
        }
        ItemKind::Text => {
            if let notedoc::ParaKind::Quote | notedoc::ParaKind::Callout(_) = kind {
                fill(&mut pm, (4.0 * scale, 2.0 * scale, 3.0 * scale, (it.height - 4.0) * scale), colors.accent, 255);
            }
            if it.marker == "☐" || it.marker == "☑" {
                // the bundled fonts have no checkbox glyphs, so draw one
                let s = 15.0;
                let (x, y) = (it.look.indent - 8.0 - s - 3.0, (it.look.line - s) / 2.0 + 1.0);
                let mut paint = Paint::default();
                paint.set_color(Color::from_rgba8(colors.accent[0], colors.accent[1], colors.accent[2], 255));
                paint.anti_alias = true;
                if let Some(r) = Rect::from_xywh(x * scale, y * scale, s * scale, s * scale) {
                    let path = PathBuilder::from_rect(r);
                    if it.marker == "☑" {
                        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                        let mut tick = PathBuilder::new();
                        tick.move_to((x + 3.0) * scale, (y + 8.0) * scale);
                        tick.line_to((x + 6.5) * scale, (y + 11.5) * scale);
                        tick.line_to((x + 12.0) * scale, (y + 4.0) * scale);
                        if let Some(tp) = tick.finish() {
                            let mut white = Paint::default();
                            white.set_color(Color::WHITE);
                            white.anti_alias = true;
                            pm.stroke_path(&tp, &white, &tiny_skia::Stroke { width: 2.0 * scale, ..Default::default() }, Transform::identity(), None);
                        }
                    } else {
                        pm.stroke_path(&path, &paint, &tiny_skia::Stroke { width: 1.6 * scale, ..Default::default() }, Transform::identity(), None);
                    }
                }
            } else if !it.marker.is_empty() {
                // marker, right-aligned in the indent
                let mut mbuf = Buffer::new(&mut fonts.fs, Metrics::new(it.look.size, it.look.line));
                mbuf.set_size(Some(it.look.indent), None);
                let a = Attrs::new().family(Family::Name(crate::layout::BODY_FONT));
                mbuf.set_rich_text([(it.marker.as_str(), a.clone())], &a, Shaping::Advanced, None);
                mbuf.shape_until_scroll(&mut fonts.fs, false);
                let mw: f32 = mbuf.layout_runs().next().map_or(0.0, |r| r.line_w);
                draw_buffer(fonts, &mbuf, &mut pm, (it.look.indent - 8.0 - mw).max(0.0), scale, colors, &[], objs);
            }
            if let Some(buf) = &it.buf {
                draw_buffer(fonts, buf, &mut pm, it.look.indent, scale, colors, &it.map.math, objs);
            }
        }
    }
    Some(pm)
}
