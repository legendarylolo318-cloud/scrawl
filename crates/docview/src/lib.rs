//! Layout, hit-testing and rendering of a `notedoc::Doc` (the page text), in logical pixels.
//!
//! Text is shaped and wrapped by `cosmic-text` with the fonts bundled in `typst-assets`, so the screen
//! and the Typst exports use the same faces. Math, images and plots are drawn by the app through the
//! `Objects` trait (the app owns the Typst renderer and the media files).

mod layout;
mod render;

pub use layout::{Fonts, Item, ItemKind, Layout, Look};
pub use notedoc::Pos;
pub use render::{render_item, Colors};

use std::sync::Arc;
use tiny_skia::Pixmap;

/// Something drawn by the app: inline math inside a line, or a whole-paragraph object.
pub trait Objects {
    /// Size in logical px of inline math.
    fn inline_size(&mut self, typst: &str) -> (f32, f32);
    fn inline_pixmap(&mut self, typst: &str, scale: f32, color: [u8; 3]) -> Option<Arc<Pixmap>>;
    /// Size in logical px of a display math / image / plot / markup paragraph in a column `col_w` wide.
    fn block_size(&mut self, kind: &notedoc::ParaKind, col_w: f32) -> (f32, f32);
    fn block_pixmap(&mut self, kind: &notedoc::ParaKind, col_w: f32, scale: f32, color: [u8; 3]) -> Option<Arc<Pixmap>>;
}

/// Fixed-size stand-in used by tests.
pub struct NoObjects;

impl Objects for NoObjects {
    fn inline_size(&mut self, typst: &str) -> (f32, f32) {
        (typst.chars().count() as f32 * 9.0 + 6.0, 22.0)
    }
    fn inline_pixmap(&mut self, _: &str, _: f32, _: [u8; 3]) -> Option<Arc<Pixmap>> {
        None
    }
    fn block_size(&mut self, _: &notedoc::ParaKind, _: f32) -> (f32, f32) {
        (100.0, 40.0)
    }
    fn block_pixmap(&mut self, _: &notedoc::ParaKind, _: f32, _: f32, _: [u8; 3]) -> Option<Arc<Pixmap>> {
        None
    }
}

#[cfg(test)]
mod tests;
