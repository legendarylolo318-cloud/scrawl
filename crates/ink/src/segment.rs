use crate::{bbox, Rect, Stroke};

#[derive(Clone, Copy, Debug)]
pub struct SegmentConfig {
    /// Strokes started more than this long apart (after the previous one ended) never merge.
    pub max_gap_ms: u64,
    /// Max horizontal gap between bboxes, as a fraction of the larger stroke height.
    pub x_gap_frac: f32,
    /// Max vertical gap for stacked strokes (=, i, ±, ≠), fraction of larger height.
    pub y_gap_frac: f32,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self { max_gap_ms: 700, x_gap_frac: 0.15, y_gap_frac: 0.7 }
    }
}

fn stroke_end(s: &Stroke) -> u64 {
    s.t0_ms + s.points.last().map_or(0, |p| p.t_ms as u64)
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

fn should_merge(a: &Stroke, ra: Rect, b: &Stroke, rb: Rect, cfg: SegmentConfig) -> bool {
    let gap = b.t0_ms.saturating_sub(stroke_end(a));
    if gap > cfg.max_gap_ms {
        return false;
    }
    let big = ra.height().max(rb.height()).max(ra.width()).max(rb.width()).max(1.0);
    let xo = overlap(ra.x0, ra.x1, rb.x0, rb.x1);
    let min_w = ra.width().min(rb.width());
    let x_gap = (rb.x0 - ra.x1).max(ra.x0 - rb.x1).max(0.0);
    let y_gap = (rb.y0 - ra.y1).max(ra.y0 - rb.y1).max(0.0);
    // Stacked (=, i, ±, ≠, ∄ ...): substantial horizontal overlap and small vertical gap.
    let stacked = xo >= 0.5 * min_w.max(1.0) && y_gap <= cfg.y_gap_frac * big;
    // Crossing / touching (x, +, t ...): bboxes intersect substantially.
    let yo = overlap(ra.y0, ra.y1, rb.y0, rb.y1);
    let crossing = xo > 0.0 && yo > 0.0 && xo * yo >= 0.25 * (ra.width() * ra.height()).min(rb.width() * rb.height());
    // Side by side but nearly touching (e.g. two halves of a bracket-like symbol).
    let adjacent = x_gap <= cfg.x_gap_frac * big && yo >= 0.6 * ra.height().min(rb.height()) && gap < 150;
    stacked || crossing || adjacent
}

/// Group strokes into symbols. Returns groups of stroke indices (into `strokes`),
/// each in time order, groups ordered left to right.
pub fn group_symbols(strokes: &[Stroke], cfg: SegmentConfig) -> Vec<Vec<usize>> {
    let n = strokes.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| strokes[i].t0_ms);
    let boxes: Vec<Option<Rect>> = strokes.iter().map(|s| bbox(&s.points)).collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut Vec<usize>, i: usize) -> usize {
        if p[i] != i {
            let r = find(p, p[i]);
            p[i] = r;
        }
        p[i]
    }
    for (k, &j) in order.iter().enumerate() {
        for &i in order[..k].iter().rev().take(6) {
            if let (Some(ra), Some(rb)) = (boxes[i], boxes[j]) {
                if should_merge(&strokes[i], ra, &strokes[j], rb, cfg) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[b] = a;
                }
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut root_idx = std::collections::HashMap::new();
    for &i in &order {
        let r = find(&mut parent, i);
        let gi = *root_idx.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[gi].push(i);
    }
    groups.sort_by(|a, b| {
        let cx = |g: &Vec<usize>| g.iter().filter_map(|&i| boxes[i]).map(|r| r.x0).fold(f32::MAX, f32::min);
        cx(a).total_cmp(&cx(b))
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InkMode, PenKind, Point};

    fn seg(x0: f32, y0: f32, x1: f32, y1: f32, t0: u64) -> Stroke {
        let pts = (0..=10)
            .map(|i| {
                let f = i as f32 / 10.0;
                Point::new(x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, i * 10)
            })
            .collect();
        Stroke { id: 0, points: pts, color: [255; 4], width: 2.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: t0 }
    }

    #[test]
    fn equals_sign_is_one_symbol() {
        let g = group_symbols(&[seg(0.0, 0.0, 40.0, 0.0, 0), seg(0.0, 15.0, 40.0, 15.0, 300)], SegmentConfig::default());
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn x_drawn_as_two_strokes_merges() {
        let g = group_symbols(&[seg(0.0, 0.0, 40.0, 40.0, 0), seg(40.0, 0.0, 0.0, 40.0, 300)], SegmentConfig::default());
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn distant_or_late_strokes_stay_separate() {
        let a = group_symbols(&[seg(0.0, 0.0, 10.0, 40.0, 0), seg(80.0, 0.0, 90.0, 40.0, 300)], SegmentConfig::default());
        assert_eq!(a.len(), 2);
        let b = group_symbols(&[seg(0.0, 0.0, 40.0, 0.0, 0), seg(0.0, 15.0, 40.0, 15.0, 5000)], SegmentConfig::default());
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn groups_ordered_left_to_right() {
        let g = group_symbols(&[seg(100.0, 0.0, 110.0, 40.0, 0), seg(0.0, 0.0, 10.0, 40.0, 200)], SegmentConfig::default());
        assert_eq!(g, vec![vec![1], vec![0]]);
    }
}
