//! Recognition session: groups strokes into symbols, classifies them (CNN + personal samples),
//! tracks the candidate strip, and learns from corrections. UI-independent so it can be tested.

use ink::{bbox, group_symbols, InkMode, PenKind, Rect, SegmentConfig, Stroke};
use layout::Glyph;
use recog::{Candidate, PersonalStore, Recognizer};
use std::path::PathBuf;

pub struct SymbolRec {
    pub stroke_ids: Vec<u64>,
    pub rect: Rect,
    pub label: String,
    pub cands: Vec<Candidate>,
}

pub struct Session {
    rec: Option<Recognizer>,
    pub symbols: Vec<SymbolRec>,
    /// The symbol whose candidates the strip shows (the most recently drawn).
    pub selected: Option<usize>,
    personal_path: Option<PathBuf>,
}

pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("scrawl"))
}

impl Session {
    /// `personal_path`: where personal samples persist (None = in memory only).
    pub fn new(personal_path: Option<PathBuf>) -> Self {
        Self { rec: None, symbols: vec![], selected: None, personal_path }
    }

    fn ensure(&mut self) -> Result<&mut Recognizer, String> {
        if self.rec.is_none() {
            let store = self
                .personal_path
                .as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| PersonalStore::from_json(&s))
                .unwrap_or_default();
            self.rec = Some(Recognizer::new(store).map_err(|e| e.to_string())?);
        }
        Ok(self.rec.as_mut().unwrap())
    }

    fn save(&self) {
        let (Some(rec), Some(path)) = (&self.rec, &self.personal_path) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // atomic write: temp file, then rename
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, rec.personal.to_json()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn labels(&mut self) -> Vec<String> {
        self.ensure().map(|r| r.cnn.labels().to_vec()).unwrap_or_default()
    }

    pub fn personal_count(&mut self, label: &str) -> usize {
        self.ensure().map(|r| r.personal.count(label)).unwrap_or(0)
    }

    fn math_strokes(all: &[Stroke]) -> Vec<Stroke> {
        all.iter().filter(|s| s.mode == InkMode::MathText && s.kind == PenKind::Pen).cloned().collect()
    }

    /// Re-run grouping + classification over the Math/Text strokes.
    pub fn recognize(&mut self, all: &[Stroke]) -> Result<(), String> {
        let strokes = Self::math_strokes(all);
        let groups = group_symbols(&strokes, SegmentConfig::default());
        let prev: Vec<(Vec<u64>, String)> = self.symbols.iter().map(|s| (s.stroke_ids.clone(), s.label.clone())).collect();
        let rec = self.ensure()?;
        let mut out = Vec::new();
        for g in &groups {
            let members: Vec<&Stroke> = g.iter().map(|&i| &strokes[i]).collect();
            let ids: Vec<u64> = members.iter().map(|s| s.id).collect();
            let cands = rec.classify(&members, 5).map_err(|e| e.to_string())?;
            let pts: Vec<ink::Point> = members.iter().flat_map(|s| s.points.iter().copied()).collect();
            let Some(rect) = bbox(&pts) else { continue };
            // A symbol the user already corrected keeps its chosen label.
            let label = match prev.iter().find(|(p, _)| *p == ids) {
                Some((_, l)) if cands.iter().any(|c| &c.label == l) => l.clone(),
                _ => cands.first().map(|c| c.label.clone()).unwrap_or_default(),
            };
            out.push(SymbolRec { stroke_ids: ids, rect, label, cands });
        }
        // the most recently drawn symbol feeds the candidate strip
        self.selected = out
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| strokes.iter().filter(|st| s.stroke_ids.contains(&st.id)).map(|st| st.t0_ms).max().unwrap_or(0))
            .map(|(i, _)| i);
        self.symbols = out;
        Ok(())
    }

    /// Replace the selected symbol with candidate `idx`; saves the strokes as a personal sample.
    pub fn choose(&mut self, idx: usize, all: &[Stroke]) -> bool {
        let Some(sel) = self.selected else { return false };
        let Some(label) = self.symbols.get(sel).and_then(|s| s.cands.get(idx)).map(|c| c.label.clone()) else { return false };
        let ids = self.symbols[sel].stroke_ids.clone();
        let strokes: Vec<&Stroke> = all.iter().filter(|s| ids.contains(&s.id)).collect();
        self.symbols[sel].label = label.clone();
        if let Ok(rec) = self.ensure() {
            rec.learn(&label, &strokes);
        }
        self.save();
        true
    }

    /// Save the strokes of symbol `i` as a personal sample of `label` (a correction made elsewhere, such as
    /// picking a different word). Returns false if the recognizer does not know that label.
    pub fn learn_symbol(&mut self, i: usize, label: &str, all: &[Stroke]) -> bool {
        let Some(ids) = self.symbols.get(i).map(|s| s.stroke_ids.clone()) else { return false };
        let strokes: Vec<&Stroke> = all.iter().filter(|s| ids.contains(&s.id)).collect();
        let Ok(rec) = self.ensure() else { return false };
        if !rec.cnn.labels().iter().any(|l| l == label) {
            return false;
        }
        rec.learn(label, &strokes);
        self.symbols[i].label = label.to_string();
        self.save();
        true
    }

    /// Training mode: store every symbol currently on the canvas as a sample of `label`.
    /// Returns the stroke ids consumed so the caller can clear them.
    pub fn train(&mut self, label: &str, all: &[Stroke]) -> Result<Vec<u64>, String> {
        let strokes = Self::math_strokes(all);
        let groups = group_symbols(&strokes, SegmentConfig::default());
        let rec = self.ensure()?;
        let mut used = Vec::new();
        for g in &groups {
            let members: Vec<&Stroke> = g.iter().map(|&i| &strokes[i]).collect();
            rec.learn(label, &members);
            used.extend(members.iter().map(|s| s.id));
        }
        self.save();
        Ok(used)
    }

    /// Typeset-able expression from the current symbols (spatial layout).
    pub fn expression(&self) -> layout::Expr {
        let glyphs: Vec<Glyph> = self.symbols.iter().map(|s| Glyph { label: s.label.clone(), rect: s.rect }).collect();
        layout::parse(&glyphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::Point;

    fn line(id: u64, t0: u64, a: (f32, f32), b: (f32, f32)) -> Stroke {
        let pts = (0..=20)
            .map(|i| {
                let t = i as f32 / 20.0;
                Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, i as u32 * 8)
            })
            .collect();
        Stroke { id, points: pts, color: [255; 4], width: 3.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: t0 }
    }

    fn plus() -> Vec<Stroke> {
        vec![line(1, 0, (0.0, 20.0), (40.0, 20.0)), line(2, 300, (20.0, 0.0), (20.0, 40.0))]
    }

    #[test]
    fn recognizes_and_builds_candidate_strip() {
        let mut s = Session::new(None);
        s.recognize(&plus()).unwrap();
        assert_eq!(s.symbols.len(), 1);
        assert_eq!(s.symbols[0].cands.len(), 5);
        assert!(s.symbols[0].cands.iter().any(|c| c.label == "+"));
        assert_eq!(s.selected, Some(0));
    }

    #[test]
    fn corrections_become_personal_samples_and_persist() {
        let dir = std::env::temp_dir().join(format!("scrawl-test-{}", std::process::id()));
        let path = dir.join("personal.json");
        let strokes = plus();
        {
            let mut s = Session::new(Some(path.clone()));
            s.recognize(&strokes).unwrap();
            // pick the second candidate regardless of what it is
            let second = s.symbols[0].cands[1].label.clone();
            assert!(s.choose(1, &strokes));
            assert_eq!(s.symbols[0].label, second);
            assert_eq!(s.personal_count(&second), 1);
        }
        assert!(path.exists(), "personal samples saved automatically");
        let mut s2 = Session::new(Some(path.clone()));
        let label = s2.labels()[0].clone();
        let _ = label;
        let total: usize = s2.labels().iter().map(|l| s2.personal_count(l)).sum();
        assert_eq!(total, 1, "reloaded from disk");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn three_corrections_make_personal_label_win() {
        let mut s = Session::new(None);
        let strokes = plus();
        s.recognize(&strokes).unwrap();
        let target = s.symbols[0].cands[2].label.clone(); // deliberately not the top guess
        for _ in 0..3 {
            s.choose(2, &strokes);
        }
        s.recognize(&strokes).unwrap();
        assert_eq!(s.symbols[0].cands[0].label, target, "{:?}", s.symbols[0].cands);
    }

    #[test]
    fn a_word_correction_teaches_the_letter() {
        let mut s = Session::new(None);
        let strokes = plus();
        s.recognize(&strokes).unwrap();
        assert!(s.learn_symbol(0, "t", &strokes));
        assert_eq!(s.personal_count("t"), 1);
        assert!(!s.learn_symbol(0, "no such label", &strokes));
        assert!(!s.learn_symbol(9, "t", &strokes));
    }

    #[test]
    fn training_mode_consumes_strokes() {
        let mut s = Session::new(None);
        let used = s.train("+", &plus()).unwrap();
        assert_eq!(used.len(), 2);
        assert_eq!(s.personal_count("+"), 1);
    }
}
