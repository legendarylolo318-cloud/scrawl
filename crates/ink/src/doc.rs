use crate::{dist_point_segment, InkMode, PenKind, Point, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EraserMode {
    /// Remove whole strokes touched by the eraser.
    #[default]
    Stroke,
    /// Cut strokes into pieces, removing only the touched points.
    Pixel,
}

#[derive(Clone, Debug)]
enum Edit {
    Add(Stroke),
    /// (index, stroke) pairs removed, ascending index; plus strokes added in their place.
    Replace { removed: Vec<(usize, Stroke)>, added: Vec<Stroke> },
}

/// A set of strokes with full undo/redo.
#[derive(Default)]
pub struct Document {
    pub strokes: Vec<Stroke>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    next_id: u64,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn add_stroke(&mut self, mut s: Stroke) -> u64 {
        if s.id == 0 {
            s.id = self.alloc_id();
        }
        let id = s.id;
        self.strokes.push(s.clone());
        self.undo.push(Edit::Add(s));
        self.redo.clear();
        id
    }

    /// Convenience constructor for strokes with defaults.
    pub fn make_stroke(points: Vec<Point>, color: [u8; 4], width: f32, kind: PenKind, mode: InkMode) -> Stroke {
        Stroke { id: 0, points, color, width, kind, mode, t0_ms: 0 }
    }

    /// Delete strokes by id as one undoable edit. Returns how many were removed.
    pub fn remove_ids(&mut self, ids: &[u64]) -> usize {
        let mut removed = Vec::new();
        for (i, s) in self.strokes.iter().enumerate() {
            if ids.contains(&s.id) {
                removed.push((i, s.clone()));
            }
        }
        if removed.is_empty() {
            return 0;
        }
        for (i, _) in removed.iter().rev() {
            self.strokes.remove(*i);
        }
        let n = removed.len();
        self.undo.push(Edit::Replace { removed, added: vec![] });
        self.redo.clear();
        n
    }

    /// Strokes that come within `radius` of (x, y).
    pub fn hit_test(&self, x: f32, y: f32, radius: f32) -> Vec<u64> {
        self.strokes
            .iter()
            .filter(|s| {
                let r = radius + s.width * 0.5;
                match s.points.as_slice() {
                    [] => false,
                    [p] => (p.x - x).hypot(p.y - y) <= r,
                    pts => pts.windows(2).any(|w| dist_point_segment(x, y, &w[0], &w[1]) <= r),
                }
            })
            .map(|s| s.id)
            .collect()
    }

    /// Erase along a path of eraser positions as a single undoable edit.
    pub fn erase_path(&mut self, path: &[(f32, f32)], radius: f32, mode: EraserMode) -> usize {
        match mode {
            EraserMode::Stroke => {
                let mut ids: Vec<u64> = Vec::new();
                for &(x, y) in path {
                    for id in self.hit_test(x, y, radius) {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                }
                self.remove_ids(&ids)
            }
            EraserMode::Pixel => self.pixel_erase(path, radius),
        }
    }

    fn pixel_erase(&mut self, path: &[(f32, f32)], radius: f32) -> usize {
        let touched = |p: &Point| path.iter().any(|&(x, y)| (p.x - x).hypot(p.y - y) <= radius);
        let mut removed = Vec::new();
        let mut added = Vec::new();
        let mut next = self.next_id;
        for (i, s) in self.strokes.iter().enumerate() {
            if !s.points.iter().any(touched) {
                continue;
            }
            removed.push((i, s.clone()));
            let mut cur: Vec<Point> = Vec::new();
            let flush = |cur: &mut Vec<Point>, added: &mut Vec<Stroke>, next: &mut u64| {
                if cur.len() >= 2 {
                    *next += 1;
                    added.push(Stroke { id: *next, points: std::mem::take(cur), ..s.clone() });
                } else {
                    cur.clear();
                }
            };
            for p in &s.points {
                if touched(p) {
                    flush(&mut cur, &mut added, &mut next);
                } else {
                    cur.push(*p);
                }
            }
            flush(&mut cur, &mut added, &mut next);
        }
        if removed.is_empty() {
            return 0;
        }
        self.next_id = next;
        for (i, _) in removed.iter().rev() {
            self.strokes.remove(*i);
        }
        self.strokes.extend(added.iter().cloned());
        let n = removed.len();
        self.undo.push(Edit::Replace { removed, added });
        self.redo.clear();
        n
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else { return false };
        match &e {
            Edit::Add(s) => self.strokes.retain(|x| x.id != s.id),
            Edit::Replace { removed, added } => {
                self.strokes.retain(|x| !added.iter().any(|a| a.id == x.id));
                for (i, s) in removed {
                    let at = (*i).min(self.strokes.len());
                    self.strokes.insert(at, s.clone());
                }
            }
        }
        self.redo.push(e);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else { return false };
        match &e {
            Edit::Add(s) => self.strokes.push(s.clone()),
            Edit::Replace { removed, added } => {
                for (i, _) in removed.iter().rev() {
                    if *i < self.strokes.len() {
                        self.strokes.remove(*i);
                    }
                }
                self.strokes.extend(added.iter().cloned());
            }
        }
        self.undo.push(e);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(y: f32) -> Stroke {
        let pts = (0..10).map(|i| Point::new(i as f32 * 10.0, y, i * 8)).collect();
        Document::make_stroke(pts, [255; 4], 2.0, PenKind::Pen, InkMode::Drawing)
    }

    #[test]
    fn undo_redo_add() {
        let mut d = Document::new();
        d.add_stroke(line(0.0));
        d.add_stroke(line(50.0));
        assert_eq!(d.strokes.len(), 2);
        assert!(d.undo());
        assert_eq!(d.strokes.len(), 1);
        assert!(d.redo());
        assert_eq!(d.strokes.len(), 2);
        d.undo();
        d.add_stroke(line(9.0));
        assert!(!d.can_redo(), "new edit clears redo");
    }

    #[test]
    fn stroke_erase_and_undo() {
        let mut d = Document::new();
        d.add_stroke(line(0.0));
        d.add_stroke(line(100.0));
        assert_eq!(d.erase_path(&[(40.0, 3.0)], 5.0, EraserMode::Stroke), 1);
        assert_eq!(d.strokes.len(), 1);
        d.undo();
        assert_eq!(d.strokes.len(), 2);
        d.redo();
        assert_eq!(d.strokes.len(), 1);
    }

    #[test]
    fn pixel_erase_splits_and_undoes() {
        let mut d = Document::new();
        d.add_stroke(line(0.0));
        d.erase_path(&[(45.0, 0.0)], 6.0, EraserMode::Pixel);
        assert_eq!(d.strokes.len(), 2, "split into two pieces");
        d.undo();
        assert_eq!(d.strokes.len(), 1);
        assert_eq!(d.strokes[0].points.len(), 10);
        d.redo();
        assert_eq!(d.strokes.len(), 2);
    }
}
