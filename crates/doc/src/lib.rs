//! The document model behind a Scrawl page: paragraphs of styled runs plus objects (display math,
//! images, plots, rules, callouts). No UI code lives here, so everything is unit-tested.
//!
//! Typst stays the storage and export format: `Doc::to_typst` writes it, `Doc::from_markup` reads
//! the subset the old text blocks used. Anything the parser does not understand is kept verbatim as a
//! `ParaKind::Markup` paragraph, so converting old pages never loses content.

mod markup;
mod model;

pub use markup::{escape_text, para_markup};
pub use model::*;
