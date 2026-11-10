//! Spatial layout of recognized symbols into an expression tree, plus Typst/LaTeX output.

pub mod expr;
pub mod latex;
pub mod parse;
pub mod typst;
pub mod unicode;

pub use expr::*;
pub use parse::{parse, Glyph};

impl Expr {
    pub fn to_typst(&self) -> String {
        typst::ser(self)
    }
    pub fn to_latex(&self) -> String {
        latex::ser(self)
    }
    pub fn to_unicode(&self) -> String {
        unicode::ser(self)
    }
}
