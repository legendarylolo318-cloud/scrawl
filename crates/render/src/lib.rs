//! Typst rendering: compile Typst source (math and rich text) to PNG / SVG / PDF with the
//! embedded `typst` crate. Fonts are bundled (typst-assets), so no system fonts are needed.
//! Renders of math blocks are cached by content so only changed blocks are re-rendered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst_layout::PagedDocument;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

struct Shared {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| {
        let fonts: Vec<Font> = typst_assets::fonts().flat_map(|d| Font::iter(Bytes::new(d))).collect();
        let mut book = FontBook::new();
        for f in &fonts {
            book.push(f.info().clone());
        }
        Shared { library: LazyHash::new(Library::default()), book: LazyHash::new(book), fonts }
    })
}

struct MemWorld {
    main: Source,
    files: HashMap<FileId, Bytes>,
}

impl World for MemWorld {
    fn library(&self) -> &LazyHash<Library> {
        &shared().library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &shared().book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotFound(id.vpath().get_without_slash().into()))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or_else(|| FileError::NotFound(id.vpath().get_without_slash().into()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        shared().fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<typst::foundations::Duration>) -> Option<Datetime> {
        None
    }
}

fn fid(name: &str) -> FileId {
    RootedPath::new(VirtualRoot::Project, VirtualPath::new(name).expect("valid virtual path")).intern()
}

/// Compile Typst `source`. `files` are extra in-memory files (e.g. images) addressed by name.
pub fn compile(source: &str, files: &[(&str, Vec<u8>)]) -> Result<PagedDocument, String> {
    let main = Source::new(fid("main.typ"), source.to_string());
    let files = files.iter().map(|(n, b)| (fid(n), Bytes::new(b.clone()))).collect();
    let world = MemWorld { main, files };
    let out = typst::compile::<PagedDocument>(&world);
    out.output.map_err(|errs| errs.iter().map(|e| e.message.to_string()).collect::<Vec<_>>().join("; "))
}

pub fn png(doc: &PagedDocument, page: usize, pixels_per_pt: f32) -> Option<Vec<u8>> {
    let p = doc.pages().get(page)?;
    let opts = typst_render::RenderOptions { pixel_per_pt: f64::from(pixels_per_pt).into(), ..Default::default() };
    typst_render::render(p, &opts).encode_png().ok()
}

pub fn svg(doc: &PagedDocument, page: usize) -> Option<String> {
    Some(typst_svg::svg(doc.pages().get(page)?, &Default::default()))
}

pub fn pdf(doc: &PagedDocument) -> Result<Vec<u8>, String> {
    typst_pdf::pdf(doc, &typst_pdf::PdfOptions::default()).map_err(|e| e.iter().map(|d| d.message.to_string()).collect::<Vec<_>>().join("; "))
}

/// A rendered snippet as premultiplied RGBA pixels, ready for a Slint image.
#[derive(Clone)]
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub rgba_premul: Vec<u8>,
}

/// Wrap a math expression in an auto-sized transparent page.
pub fn math_source(typst_math: &str, size_pt: f32, color: [u8; 3]) -> String {
    format!(
        "#set page(width: auto, height: auto, margin: 3pt, fill: none)\n#set text(size: {size_pt}pt, fill: rgb({}, {}, {}))\n$ {typst_math} $\n",
        color[0], color[1], color[2]
    )
}

/// Math renders are cached by (source, size, colour, scale): unchanged blocks cost nothing.
#[derive(Default)]
pub struct MathCache {
    map: Mutex<HashMap<String, Arc<Rendered>>>,
    pub misses: std::sync::atomic::AtomicUsize,
}

impl MathCache {
    pub fn render(&self, typst_math: &str, size_pt: f32, color: [u8; 3], scale: f32) -> Result<Arc<Rendered>, String> {
        let key = format!("{typst_math}\u{0}{size_pt}\u{0}{color:?}\u{0}{scale}");
        if let Some(r) = self.map.lock().unwrap().get(&key) {
            return Ok(r.clone());
        }
        self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let doc = compile(&math_source(typst_math, size_pt, color), &[])?;
        let page = doc.pages().first().ok_or("empty document")?;
        let opts = typst_render::RenderOptions { pixel_per_pt: f64::from(scale).into(), ..Default::default() };
        let pm = typst_render::render(page, &opts);
        let r = Arc::new(Rendered { width: pm.width(), height: pm.height(), rgba_premul: pm.data().to_vec() });
        let mut m = self.map.lock().unwrap();
        if m.len() > 512 {
            m.clear();
        }
        m.insert(key, r.clone());
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_math_to_png_pdf_svg() {
        let doc = compile(&math_source("integral_0^1 x^2 d x = 1/3", 14.0, [0, 0, 0]), &[]).unwrap();
        let png_bytes = png(&doc, 0, 2.0).unwrap();
        assert_eq!(&png_bytes[1..4], b"PNG");
        assert!(svg(&doc, 0).unwrap().contains("<svg"));
        assert!(pdf(&doc).unwrap().starts_with(b"%PDF"));
    }

    #[test]
    fn typst_errors_are_reported_not_panicked() {
        assert!(compile("$ frac( $", &[]).is_err());
    }

    #[test]
    fn math_cache_only_rerenders_changed_blocks() {
        let c = MathCache::default();
        let a = c.render("x^2", 14.0, [255, 255, 255], 1.0).unwrap();
        let b = c.render("x^2", 14.0, [255, 255, 255], 1.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        c.render("x^3", 14.0, [255, 255, 255], 1.0).unwrap();
        assert_eq!(c.misses.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert!(a.width > 4 && a.height > 4);
        assert!(a.rgba_premul.chunks(4).any(|p| p[3] > 0), "something was drawn");
    }

    #[test]
    fn every_serializer_output_compiles() {
        use layout::Expr;
        let e = Expr::Big {
            op: layout::BigOp::Sum,
            lower: Some(Box::new(Expr::Row(vec![Expr::sym("i"), Expr::sym("="), Expr::num("0")]))),
            upper: Some(Box::new(Expr::sym("∞"))),
            body: Box::new(Expr::Frac(Box::new(Expr::num("1")), Box::new(Expr::Sup(Box::new(Expr::sym("i")), Box::new(Expr::num("2")))))),
        };
        compile(&math_source(&e.to_typst(), 12.0, [0; 3]), &[]).unwrap_or_else(|err| panic!("{}: {err}", e.to_typst()));
        let m = Expr::Matrix { kind: layout::MatrixKind::Det, rows: vec![vec![Expr::num("1"), Expr::sym("α")], vec![Expr::num("3"), Expr::Func("rank".into())]], augment: Some(1) };
        compile(&math_source(&m.to_typst(), 12.0, [0; 3]), &[]).unwrap_or_else(|err| panic!("{}: {err}", m.to_typst()));
    }
}
