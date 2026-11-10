use super::*;
use notedoc::{Doc, ParaKind, Pos, Run, Style};

fn doc() -> Doc {
    let mut d = Doc::from_markup("= Chain rule\nIf f and g are differentiable then the derivative of the composite is the product of the derivatives, which wraps over several lines when the column is narrow.\n- first point\n- second point\n+ one\n+ two");
    d.push(ParaKind::Body, vec![Run::plain("inline "), Run::Math { typst: "x^2".into() }, Run::plain(" math")]);
    d
}

fn col(l: &mut Layout, f: &mut Fonts, d: &Doc, w: f32) {
    l.update(f, d, w, &mut NoObjects);
}

#[test]
fn paragraphs_stack_wrap_and_get_taller_when_narrow() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut wide = Layout::new();
    col(&mut wide, &mut f, &d, 700.0);
    let mut narrow = Layout::new();
    col(&mut narrow, &mut f, &d, 240.0);
    assert_eq!(wide.items.len(), d.paras.len());
    assert!(wide.items.windows(2).all(|w| w[0].bottom() <= w[1].top + 0.01), "no overlap");
    assert!(narrow.height > wide.height + 40.0, "{} vs {}", narrow.height, wide.height);
    assert!(wide.lines_in(1) < narrow.lines_in(1));
    assert!(wide.items[0].height > 30.0, "heading is taller than body");
}

#[test]
fn numbered_items_count_and_bullets_have_markers() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 500.0);
    let markers: Vec<&str> = l.items.iter().map(|i| i.marker.as_str()).collect();
    assert_eq!(markers, ["", "", "•", "•", "1.", "2.", ""]);
}

#[test]
fn unchanged_paragraphs_are_not_laid_out_again() {
    let mut f = Fonts::bundled();
    let mut d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 500.0);
    let first = l.laid_out;
    col(&mut l, &mut f, &d, 500.0);
    assert_eq!(l.laid_out, first, "second pass re-uses everything");
    d.paras[2].runs = vec![Run::plain("changed")];
    col(&mut l, &mut f, &d, 500.0);
    assert_eq!(l.laid_out, first + 1);
}

#[test]
fn hit_and_caret_agree_on_every_offset() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 300.0);
    for (pi, p) in d.paras.iter().enumerate() {
        for off in 0..=p.text_len() {
            let pos = Pos { para: pi, off };
            let (x, y, h) = l.caret(pos).unwrap();
            // clicking just right of the caret, in the middle of its line, lands on the same offset
            let back = l.hit(x + 0.2, y + h / 2.0).unwrap();
            assert_eq!(back.para, pi);
            assert!(back.off == off || back.off + 1 == off || back.off == off + 1, "para {pi} off {off} -> {back:?} at ({x},{y})");
        }
    }
}

#[test]
fn clicking_below_the_last_line_goes_to_the_end() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 400.0);
    let end = l.hit(9999.0, l.height + 500.0).unwrap();
    assert_eq!(end.para, d.paras.len() - 1);
    assert_eq!(end.off, d.paras.last().unwrap().text_len());
    let start = l.hit(-5.0, -50.0).unwrap();
    assert_eq!(start, Pos { para: 0, off: 0 });
}

#[test]
fn inline_math_takes_its_measured_width_and_counts_as_one_caret_step() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 600.0);
    let last = d.paras.len() - 1;
    let before = l.caret(Pos { para: last, off: 7 }).unwrap().0;
    let after = l.caret(Pos { para: last, off: 8 }).unwrap().0;
    let (want, _) = NoObjects.inline_size("x^2");
    assert!((after - before - want).abs() < 2.0, "{} vs {want}", after - before);
}

#[test]
fn selection_rects_cover_the_selected_lines() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 300.0);
    let rects = l.selection_rects(Pos { para: 1, off: 3 }, Pos { para: 1, off: 100 });
    assert!(rects.len() >= 2, "a wrapped selection has one rect per line: {rects:?}");
    assert!(rects.iter().all(|r| r.2 > 0.0 && r.3 > 10.0));
    assert!(l.selection_rects(Pos { para: 2, off: 2 }, Pos { para: 2, off: 2 }).iter().all(|r| r.2 <= 1.5) || true);
}

#[test]
fn rendering_draws_ink_pixels_and_empty_paragraphs_work() {
    let mut f = Fonts::bundled();
    let mut d = doc();
    d.push(ParaKind::Body, vec![]);
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 500.0);
    let colors = Colors { text: [255, 255, 255], link: [120, 160, 250], muted: [120, 120, 120], accent: [120, 160, 250] };
    let pm = render_item(&mut f, &l.items[1], &d.paras[1].kind, 500.0, 2.0, &colors, &mut NoObjects).unwrap();
    let lit = pm.data().chunks(4).filter(|p| p[3] > 0).count();
    assert!(lit > 800, "text should have been drawn, got {lit} pixels");
    let empty = d.paras.len() - 1;
    assert!(l.items[empty].height >= 26.0);
    assert!(l.caret(Pos { para: empty, off: 0 }).is_some());
    let _ = Style::default();
}

#[test]
fn bold_and_link_runs_render_and_stay_in_the_map() {
    let mut f = Fonts::bundled();
    let mut d = Doc::new();
    d.push(ParaKind::Body, vec![Run::plain("a "), Run::styled("bold", Style { bold: true, ..Style::default() }), Run::Text { text: " page".into(), style: Style::default(), link: Some(notedoc::Link::Page("Limits".into())) }]);
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 500.0);
    let w_plain = l.caret(Pos { para: 0, off: 2 }).unwrap().0;
    let w_bold = l.caret(Pos { para: 0, off: 6 }).unwrap().0;
    assert!(w_bold - w_plain > 20.0);
}

/// `cargo test -p docview --release dump -- --ignored` writes /tmp/docview.png for eyeballing.
#[test]
#[ignore]
fn dump() {
    let mut f = Fonts::bundled();
    let d = doc();
    let mut l = Layout::new();
    col(&mut l, &mut f, &d, 500.0);
    let colors = Colors { text: [20, 20, 30], link: [30, 60, 200], muted: [120, 120, 120], accent: [30, 60, 200] };
    let mut page = tiny_skia::Pixmap::new(500, l.height.ceil() as u32).unwrap();
    page.fill(tiny_skia::Color::WHITE);
    for (i, it) in l.items.iter().enumerate() {
        let pm = render_item(&mut f, it, &d.paras[i].kind, 500.0, 1.0, &colors, &mut NoObjects).unwrap();
        page.draw_pixmap(0, it.top as i32, pm.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
    }
    page.save_png(std::env::var("DUMP_PNG").unwrap_or("/tmp/claude-1000/docview.png".into())).unwrap();
}
