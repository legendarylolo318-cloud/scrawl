//! Every action the app offers, in one table: the command palette lists them, shortcuts map to
//! them (rebindable in Settings), and the shortcut reference screen is generated from it.

pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    /// Default key combo(s), `|`-separated; "" = none. Format: modifiers in the order
    /// ctrl, alt, shift, meta, then the key in lower case: "ctrl+shift+p", "f11", "delete".
    pub key: &'static str,
    pub group: &'static str,
}

macro_rules! cmds {
    ($($id:literal, $title:literal, $key:literal, $group:literal;)*) => {
        pub const ALL: &[Command] = &[$(Command { id: $id, title: $title, key: $key, group: $group }),*];
    };
}

cmds! {
    "palette", "Command palette", "ctrl+k|ctrl+shift+p", "General";
    "switcher", "Quick switcher: go to page", "ctrl+p", "General";
    "settings", "Settings", "ctrl+,", "General";
    "shortcuts", "Keyboard shortcuts reference", "ctrl+/", "General";
    "focus", "Focus mode (hide toolbars)", "f9", "View";
    "present", "Presentation mode (full screen page)", "f5", "View";
    "fullscreen", "Full screen", "f11", "View";
    "toggle-sidebar", "Show or hide the notes sidebar", "ctrl+\\", "View";
    "toggle-doc", "Show or hide the page (typeset blocks)", "ctrl+shift+d", "View";
    "split", "Split view: show another page beside this one", "", "View";
    "split-close", "Close split view", "", "View";
    "zoom-in", "Zoom in", "ctrl+=", "View";
    "zoom-out", "Zoom out", "ctrl+-", "View";
    "zoom-reset", "Reset zoom", "ctrl+0", "View";
    "theme-dark", "Theme: dark", "", "View";
    "theme-light", "Theme: light", "", "View";
    "theme-contrast", "Theme: high contrast", "", "View";
    "pen", "Pen", "alt+p", "Ink";
    "highlighter", "Highlighter", "alt+h", "Ink";
    "eraser", "Eraser", "alt+e", "Ink";
    "lasso", "Lasso select", "alt+l", "Ink";
    "pen1", "Favourite pen 1", "alt+1", "Ink";
    "pen2", "Favourite pen 2", "alt+2", "Ink";
    "pen3", "Favourite pen 3", "alt+3", "Ink";
    "mode-math", "Math ink mode (handwriting becomes math)", "alt+m", "Ink";
    "mode-text", "Text ink mode (handwriting becomes words)", "alt+w", "Ink";
    "textbox", "Type here: put typed text straight onto the page (or just start typing)", "alt+t", "Ink";
    "mode-drawing", "Drawing ink mode", "alt+d", "Ink";
    "undo", "Undo", "ctrl+z", "Ink";
    "redo", "Redo", "ctrl+y|ctrl+shift+z", "Ink";
    "clear", "Clear the ink canvas", "", "Ink";
    "delete-selection", "Delete selected ink", "delete", "Ink";
    "insert-axes", "Insert coordinate axes", "", "Ink";
    "insert-ruler", "Insert a ruler line", "", "Ink";
    "snap-toggle", "Shape snapping on/off (Drawing mode)", "", "Ink";
    "recognize", "Recognize now", "ctrl+r", "Math";
    "insert", "Insert expression into the page", "ctrl+enter", "Math";
    "typing", "Type math (keyboard)", "ctrl+t", "Math";
    "symbols", "Symbol palette", "ctrl+shift+s", "Math";
    "train", "Train my handwriting", "", "Math";
    "copy-latex", "Copy expression as LaTeX", "ctrl+shift+l", "Math";
    "copy-typst", "Copy expression as Typst", "", "Math";
    "copy-unicode", "Copy expression as text", "", "Math";
    "copy-image", "Copy expression as image", "", "Math";
    "tmpl-frac", "Template: fraction", "", "Math";
    "tmpl-sqrt", "Template: square root", "", "Math";
    "tmpl-int", "Template: integral", "", "Math";
    "tmpl-sum", "Template: sum", "", "Math";
    "tmpl-lim", "Template: limit", "", "Math";
    "tmpl-matrix", "Template: 2×2 matrix", "", "Math";
    "tmpl-cases", "Template: cases", "", "Math";
    "tmpl-deriv", "Template: derivative", "", "Math";
    "new-page", "New page", "ctrl+n", "Notes";
    "new-section", "New section", "", "Notes";
    "new-notebook", "New notebook", "", "Notes";
    "new-encrypted", "New encrypted notebook", "", "Notes";
    "lock", "Lock this notebook", "", "Notes";
    "duplicate", "Duplicate page", "", "Notes";
    "delete-page", "Move page to trash", "", "Notes";
    "trash", "Trash: restore deleted pages", "", "Notes";
    "history", "Version history of this page", "", "Notes";
    "favorite", "Favourite / unfavourite page", "ctrl+d", "Notes";
    "save-template", "Save page as a template", "", "Notes";
    "back", "Back", "alt+left", "Notes";
    "forward", "Forward", "alt+right", "Notes";
    "close-tab", "Close tab", "ctrl+w", "Notes";
    "next-tab", "Next tab", "ctrl+tab", "Notes";
    "search", "Search all pages", "ctrl+shift+f", "Notes";
    "find", "Find and replace on this page", "ctrl+f", "Notes";
    "text-block", "Add a text block", "ctrl+shift+t", "Page";
    "callout", "Add a callout", "", "Page";
    "rule", "Add a horizontal rule", "", "Page";
    "image", "Insert image from file", "", "Page";
    "paste-image", "Paste image from clipboard", "ctrl+shift+v", "Page";
    "pdf", "Import PDF (one page per PDF page, annotate with ink)", "", "Page";
    "plot", "Insert function plot", "", "Page";
    "graph", "Graph view (Desmos-style)", "ctrl+g", "Page";
    "save-graph", "Save the graph into the page", "", "Page";
    "record", "Record audio (synced to ink)", "", "Page";
    "play", "Play recording from the selected ink", "", "Page";
    "paper", "Next paper background", "", "Page";
    "page-mode", "Page size: infinite / Letter / A4", "", "Page";
    "export", "Export page", "ctrl+e", "Export";
    "export-notebook", "Export the whole notebook (zip)", "", "Export";
    "print", "Print", "ctrl+shift+o", "Export";
    "backup", "Back up now", "", "Data";
    "restore-backup", "Restore from a backup", "", "Data";
    "fmt-body", "Text: normal paragraph", "", "Text";
    "fmt-h1", "Text: heading 1", "", "Text";
    "fmt-h2", "Text: heading 2", "", "Text";
    "fmt-h3", "Text: heading 3", "", "Text";
    "fmt-bullet", "Text: bullet list", "", "Text";
    "fmt-numbered", "Text: numbered list", "", "Text";
    "fmt-check", "Text: checklist", "", "Text";
    "fmt-quote", "Text: quote", "", "Text";
    "fmt-bold", "Text: bold (Ctrl+B)", "", "Text";
    "fmt-italic", "Text: italic (Ctrl+I)", "", "Text";
    "fmt-underline", "Text: underline (Ctrl+U)", "", "Text";
    "fmt-code", "Text: code", "", "Text";
    "fmt-indent", "Text: indent list item (Tab)", "", "Text";
    "fmt-outdent", "Text: outdent list item (Shift+Tab)", "", "Text";
    "fmt-math", "Insert inline math", "", "Text";
    "fmt-link", "Link text to a page or web address", "", "Text";
    "insert-table", "Insert a table", "", "Page";
    "chip-dismiss", "Keep handwriting as ink (dismiss the suggestion)", "", "Ink";
    "outline", "Outline of this page (headings)", "", "Page";
    "word-count", "Word count", "", "Page";
    "today", "Open today's note", "", "Notes";
    "quick-note", "Quick note: a new blank page", "ctrl+shift+n", "Notes";
    "obj-smaller", "Object: smaller", "", "Object";
    "obj-larger", "Object: larger", "", "Object";
    "obj-crop", "Image: trim the edges", "", "Object";
    "obj-uncrop", "Image: restore the edges", "", "Object";
    "obj-copy-latex", "Math: copy as LaTeX", "", "Object";
    "obj-copy-typst", "Math: copy as Typst", "", "Object";
    "obj-delete", "Delete the selected object", "", "Object";
    "tablet", "Tablet panel", "", "Tablet";
}

pub fn get(id: &str) -> Option<&'static Command> {
    ALL.iter().find(|c| c.id == id)
}

/// Normalize a key event into the combo format used above.
pub fn combo(key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let k = match key {
        "\u{8}" => "backspace".to_string(),
        "\u{7f}" => "delete".to_string(),
        "\t" => "tab".to_string(),
        "\n" | "\r" => "enter".to_string(),
        "\u{1b}" => "escape".to_string(),
        " " => "space".to_string(),
        k if k.chars().count() == 1 => {
            let c = k.chars().next()?;
            match c as u32 {
                // Slint's private-use key codes for arrows and function keys
                0xF700 => "up".into(),
                0xF701 => "down".into(),
                0xF702 => "left".into(),
                0xF703 => "right".into(),
                0xF704..=0xF70F => format!("f{}", c as u32 - 0xF704 + 1),
                0xF729 => "home".into(),
                0xF72B => "end".into(),
                0xF72C => "pageup".into(),
                0xF72D => "pagedown".into(),
                0xF728 => "delete".into(),
                _ if c.is_control() => return None,
                _ => c.to_lowercase().to_string(),
            }
        }
        _ => return None,
    };
    let mut s = String::new();
    for (on, m) in [(ctrl, "ctrl+"), (alt, "alt+"), (shift && !is_shifted_symbol(&k), "shift+"), (meta, "meta+")] {
        if on {
            s += m;
        }
    }
    Some(s + &k)
}

/// Keys whose shifted form is already a different character (so "shift+" is not added twice).
fn is_shifted_symbol(k: &str) -> bool {
    k.chars().count() == 1 && !k.chars().all(|c| c.is_alphanumeric())
}

/// Fuzzy filter for the palette: every query word must appear in the title or id.
pub fn matches(c: &Command, query: &str) -> bool {
    let hay = format!("{} {} {}", c.title, c.id, c.group).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// Nice display form of a combo: "ctrl+shift+p" -> "Ctrl+Shift+P".
pub fn pretty(combo: &str) -> String {
    combo
        .split('|')
        .map(|c| c.split('+').map(|p| {
            let mut ch = p.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => "+".into(),
            }
        }).collect::<Vec<_>>().join("+"))
        .collect::<Vec<_>>()
        .join("  or  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_default_keys_do_not_clash() {
        let mut ids: Vec<&str> = ALL.iter().map(|c| c.id).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate command id");
        let mut keys: Vec<&str> = ALL.iter().flat_map(|c| c.key.split('|')).filter(|k| !k.is_empty()).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "two commands share a default shortcut");
    }

    #[test]
    fn combos_normalize() {
        assert_eq!(combo("K", true, false, true, false).as_deref(), Some("ctrl+shift+k"));
        assert_eq!(combo("\u{F702}", false, true, false, false).as_deref(), Some("alt+left"));
        assert_eq!(combo("\u{F70A}", false, false, false, false).as_deref(), Some("f7"));
        assert_eq!(combo("\u{7f}", false, false, false, false).as_deref(), Some("delete"));
        assert_eq!(combo("?", true, false, true, false).as_deref(), Some("ctrl+?"));
        assert_eq!(pretty("ctrl+k|ctrl+shift+p"), "Ctrl+K  or  Ctrl+Shift+P");
    }

    #[test]
    fn palette_filter() {
        let hits: Vec<&str> = ALL.iter().filter(|c| matches(c, "exp note")).map(|c| c.id).collect();
        assert_eq!(hits, vec!["export-notebook"]);
    }
}
