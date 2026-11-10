//! Export pages/sections/notebooks to PDF, PNG, Markdown, Typst, LaTeX, HTML; copy-as helpers.

use ink::Stroke;
use layout::Expr;
use store::{Block, CalloutKind, Page};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Png,
    Markdown,
    Typst,
    Latex,
    Html,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Markdown => "md",
            Self::Typst => "typ",
            Self::Latex => "tex",
            Self::Html => "html",
        }
    }
}

/// Page ink as a standalone SVG (so ink survives in PDF/PNG/HTML export).
pub fn ink_svg(strokes: &[Stroke]) -> Option<String> {
    let pts: Vec<&ink::Point> = strokes.iter().flat_map(|s| s.points.iter()).collect();
    if pts.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let pad = 8.0;
    let (w, h) = (x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad);
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.1}\" height=\"{h:.1}\" viewBox=\"0 0 {w:.1} {h:.1}\">");
    let mut ordered: Vec<&Stroke> = strokes.iter().collect();
    ordered.sort_by_key(|s| s.kind != ink::PenKind::Highlighter); // highlighters under ink
    for st in ordered {
        if st.points.is_empty() {
            continue;
        }
        let d: String = st
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { "L" }, p.x - x0 + pad, p.y - y0 + pad))
            .collect();
        let [r, g, b, a] = st.color;
        let width = if st.kind == ink::PenKind::Highlighter { st.width * 4.0 } else { st.width };
        // Dark ink on white paper for export: invert near-white default pen colour.
        let (r, g, b) = if r > 200 && g > 200 && b > 200 { (0x1a, 0x1b, 0x26) } else { (r, g, b) };
        s += &format!("<path d=\"{d}\" fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-opacity=\"{:.2}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>", a as f32 / 255.0);
    }
    s += "</svg>";
    Some(s)
}

fn typst_doc(pages: &[&Page]) -> (String, Vec<(String, Vec<u8>)>) {
    let mut src = String::from("#set page(paper: \"a4\", margin: 18mm)\n#set text(size: 11pt)\n");
    let mut files = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            src += "#pagebreak()\n";
        }
        src += &p.to_typst();
        if let Some(svg) = ink_svg(&p.strokes) {
            let name = format!("ink{i}.svg");
            src += &format!("\n#image(\"{name}\", width: 100%)\n");
            files.push((name, svg.into_bytes()));
        }
    }
    (src, files)
}

fn md_inline(markup: &str) -> String {
    // Typst markup -> Markdown for the subset the rich-text editor produces.
    let mut out = Vec::new();
    for line in markup.lines() {
        let t = line.trim_start();
        let hashes = t.chars().take_while(|c| *c == '=').count();
        let line = if hashes > 0 && t[hashes..].starts_with(' ') {
            format!("{} {}", "#".repeat(hashes), &t[hashes + 1..])
        } else if let Some(rest) = t.strip_prefix("+ ") {
            format!("1. {rest}")
        } else if let Some(rest) = t.strip_prefix("- ") {
            format!("- {rest}")
        } else {
            t.to_string()
        };
        // *bold* -> **bold**, _italic_ -> *italic*
        let mut s = String::new();
        let mut bold = false;
        for c in line.chars() {
            match c {
                '*' => {
                    s += "**";
                    bold = !bold;
                }
                _ => s.push(c),
            }
        }
        let _ = bold;
        out.push(s);
    }
    out.join("\n")
}

pub fn to_markdown(p: &Page) -> String {
    let mut s = format!("# {}\n", p.title);
    if !p.course.is_empty() || p.lecture.is_some() {
        s += &format!("*{}{} · {}*\n", p.course, p.lecture.map(|l| format!(" · lecture {l}")).unwrap_or_default(), p.date);
    }
    for b in &p.blocks {
        s += "\n";
        match b {
            Block::Math { expr, source } => s += &format!("$$\n{}\n$$\n", if expr.is_empty() { source.clone().unwrap_or_default() } else { expr.to_latex() }),
            Block::Text { markup } => s += &format!("{}\n", md_inline(markup)),
            Block::Callout { kind, markup } => s += &format!("> **{kind:?}.** {}\n", md_inline(markup)),
            Block::Image { file, .. } => s += &format!("![]({file})\n"),
            Block::Plot { functions, .. } => s += &format!("*Plot: {}*\n", functions.join(", ")),
            Block::Rule => s += "---\n",
        }
    }
    s
}

pub fn to_latex_doc(pages: &[&Page]) -> String {
    let mut s = String::from("\\documentclass{article}\n\\usepackage{amsmath,amssymb,amsfonts}\n\\usepackage[utf8]{inputenc}\n\\begin{document}\n");
    for p in pages {
        s += &format!("\\section*{{{}}}\n", p.title.replace('_', "\\_").replace('&', "\\&"));
        for b in &p.blocks {
            match b {
                Block::Math { expr, source } => s += &format!("\\[\n{}\n\\]\n", if expr.is_empty() { source.clone().unwrap_or_default() } else { expr.to_latex() }),
                Block::Text { markup } | Block::Callout { markup, .. } => {
                    for line in markup.lines() {
                        let t = line.trim_start();
                        let h = t.chars().take_while(|c| *c == '=').count();
                        if h > 0 {
                            s += &format!("\\subsection*{{{}}}\n", t[h..].trim());
                        } else {
                            s += &format!("{}\n", t.replace('#', "\\#").replace('_', "\\_"));
                        }
                    }
                }
                Block::Rule => s += "\\hrule\n",
                _ => {}
            }
        }
    }
    s += "\\end{document}\n";
    s
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Self-contained HTML: math is embedded as SVG so it renders offline with no scripts.
pub fn to_html(pages: &[&Page]) -> Result<String, String> {
    let mut s = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><title>Inkwell export</title><style>body{font-family:sans-serif;max-width:48em;margin:2em auto;padding:0 1em}.callout{background:#eef;padding:.6em;border-radius:6px}</style></head><body>\n");
    for p in pages {
        s += &format!("<h1>{}</h1>\n", esc_html(&p.title));
        for b in &p.blocks {
            match b {
                Block::Math { expr, source } => {
                    let t = if expr.is_empty() { source.clone().unwrap_or_default() } else { expr.to_typst() };
                    let doc = render::compile(&render::math_source(&t, 14.0, [0x1a, 0x1b, 0x26]), &[]).map_err(|e| format!("math render: {e}"))?;
                    s += &format!("<div class=\"math\">{}</div>\n", render::svg(&doc, 0).unwrap_or_default());
                }
                Block::Text { markup } => {
                    for line in markup.lines() {
                        let t = line.trim_start();
                        let h = t.chars().take_while(|c| *c == '=').count();
                        if (1..=3).contains(&h) {
                            s += &format!("<h{0}>{1}</h{0}>\n", h + 1, esc_html(t[h..].trim()));
                        } else {
                            s += &format!("<p>{}</p>\n", esc_html(t));
                        }
                    }
                }
                Block::Callout { kind, markup } => s += &format!("<div class=\"callout\"><b>{}.</b> {}</div>\n", match kind { CalloutKind::Definition => "Definition", CalloutKind::Theorem => "Theorem", CalloutKind::Example => "Example", CalloutKind::Warning => "Warning" }, esc_html(markup)),
                Block::Rule => s += "<hr>\n",
                Block::Image { file, .. } => s += &format!("<img src=\"{}\" alt=\"\">\n", esc_html(file)),
                Block::Plot { functions, .. } => s += &format!("<p><i>Plot: {}</i></p>\n", esc_html(&functions.join(", "))),
            }
        }
        if let Some(svg) = ink_svg(&p.strokes) {
            s += &format!("<div class=\"ink\">{svg}</div>\n");
        }
    }
    s += "</body></html>\n";
    Ok(s)
}

pub fn export_pages(pages: &[&Page], fmt: Format) -> Result<Vec<u8>, String> {
    match fmt {
        Format::Markdown => Ok(pages.iter().map(|p| to_markdown(p)).collect::<Vec<_>>().join("\n\n---\n\n").into_bytes()),
        Format::Typst => Ok(pages.iter().map(|p| p.to_typst()).collect::<Vec<_>>().join("\n#pagebreak()\n").into_bytes()),
        Format::Latex => Ok(to_latex_doc(pages).into_bytes()),
        Format::Html => to_html(pages).map(String::into_bytes),
        Format::Pdf | Format::Png => {
            let (src, files) = typst_doc(pages);
            let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
            let doc = render::compile(&src, &refs)?;
            if fmt == Format::Pdf {
                render::pdf(&doc)
            } else {
                render::png(&doc, 0, 2.0).ok_or_else(|| "empty document".into())
            }
        }
    }
}

pub fn export_page(page: &Page, fmt: Format) -> Result<Vec<u8>, String> {
    export_pages(&[page], fmt)
}

/// Export several pages (a section or notebook) as a zip with one file per page.
pub fn export_zip(pages: &[&Page], fmt: Format) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        for (i, p) in pages.iter().enumerate() {
            let safe: String = p.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' }).collect();
            z.start_file(format!("{:02}-{}.{}", i + 1, safe.trim(), fmt.extension()), opts).map_err(|e| e.to_string())?;
            z.write_all(&export_page(p, fmt)?).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyAs {
    Latex,
    Typst,
    Unicode,
    Image,
}

/// Copy a math block as LaTeX, Typst, Unicode text, or PNG bytes.
pub fn copy_block(expr: &Expr, how: CopyAs) -> Result<Vec<u8>, String> {
    match how {
        CopyAs::Latex => Ok(expr.to_latex().into_bytes()),
        CopyAs::Typst => Ok(expr.to_typst().into_bytes()),
        CopyAs::Unicode => Ok(expr.to_unicode().into_bytes()),
        CopyAs::Image => {
            let doc = render::compile(&render::math_source(&expr.to_typst(), 16.0, [0, 0, 0]), &[])?;
            render::png(&doc, 0, 3.0).ok_or_else(|| "empty".into())
        }
    }
}

/// Open the exported PDF in the OS viewer, which provides the native print dialog.
pub fn print_pdf(pdf: &[u8]) -> std::io::Result<()> {
    let path = std::env::temp_dir().join(format!("inkwell-print-{}.pdf", store::new_id()));
    std::fs::write(&path, pdf)?;
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd").args(["/C", "start", "", &path.to_string_lossy()]).status()?;
    #[cfg(not(target_os = "windows"))]
    let status = std::process::Command::new("xdg-open").arg(&path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("could not open a PDF viewer"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::{InkMode, PenKind, Point};
    use store::CalloutKind;

    fn sample() -> Page {
        let mut p = Page::new("p", "Chain rule", "2026-11-10");
        p.course = "Calc 1".into();
        p.blocks = vec![
            Block::Text { markup: "== Statement\nThe *derivative* of a composite.".into() },
            Block::Math { expr: Expr::Frac(Box::new(Expr::sym("d")), Box::new(Expr::Row(vec![Expr::sym("d"), Expr::sym("x")]))), source: None },
            Block::Callout { kind: CalloutKind::Theorem, markup: "Chain rule holds.".into() },
            Block::Rule,
        ];
        p.strokes = vec![Stroke {
            id: 1,
            points: (0..10).map(|i| Point::new(i as f32 * 5.0, (i * i) as f32, i as u32)).collect(),
            color: [255; 4],
            width: 3.0,
            kind: PenKind::Pen,
            mode: InkMode::Drawing,
            t0_ms: 0,
        }];
        p
    }

    #[test]
    fn markdown_has_latex_math_and_headings() {
        let m = to_markdown(&sample());
        assert!(m.starts_with("# Chain rule"));
        assert!(m.contains("## Statement"));
        assert!(m.contains("**derivative**"));
        assert!(m.contains("$$\n\\frac{d}{d x}\n$$"), "{m}");
    }

    #[test]
    fn latex_and_typst_exports() {
        let l = String::from_utf8(export_page(&sample(), Format::Latex).unwrap()).unwrap();
        assert!(l.contains("\\begin{document}") && l.contains("\\frac{d}{d x}"));
        let t = String::from_utf8(export_page(&sample(), Format::Typst).unwrap()).unwrap();
        assert!(t.contains("frac(d, d x)"));
    }

    #[test]
    fn html_is_self_contained_with_svg_math_and_ink() {
        let h = String::from_utf8(export_page(&sample(), Format::Html).unwrap()).unwrap();
        assert!(h.contains("<svg") && h.contains("class=\"ink\""));
        assert!(!h.contains("http://cdn") && !h.contains("<script"), "no third-party or script");
    }

    #[test]
    fn pdf_png_and_zip_export_work_including_ink() {
        let pdf = export_page(&sample(), Format::Pdf).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let png = export_page(&sample(), Format::Png).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let z = export_zip(&[&sample(), &sample()], Format::Markdown).unwrap();
        let a = zip::ZipArchive::new(std::io::Cursor::new(z)).unwrap();
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn copy_block_variants() {
        let e = Expr::Sup(Box::new(Expr::sym("x")), Box::new(Expr::num("2")));
        assert_eq!(copy_block(&e, CopyAs::Latex).unwrap(), b"x^{2}");
        assert_eq!(copy_block(&e, CopyAs::Typst).unwrap(), b"x^2");
        assert_eq!(copy_block(&e, CopyAs::Unicode).unwrap(), "x²".as_bytes());
        assert_eq!(&copy_block(&e, CopyAs::Image).unwrap()[1..4], b"PNG");
    }
}
