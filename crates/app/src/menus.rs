//! The top-bar menus (＋ Insert, Share, ⋯) as data. Every command still exists in the command palette;
//! the test at the bottom checks that each one is either in a menu, in the pen tray / formatting bar /
//! chip, or deliberately palette-only.

pub struct Item {
    pub id: &'static str,
    pub label: &'static str,
    /// Small grey hint on the right (a shortcut).
    pub hint: &'static str,
    /// Section title shown above the item when it differs from the previous one.
    pub group: &'static str,
}

macro_rules! items {
    ($($group:literal => [$($id:literal, $label:literal, $hint:literal;)*])*) => {
        &[$($(Item { id: $id, label: $label, hint: $hint, group: $group }),*),*]
    };
}

pub const INSERT: &[Item] = items! {
    "Text" => [
        "fmt-h1", "Heading 1", "# ";
        "fmt-h2", "Heading 2", "## ";
        "fmt-bullet", "Bullet list", "- ";
        "fmt-numbered", "Numbered list", "1. ";
        "fmt-check", "Checklist", "[] ";
        "fmt-quote", "Quote", "> ";
        "fmt-code", "Code", "";
        "insert-table", "Table", "";
        "callout", "Callout box", "";
        "rule", "Divider line", "";
        "fmt-link", "Link to a page or web address", "";
    ]
    "Math" => [
        "fmt-math", "Math inside text", "$x$";
        "symbols", "Symbols", "Ctrl+Shift+S";
        "mode-math", "Write math by hand", "Alt+M";
        "graph", "Graph and MATLAB", "Ctrl+G";
        "plot", "Function plot", "";
    ]
    "Media" => [
        "image", "Image from file", "";
        "paste-image", "Paste image", "Ctrl+Shift+V";
        "pdf", "PDF to annotate", "";
        "record", "Audio recording", "";
        "insert-axes", "Coordinate axes (ink)", "";
        "insert-ruler", "Ruler (ink)", "";
    ]
};

pub const SHARE: &[Item] = items! {
    "Share" => [
        "export", "Export this page…", "Ctrl+E";
        "export-notebook", "Export the whole notebook", "";
        "print", "Print", "Ctrl+Shift+O";
    ]
    "Copy" => [
        "copy-latex", "Copy math as LaTeX", "Ctrl+Shift+L";
        "copy-typst", "Copy math as Typst", "";
        "copy-image", "Copy math as an image", "";
    ]
};

pub const MORE: &[Item] = items! {
    "Share and export" => [
        "export", "Export this page…", "Ctrl+E";
        "export-notebook", "Export the whole notebook", "";
        "print", "Print", "Ctrl+Shift+O";
        "copy-latex", "Copy math as LaTeX", "Ctrl+Shift+L";
    ]
    "This page" => [
        "favorite", "Pin or unpin", "Ctrl+D";
        "outline", "Outline", "";
        "word-count", "Word count", "";
        "find", "Find and replace", "Ctrl+F";
        "duplicate", "Duplicate", "";
        "history", "Version history", "";
        "save-template", "Save as a template", "";
        "paper", "Next paper style", "";
        "page-mode", "Page size", "";
        "delete-page", "Move to trash", "";
    ]
    "Notes" => [
        "switcher", "Go to page…", "Ctrl+P";
        "quick-note", "Quick note", "Ctrl+Shift+N";
        "today", "Today's note", "";
        "trash", "Trash", "";
        "new-section", "New section", "";
        "new-notebook", "New notebook", "";
        "new-encrypted", "New encrypted notebook", "";
        "lock", "Lock this notebook", "";
    ]
    "View" => [
        "focus", "Focus mode", "F9";
        "present", "Presentation", "F5";
        "fullscreen", "Full screen", "F11";
        "split", "Open another page beside this one", "";
        "zoom-in", "Zoom in", "Ctrl+=";
        "zoom-out", "Zoom out", "Ctrl+-";
    ]
    "Tools" => [
        "tablet", "Use a tablet as a pen", "";
        "train", "Teach Scrawl my handwriting", "";
        "clear", "Clear all ink on this page", "";
        "snap-toggle", "Snap shapes on or off", "";
        "settings", "Settings", "Ctrl+,";
        "shortcuts", "Keyboard shortcuts", "Ctrl+/";
        "palette", "All commands", "Ctrl+K";
    ]
};

/// Commands that live in the pen tray, formatting bar, suggestion chip or top bar buttons.
#[cfg_attr(not(test), allow(dead_code))]
pub const ELSEWHERE: &[&str] = &[
    "pen", "highlighter", "eraser", "lasso", "pen1", "pen2", "pen3", "mode-text", "mode-drawing", "chip-toggle-words", "textbox", "undo", "redo", "delete-selection", "recognize", "insert", "chip-dismiss",
    "obj-smaller", "obj-larger", "obj-crop", "obj-uncrop", "obj-copy-latex", "obj-copy-typst", "obj-delete", "fmt-body", "fmt-h3", "fmt-bold", "fmt-italic", "fmt-underline", "fmt-indent", "fmt-outdent", "toggle-sidebar", "back", "forward", "new-page", "search",
];

/// Reachable from the command palette (Ctrl+K) and their own screens only, on purpose.
#[cfg_attr(not(test), allow(dead_code))]
pub const PALETTE_ONLY: &[&str] = &[
    "typing", "copy-unicode", "tmpl-frac", "tmpl-sqrt", "tmpl-int", "tmpl-sum", "tmpl-lim", "tmpl-matrix", "tmpl-cases", "tmpl-deriv", "theme-dark", "theme-light", "theme-contrast", "text-block", "toggle-doc", "next-tab", "close-tab", "play", "zoom-reset", "backup", "restore-backup", "split-close", "save-graph",
];

pub fn menu(name: &str) -> &'static [Item] {
    match name {
        "insert" => INSERT,
        "share" => SHARE,
        _ => MORE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;

    #[test]
    fn every_command_is_reachable_without_the_palette_or_is_palette_only_on_purpose() {
        let mut known: Vec<&str> = Vec::new();
        for m in [INSERT, SHARE, MORE] {
            known.extend(m.iter().map(|i| i.id));
        }
        known.extend(ELSEWHERE);
        known.extend(PALETTE_ONLY);
        for c in commands::ALL {
            assert!(known.contains(&c.id), "command {:?} ({}) has no home", c.id, c.title);
        }
        for id in &known {
            assert!(commands::get(id).is_some(), "menu entry {id:?} is not a command");
        }
    }

    #[test]
    fn menu_labels_are_short_and_plain() {
        for m in [INSERT, SHARE, MORE] {
            for i in m {
                assert!(i.label.chars().count() <= 44, "{}", i.label);
            }
        }
    }
}
