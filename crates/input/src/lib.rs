//! Keyboard input: shorthand conversion, autocomplete, structure editing.

pub mod editor;
pub mod shorthand;

pub use editor::{clean, derivation, normalize, Editor};
pub use shorthand::{Out, Suggestion, Table, UserShorthand};
