//! PDF import for annotating lecture slides: render pages (pure-Rust `hayro`, no native library
//! to bundle) and extract text for search.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::{RenderSettings, PixmapSettings};
use std::sync::Arc;

pub struct PdfDoc {
    pdf: Pdf,
    bytes: Vec<u8>,
}

pub struct PageImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA.
    pub rgba: Vec<u8>,
}

impl PdfDoc {
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let pdf = Pdf::new(Arc::new(bytes.clone())).map_err(|e| format!("not a readable PDF: {e:?}"))?;
        Ok(Self { pdf, bytes })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Render page `index` at `scale` (1.0 = 72 dpi).
    pub fn render_page(&self, index: usize, scale: f32) -> Option<PageImage> {
        let pages = hayro::render_pdf(&self.pdf, scale, InterpreterSettings::default(), &RenderSettings::default(), Some(index..=index))?;
        let pm = pages.into_iter().next()?;
        let _ = PixmapSettings::default;
        Some(PageImage { width: pm.width() as u32, height: pm.height() as u32, rgba: pm.data_as_u8_slice().to_vec() })
    }

    /// All text, for the search index ("PDF text").
    pub fn text(&self) -> String {
        pdf_extract::extract_text_from_mem(&self.bytes).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_renders_and_extracts_text_from_a_pdf() {
        let doc = render::compile("#set page(width: 200pt, height: 100pt)\nConvolution theorem slides\n\n#pagebreak()\nSecond slide", &[]).unwrap();
        let bytes = render::pdf(&doc).unwrap();
        let p = PdfDoc::open(bytes).unwrap();
        assert_eq!(p.page_count(), 2);
        let img = p.render_page(0, 2.0).unwrap();
        assert_eq!((img.width, img.height), (400, 200));
        assert!(img.rgba.chunks(4).any(|px| px[0] < 100), "text pixels were drawn");
        assert!(p.text().contains("Convolution"), "{}", p.text());
        assert!(PdfDoc::open(b"not a pdf".to_vec()).is_err());
    }
}
