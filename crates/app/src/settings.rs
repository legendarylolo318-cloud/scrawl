//! Persisted user settings (`settings.json` in the config folder): ink defaults, theme, UI scale,
//! shortcuts, favourite pens, backups, and the window/tab state to restore on the next start.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pen {
    pub rgb: [u8; 3],
    pub width: f32,
    pub highlighter: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeColors {
    pub bg: [u8; 3],
    pub surface: [u8; 3],
    pub panel: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
    pub urgent: [u8; 3],
}

impl ThemeColors {
    pub fn dark() -> Self {
        Self { bg: [0x1a, 0x1b, 0x26], surface: [0x24, 0x28, 0x3b], panel: [0x1f, 0x23, 0x35], text: [0xc0, 0xca, 0xf5], muted: [0x73, 0x7a, 0xa2], accent: [0x7a, 0xa2, 0xf7], urgent: [0xf7, 0x76, 0x8e] }
    }
    pub fn light() -> Self {
        Self { bg: [0xf7, 0xf7, 0xfa], surface: [0xe4, 0xe7, 0xf0], panel: [0xee, 0xf0, 0xf6], text: [0x1f, 0x23, 0x35], muted: [0x5a, 0x60, 0x80], accent: [0x2e, 0x5c, 0xd6], urgent: [0xc0, 0x26, 0x4a] }
    }
    pub fn high_contrast() -> Self {
        Self { bg: [0, 0, 0], surface: [0x20, 0x20, 0x20], panel: [0x10, 0x10, 0x10], text: [0xff, 0xff, 0xff], muted: [0xc8, 0xc8, 0xc8], accent: [0xff, 0xd7, 0x00], urgent: [0xff, 0x55, 0x55] }
    }
    pub fn is_dark(&self) -> bool {
        let [r, g, b] = self.bg;
        (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000 < 128
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub smoothing: f32,
    pub width: f32,
    pub recog_delay_ms: u64,
    pub paper_kind: String,
    pub paper_spacing: f32,
    pub paper_color: [u8; 3],
    pub page_mode: String,
    /// "dark" | "light" | "contrast" | "custom"
    pub theme: String,
    pub custom: ThemeColors,
    pub ui_scale: f32,
    pub tablet_mode: bool,
    pub snap_shapes: bool,
    pub spell_check: bool,
    pub pens: Vec<Pen>,
    pub custom_colors: Vec<[u8; 3]>,
    /// command id -> key combo, e.g. "ctrl+k"
    pub shortcuts: BTreeMap<String, String>,
    pub backup_daily: bool,
    pub backup_keep: usize,
    pub backup_dir: Option<PathBuf>,
    pub window: Option<(u32, u32)>,
    pub maximized: bool,
    pub last_notebook: Option<String>,
    pub last_page: Option<String>,
    pub tabs: Vec<String>,
    pub show_doc: bool,
    /// Show the page with the new document editor (off until it is finished).
    pub new_editor: bool,
    pub show_sidebar: bool,
    pub tablet_idle_min: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            smoothing: 0.5,
            width: 3.0,
            recog_delay_ms: 600,
            paper_kind: "lined".into(),
            paper_spacing: 32.0,
            paper_color: [0x41, 0x48, 0x68],
            page_mode: "infinite".into(),
            theme: "dark".into(),
            custom: ThemeColors::dark(),
            ui_scale: 1.0,
            tablet_mode: false,
            snap_shapes: true,
            spell_check: true,
            pens: vec![
                Pen { rgb: [0xc0, 0xca, 0xf5], width: 3.0, highlighter: false },
                Pen { rgb: [0xf7, 0x76, 0x8e], width: 2.0, highlighter: false },
                Pen { rgb: [0xe0, 0xaf, 0x68], width: 6.0, highlighter: true },
            ],
            custom_colors: vec![],
            shortcuts: BTreeMap::new(),
            backup_daily: true,
            backup_keep: 14,
            backup_dir: None,
            window: None,
            maximized: false,
            last_notebook: None,
            last_page: None,
            tabs: vec![],
            show_doc: true,
            show_sidebar: true,
            new_editor: false,
            tablet_idle_min: 15,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        crate::session::config_dir().map(|d| d.join("settings.json"))
    }

    pub fn load() -> Self {
        let mut s: Settings = Self::path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        s.sanitize();
        s
    }

    /// Clamp everything a hand-edited or corrupt file could get wrong.
    pub fn sanitize(&mut self) {
        self.smoothing = finite_or(self.smoothing, 0.5).clamp(0.0, 1.0);
        self.width = finite_or(self.width, 3.0).clamp(0.5, 24.0);
        self.recog_delay_ms = self.recog_delay_ms.clamp(150, 5000);
        self.paper_spacing = finite_or(self.paper_spacing, 32.0).clamp(8.0, 128.0);
        self.ui_scale = finite_or(self.ui_scale, 1.0).clamp(0.75, 2.0);
        self.backup_keep = self.backup_keep.clamp(1, 365);
        self.tablet_idle_min = self.tablet_idle_min.clamp(1, 240);
        while self.pens.len() < 3 {
            self.pens.push(Pen { rgb: [0xc0, 0xca, 0xf5], width: 3.0, highlighter: false });
        }
        self.pens.truncate(3);
        for p in &mut self.pens {
            p.width = finite_or(p.width, 3.0).clamp(0.5, 24.0);
        }
        self.custom_colors.truncate(12);
        self.tabs.truncate(12);
        if let Some((w, h)) = self.window {
            if w < 400 || h < 300 || w > 16384 || h > 16384 {
                self.window = None;
            }
        }
    }

    pub fn save(&self) {
        let Some(p) = Self::path() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = store::fsio::atomic_write(&p, &b);
        }
    }

    pub fn colors(&self) -> ThemeColors {
        match self.theme.as_str() {
            "light" => ThemeColors::light(),
            "contrast" => ThemeColors::high_contrast(),
            "custom" => self.custom.clone(),
            _ => ThemeColors::dark(),
        }
    }

    /// Effective binding for a command (user override, else default).
    pub fn binding(&self, id: &str) -> Option<String> {
        if let Some(k) = self.shortcuts.get(id) {
            return (!k.is_empty()).then(|| k.clone());
        }
        crate::commands::ALL.iter().find(|c| c.id == id).and_then(|c| (!c.key.is_empty()).then(|| c.key.to_string()))
    }

    /// Command bound to a key combo (a binding may list alternatives separated by `|`).
    pub fn command_for(&self, combo: &str) -> Option<&'static str> {
        crate::commands::ALL.iter().find(|c| self.binding(c.id).is_some_and(|b| b.split('|').any(|k| k == combo))).map(|c| c.id)
    }
}

fn finite_or(v: f32, d: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_values_are_clamped_and_missing_fields_default() {
        let mut s: Settings = serde_json::from_str(r#"{"smoothing": 9, "ui_scale": 0.1, "pens": [], "window": [10, 10]}"#).unwrap();
        s.sanitize();
        assert_eq!(s.smoothing, 1.0);
        assert_eq!(s.ui_scale, 0.75);
        assert_eq!(s.pens.len(), 3);
        assert_eq!(s.window, None);
        assert_eq!(s.recog_delay_ms, 600, "missing fields take defaults");
    }

    #[test]
    fn shortcuts_can_be_rebound_and_cleared() {
        let mut s = Settings::default();
        assert_eq!(s.command_for("ctrl+k"), Some("palette"));
        s.shortcuts.insert("palette".into(), "ctrl+shift+space".into());
        assert_eq!(s.command_for("ctrl+shift+space"), Some("palette"));
        assert_ne!(s.command_for("ctrl+k"), Some("palette"));
        s.shortcuts.insert("undo".into(), String::new());
        assert_eq!(s.binding("undo"), None, "an empty binding removes the shortcut");
    }

    #[test]
    fn themes_have_readable_contrast() {
        for t in [ThemeColors::dark(), ThemeColors::light(), ThemeColors::high_contrast()] {
            let lum = |c: [u8; 3]| (c[0] as f32 * 0.299 + c[1] as f32 * 0.587 + c[2] as f32 * 0.114) / 255.0;
            assert!((lum(t.text) - lum(t.bg)).abs() > 0.5, "text must stand out from the background");
        }
    }
}
