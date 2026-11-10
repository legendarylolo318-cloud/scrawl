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
    ink_layer(strokes, &[], true).map(|l| l.svg)
}

/// Ink (and typed canvas text) as an SVG plus the page-space box it covers.
pub struct InkLayer {
    pub svg: String,
    pub x0: f32,
    pub y0: f32,
    pub w: f32,
    pub h: f32,
}

fn dark_on_paper([r, g, b, a]: [u8; 4]) -> (u8, u8, u8, u8) {
    // dark ink on white paper for export: invert the near-white default pen colour
    if r > 200 && g > 200 && b > 200 {
        (0x1a, 0x1b, 0x26, a)
    } else {
        (r, g, b, a)
    }
}

/// `embed_text`: put canvas text into the SVG as `<text>` (browsers); otherwise the caller places
/// it (Typst, whose SVG import does not use our fonts for text).
pub fn ink_layer(strokes: &[Stroke], texts: &[store::CanvasText], embed_text: bool) -> Option<InkLayer> {
    let mut pts: Vec<(f32, f32)> = strokes.iter().flat_map(|s| s.points.iter().map(|p| (p.x, p.y))).collect();
    for t in texts {
        let (a, b, c, d) = t.bbox();
        pts.push((a, b));
        pts.push((c, d));
    }
    if pts.is_empty() {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
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
        let d: String = st.points.iter().enumerate().map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { "L" }, p.x - x0 + pad, p.y - y0 + pad)).collect();
        let (r, g, b, a) = dark_on_paper(st.color);
        let width = if st.kind == ink::PenKind::Highlighter { st.width * 4.0 } else { st.width };
        s += &format!("<path d=\"{d}\" fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-opacity=\"{:.2}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>", a as f32 / 255.0);
    }
    if embed_text {
        for t in texts {
            let (r, g, b, _) = dark_on_paper(t.color);
            for (i, line) in t.text.lines().enumerate() {
                s += &format!(
                    "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{:.1}\" font-family=\"sans-serif\" fill=\"rgb({r},{g},{b})\">{}</text>",
                    t.x - x0 + pad,
                    t.y - y0 + pad + t.size * (1.0 + i as f32 * 1.3),
                    t.size,
                    esc_html(line)
                );
            }
        }
    }
    s += "</svg>";
    Some(InkLayer { svg: s, x0: x0 - pad, y0: y0 - pad, w, h })
}

/// Files referenced by pages (images, PDF page renders) keyed by their media name.
pub type Media = Vec<(String, Vec<u8>)>;

/// Render a plot block to SVG (light theme, for paper).
pub fn plot_svg(b: &Block) -> Option<String> {
    let Block::Plot { functions, x_range, y_range } = b else { return None };
    let plot = media::plot::Plot {
        curves: functions.iter().map(|f| media::plot::Curve::Cartesian(f.split_once('=').map_or(f.as_str(), |(_, r)| r).trim().to_string())).collect(),
        x_range: (x_range.0 as f64, x_range.1 as f64),
        y_range: y_range.map(|(a, b)| (a as f64, b as f64)),
        x_label: "x".into(),
        y_label: "y".into(),
    };
    media::plot::render_svg(&plot, 480.0, 300.0, false).ok()
}

/// Typst source for the pages plus every file it references. `media` supplies image bytes;
/// `backgrounds[i]` is an optional pre-composited background+ink PNG for page i (PDF annotation).
fn typst_doc(pages: &[&Page], media: &Media, backgrounds: &[Option<Vec<u8>>], one_tall_page: bool) -> (String, Vec<(String, Vec<u8>)>) {
    let mut src = pages.first().map(|p| p.paper_typst()).unwrap_or("#set page(paper: \"a4\", margin: 18mm)\n").to_string();
    if one_tall_page {
        src += "#set page(height: auto)\n";
    }
    src += "#set text(size: 11pt)\n";
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            src += "#pagebreak()\n";
        }
        src += &p.header_typst();
        for (j, b) in p.blocks.iter().enumerate() {
            src += "\n";
            match b {
                Block::Image { file, .. } => match media.iter().find(|(n, _)| n == file) {
                    Some((n, bytes)) => {
                        if !files.iter().any(|(f, _)| f == n) {
                            files.push((n.clone(), bytes.clone()));
                        }
                        src += &store::block_typst(b, None);
                    }
                    None => src += &format!("_[missing image {}]_\n", store::typst_escape(file)),
                },
                Block::Plot { .. } => {
                    let name = format!("plot{i}-{j}.svg");
                    match plot_svg(b) {
                        Some(svg) => {
                            files.push((name.clone(), svg.into_bytes()));
                            src += &store::block_typst(b, Some(&name));
                        }
                        None => src += &store::block_typst(b, None),
                    }
                }
                _ => src += &store::block_typst(b, None),
            }
        }
        match backgrounds.get(i).and_then(|b| b.as_ref()) {
            Some(png) => {
                let name = format!("page{i}.png");
                src += &format!("\n#image(\"{name}\", width: 100%)\n");
                files.push((name, png.clone()));
            }
            None => {
                if let Some(layer) = ink_layer(&p.strokes, &p.texts, false) {
                    let name = format!("ink{i}.svg");
                    // the ink image spans the text width; typed text is placed on top with Typst
                    src += &format!("\n#block(width: 100%)[#image(\"{name}\", width: 100%)");
                    let text_w = if p.mode == store::PageMode::Letter { 510.0 } else { 493.0 };
                    let k = text_w / layer.w.max(1.0);
                    for t in &p.texts {
                        let (r, g, b, _) = dark_on_paper(t.color);
                        let body: Vec<String> = t.text.lines().map(store::typst_escape).collect();
                        src += &format!(
                            "#place(top + left, dx: {:.2}%, dy: {:.2}%)[#text(size: {:.1}pt, fill: rgb({r}, {g}, {b}))[{}]]",
                            (t.x - layer.x0) / layer.w * 100.0,
                            (t.y - layer.y0) / layer.h * 100.0,
                            t.size * k,
                            body.join(" \\ ")
                        );
                    }
                    src += "]\n";
                    files.push((name, layer.svg.into_bytes()));
                }
            }
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

/// Escape LaTeX special characters in text; `$...$` inline math is passed through.
pub fn latex_escape(s: &str) -> String {
    let mut out = String::new();
    for (k, part) in s.split('$').enumerate() {
        if k > 0 {
            out.push('$');
        }
        if k % 2 == 1 && s.matches('$').count().is_multiple_of(2) {
            out += part; // inside inline math
            continue;
        }
        for c in part.chars() {
            match c {
                '\\' => out += "\\textbackslash{}",
                '{' | '}' | '#' | '%' | '&' | '_' => {
                    out.push('\\');
                    out.push(c);
                }
                '^' => out += "\\textasciicircum{}",
                '~' => out += "\\textasciitilde{}",
                '*' => {}
                _ => out.push(c),
            }
        }
    }
    if s.matches('$').count() % 2 == 1 {
        out = out.replace('$', "\\$");
    }
    out
}

fn latex_markup(markup: &str, s: &mut String) {
    let mut list: Option<&str> = None;
    for line in markup.lines() {
        let t = line.trim_start();
        let item = t.strip_prefix("- ").map(|r| ("itemize", r)).or_else(|| t.strip_prefix("+ ").map(|r| ("enumerate", r)));
        if list.is_some() && item.map(|i| i.0) != list {
            *s += &format!("\\end{{{}}}\n", list.take().unwrap_or("itemize"));
        }
        if let Some((env, rest)) = item {
            if list.is_none() {
                *s += &format!("\\begin{{{env}}}\n");
                list = Some(env);
            }
            *s += &format!("\\item {}\n", latex_escape(rest));
            continue;
        }
        let h = t.chars().take_while(|c| *c == '=').count();
        if h > 0 {
            *s += &format!("\\{}section*{{{}}}\n", if h >= 3 { "subsub" } else { "sub" }, latex_escape(t[h..].trim()));
        } else if t.starts_with("```") {
            *s += "% code fence\n";
        } else {
            *s += &format!("{}\n", latex_escape(t));
        }
    }
    if let Some(env) = list {
        *s += &format!("\\end{{{env}}}\n");
    }
}

pub fn to_latex_doc(pages: &[&Page]) -> String {
    let mut s = String::from("\\documentclass{article}\n\\usepackage{amsmath,amssymb,amsfonts}\n\\usepackage[utf8]{inputenc}\n\\usepackage{graphicx}\n\\begin{document}\n");
    for p in pages {
        s += &format!("\\section*{{{}}}\n", latex_escape(&p.title));
        for b in &p.blocks {
            match b {
                Block::Math { expr, source } => s += &format!("\\[\n{}\n\\]\n", if expr.is_empty() { source.clone().unwrap_or_default() } else { expr.to_latex() }),
                Block::Text { markup } => latex_markup(markup, &mut s),
                Block::Callout { kind, markup } => {
                    s += &format!("\\paragraph{{{kind:?}.}}\n");
                    latex_markup(markup, &mut s);
                }
                Block::Rule => s += "\\noindent\\rule{\\linewidth}{0.4pt}\n",
                Block::Image { file, .. } => s += &format!("\\includegraphics[width=0.8\\linewidth]{{{}}}\n", latex_escape(file)),
                Block::Plot { functions, .. } => s += &format!("\\emph{{Plot: {}}}\n", latex_escape(&functions.join(", "))),
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
    to_html_with(pages, &Vec::new())
}

fn data_uri(name: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    let mime = match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Self-contained HTML; images are embedded as data URIs.
pub fn to_html_with(pages: &[&Page], media: &Media) -> Result<String, String> {
    let mut s = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><title>Scrawl export</title><style>body{font-family:sans-serif;max-width:48em;margin:2em auto;padding:0 1em}.callout{background:#eef;padding:.6em;border-radius:6px}</style></head><body>\n");
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
                Block::Image { file, w, .. } => match media.iter().find(|(n, _)| n == file) {
                    Some((n, b)) => s += &format!("<img src=\"{}\" width=\"{w}\" alt=\"\">\n", data_uri(n, b)),
                    None => s += &format!("<p><i>[missing image {}]</i></p>\n", esc_html(file)),
                },
                Block::Plot { .. } => s += &format!("<div class=\"plot\">{}</div>\n", plot_svg(b).unwrap_or_default()),
            }
        }
        if let Some(svg) = ink_layer(&p.strokes, &p.texts, true).map(|l| l.svg) {
            s += &format!("<div class=\"ink\">{svg}</div>\n");
        }
    }
    s += "</body></html>\n";
    Ok(s)
}

pub fn export_pages(pages: &[&Page], fmt: Format) -> Result<Vec<u8>, String> {
    export_pages_with(pages, fmt, &Vec::new(), &[])
}

/// Export with the pages' media (images) and optional per-page background composites.
pub fn export_pages_with(pages: &[&Page], fmt: Format, media: &Media, backgrounds: &[Option<Vec<u8>>]) -> Result<Vec<u8>, String> {
    match fmt {
        Format::Markdown => Ok(pages.iter().map(|p| to_markdown(p)).collect::<Vec<_>>().join("\n\n---\n\n").into_bytes()),
        Format::Typst => {
            let (src, _) = typst_doc(pages, media, &[], false);
            Ok(src.into_bytes())
        }
        Format::Latex => Ok(to_latex_doc(pages).into_bytes()),
        Format::Html => to_html_with(pages, media).map(String::into_bytes),
        Format::Pdf | Format::Png => {
            let (src, files) = typst_doc(pages, media, backgrounds, fmt == Format::Png);
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
    let path = std::env::temp_dir().join(format!("scrawl-print-{}.pdf", store::new_id()));
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

#[cfg(test)]
mod media_tests {
    use super::*;

    fn png_1x1() -> Vec<u8> {
        let doc = render::compile("#set page(width: 10pt, height: 10pt, margin: 0pt)\n#rect(fill: red)", &[]).unwrap();
        render::png(&doc, 0, 1.0).unwrap()
    }

    #[test]
    fn images_plots_and_hostile_titles_export() {
        let mut p = Page::new("p", "Costs: 50% & $5 #1 _x_ {y}", "2026-11-10");
        p.blocks = vec![
            Block::Image { file: "a.png".into(), x: 0.0, y: 0.0, w: 120.0, h: 80.0, crop: Some([0.1, 0.1, 0.1, 0.1]) },
            Block::Plot { functions: vec!["y = sin(x)".into()], x_range: (-3.0, 3.0), y_range: None },
            Block::Text { markup: "- one\n- two\n+ first\nprice 5% & $x^2$".into() },
        ];
        let media = vec![("a.png".to_string(), png_1x1())];
        let pdf = export_pages_with(&[&p], Format::Pdf, &media, &[]).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let png = export_pages_with(&[&p], Format::Png, &media, &[]).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let html = String::from_utf8(export_pages_with(&[&p], Format::Html, &media, &[]).unwrap()).unwrap();
        assert!(html.contains("data:image/png;base64,") && html.contains("class=\"plot\"><svg"));
        let tex = to_latex_doc(&[&p]);
        assert!(tex.contains("Costs: 50\\% \\& \\$5 \\#1 \\_x\\_ \\{y\\}"), "{tex}");
        assert!(tex.contains("\\begin{itemize}\n\\item one\n\\item two\n\\end{itemize}\n\\begin{enumerate}"), "{tex}");
        assert!(tex.contains("5\\% \\& $x^2$"), "{tex}");
        // a missing image is reported, not a compile failure
        assert!(export_pages_with(&[&p], Format::Pdf, &Vec::new(), &[]).is_ok());
    }
}

#[cfg(test)]
mod canvas_text_tests {
    use super::*;

    #[test]
    fn typed_canvas_text_is_exported_everywhere() {
        let mut p = Page::new("p", "Notes", "2026-11-10");
        p.texts = vec![
            store::CanvasText { id: 1, x: 10.0, y: 20.0, text: "Remember: brandy & <tags> $5".into(), size: 18.0, color: [255; 4] },
            store::CanvasText { id: 2, x: 10.0, y: 80.0, text: "two\nlines".into(), size: 14.0, color: [0xf7, 0x76, 0x8e, 255] },
        ];
        let html = String::from_utf8(export_page(&p, Format::Html).unwrap()).unwrap();
        assert!(html.contains("Remember: brandy &amp; &lt;tags&gt; $5"), "{html}");
        let pdf = export_page(&p, Format::Pdf).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let typ = typst_doc(&[&p], &Vec::new(), &[], false).0;
        assert!(typ.contains("#place(top + left") && typ.contains("two \\ lines"), "{typ}");
        assert_eq!(&export_page(&p, Format::Png).unwrap()[1..4], b"PNG");
    }
}
