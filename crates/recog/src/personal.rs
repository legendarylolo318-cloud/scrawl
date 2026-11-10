//! $P+ point-cloud recognizer (Vatavu, Anthony, Wobbrock 2012) over the user's own samples,
//! and fusion with the CNN's probabilities.

use ink::Stroke;
use serde::{Deserialize, Serialize};

const N: usize = 32;
/// Distance at or below which a personal match counts as fully confident.
const D_GOOD: f32 = 0.25;
/// Sharpness of the distance -> similarity conversion.
const BETA: f32 = 6.0;
pub const PRIORITY_MIN_SAMPLES: usize = 3;
const MAX_SAMPLES_PER_LABEL: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq)]
struct P {
    x: f32,
    y: f32,
    stroke: u32,
    /// Normalized turning angle in [0, 1] ($P+).
    theta: f32,
}

type Cloud = [P; N];

/// A stored sample: raw stroke polylines (so the cloud can be recomputed if the algorithm changes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub label: String,
    pub strokes: Vec<Vec<(f32, f32)>>,
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Resample to N equidistant points along the strokes (gaps between strokes are not counted).
fn resample(strokes: &[Vec<(f32, f32)>]) -> Vec<(f32, f32, u32)> {
    let total: f32 = strokes.iter().map(|s| s.windows(2).map(|w| dist(w[0], w[1])).sum::<f32>()).sum();
    let interval = (total / (N as f32 - 1.0)).max(1e-6);
    let mut out: Vec<(f32, f32, u32)> = Vec::with_capacity(N);
    let mut carry = 0.0f32;
    for (si, s) in strokes.iter().enumerate() {
        let si = si as u32;
        if s.is_empty() {
            continue;
        }
        if carry == 0.0 && (out.is_empty() || out.last().map(|l| l.2) != Some(si)) {
            out.push((s[0].0, s[0].1, si));
        }
        let mut prev = s[0];
        let mut i = 1;
        let mut pts: Vec<(f32, f32)> = s.clone();
        while i < pts.len() {
            let cur = pts[i];
            let d = dist(prev, cur);
            if carry + d >= interval && d > 0.0 {
                let t = (interval - carry) / d;
                let q = (prev.0 + t * (cur.0 - prev.0), prev.1 + t * (cur.1 - prev.1));
                out.push((q.0, q.1, si));
                pts.insert(i, q);
                prev = q;
                carry = 0.0;
                i += 1;
            } else {
                carry += d;
                prev = cur;
                i += 1;
            }
        }
        // a stroke boundary resets accumulated distance
        carry = 0.0;
    }
    // Ensure exactly N points (rounding can leave us one short/over).
    while out.len() < N {
        let l = *out.last().unwrap_or(&(0.0, 0.0, 0));
        out.push(l);
    }
    out.truncate(N);
    out
}

fn cloud(strokes: &[Vec<(f32, f32)>]) -> Option<Cloud> {
    if strokes.iter().all(|s| s.is_empty()) {
        return None;
    }
    let pts = resample(strokes);
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    let scale = (x1 - x0).max(y1 - y0).max(1e-4);
    let mut v: Vec<(f32, f32, u32)> = pts.iter().map(|p| ((p.0 - x0) / scale, (p.1 - y0) / scale, p.2)).collect();
    let (cx, cy) = (v.iter().map(|p| p.0).sum::<f32>() / N as f32, v.iter().map(|p| p.1).sum::<f32>() / N as f32);
    for p in v.iter_mut() {
        p.0 -= cx;
        p.1 -= cy;
    }
    let mut out = [P { x: 0.0, y: 0.0, stroke: 0, theta: 0.0 }; N];
    for i in 0..N {
        let mut theta = 0.0;
        if i > 0 && i + 1 < N && v[i - 1].2 == v[i].2 && v[i].2 == v[i + 1].2 {
            let (ax, ay) = (v[i].0 - v[i - 1].0, v[i].1 - v[i - 1].1);
            let (bx, by) = (v[i + 1].0 - v[i].0, v[i + 1].1 - v[i].1);
            let (na, nb) = (ax.hypot(ay), bx.hypot(by));
            if na > 1e-6 && nb > 1e-6 {
                let c = ((ax * bx + ay * by) / (na * nb)).clamp(-1.0, 1.0);
                theta = c.acos() / std::f32::consts::PI;
            }
        }
        out[i] = P { x: v[i].0, y: v[i].1, stroke: v[i].2, theta };
    }
    Some(out)
}

fn pdist(a: &P, b: &P) -> f32 {
    let (dx, dy, dt) = (a.x - b.x, a.y - b.y, a.theta - b.theta);
    (dx * dx + dy * dy + dt * dt).sqrt()
}

fn cloud_distance(a: &Cloud, b: &Cloud, start: usize) -> f32 {
    let mut matched = [false; N];
    let mut sum = 0.0;
    let mut i = start;
    loop {
        let mut best = f32::MAX;
        let mut idx = 0;
        for (j, m) in matched.iter().enumerate() {
            if !m {
                let d = pdist(&a[i], &b[j]);
                if d < best {
                    best = d;
                    idx = j;
                }
            }
        }
        matched[idx] = true;
        let weight = 1.0 - ((i + N - start) % N) as f32 / N as f32;
        sum += weight * best;
        i = (i + 1) % N;
        if i == start {
            break;
        }
    }
    sum
}

/// Greedy cloud matching, both directions, a few restarts. Lower = more similar.
fn greedy_distance(a: &Cloud, b: &Cloud) -> f32 {
    let step = (N as f32).sqrt().floor() as usize;
    let mut best = f32::MAX;
    let mut i = 0;
    while i < N {
        best = best.min(cloud_distance(a, b, i)).min(cloud_distance(b, a, i));
        i += step;
    }
    // Normalize by the total weight so distances are comparable (~per-point distance).
    best / ((N as f32 + 1.0) / 2.0)
}

#[derive(Default, Serialize, Deserialize)]
pub struct PersonalStore {
    pub samples: Vec<Sample>,
}

fn to_raw(strokes: &[&Stroke]) -> Vec<Vec<(f32, f32)>> {
    strokes.iter().map(|s| s.points.iter().map(|p| (p.x, p.y)).collect()).collect()
}

impl PersonalStore {
    pub fn count(&self, label: &str) -> usize {
        self.samples.iter().filter(|s| s.label == label).count()
    }

    pub fn add(&mut self, label: &str, strokes: &[&Stroke]) {
        let raw = to_raw(strokes);
        if cloud(&raw).is_none() {
            return;
        }
        self.samples.push(Sample { label: label.to_string(), strokes: raw });
        // keep the newest MAX_SAMPLES_PER_LABEL per label
        while self.count(label) > MAX_SAMPLES_PER_LABEL {
            if let Some(i) = self.samples.iter().position(|s| s.label == label) {
                self.samples.remove(i);
            }
        }
    }

    pub fn clear_label(&mut self, label: &str) {
        self.samples.retain(|s| s.label != label);
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    /// Best (minimum) distance per label for the given strokes.
    pub fn distances(&self, strokes: &[&Stroke]) -> Vec<(String, f32, usize)> {
        let Some(c) = cloud(&to_raw(strokes)) else { return vec![] };
        let mut best: Vec<(String, f32, usize)> = Vec::new();
        for s in &self.samples {
            let Some(t) = cloud(&s.strokes) else { continue };
            let d = greedy_distance(&c, &t);
            match best.iter_mut().find(|b| b.0 == s.label) {
                Some(b) => b.1 = b.1.min(d),
                None => best.push((s.label.clone(), d, self.count(&s.label))),
            }
        }
        best
    }
}

/// Fuse CNN probabilities (indexed like `labels`) with personal-sample similarity.
/// Labels with >= 3 personal samples get priority (weight 0.8), 1-2 samples a smaller weight.
pub fn fuse(labels: &[String], p_cnn: &[f32], personal: &[(String, f32, usize)]) -> Vec<f32> {
    if personal.is_empty() {
        return p_cnn.to_vec();
    }
    let sims: Vec<(usize, f32, usize)> = personal
        .iter()
        .filter_map(|(l, d, n)| labels.iter().position(|x| x == l).map(|i| (i, (-BETA * (d - D_GOOD).max(0.0)).exp(), *n)))
        .collect();
    if sims.is_empty() {
        return p_cnn.to_vec();
    }
    let sum: f32 = sims.iter().map(|s| s.1).sum();
    let best = sims.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
    let conf = best.1.min(1.0);
    let base_w = if best.2 >= PRIORITY_MIN_SAMPLES { 0.8 } else { 0.4 };
    let w = conf * base_w;
    let mut q = vec![0.0f32; labels.len()];
    for (i, s, _) in &sims {
        q[*i] = s / sum;
    }
    let fused: Vec<f32> = p_cnn.iter().zip(&q).map(|(p, q)| (1.0 - w) * p + w * q).collect();
    let z: f32 = fused.iter().sum();
    fused.into_iter().map(|v| v / z).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::{InkMode, PenKind, Point};

    fn stroke(pts: &[(f32, f32)]) -> Stroke {
        let mut out = Vec::new();
        for w in pts.windows(2) {
            for i in 0..10 {
                let t = i as f32 / 10.0;
                out.push(Point::new(w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t, out.len() as u32));
            }
        }
        out.push(Point::new(pts.last().unwrap().0, pts.last().unwrap().1, out.len() as u32));
        Stroke { id: 0, points: out, color: [255; 4], width: 2.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: 0 }
    }

    fn jitter(pts: &[(f32, f32)], seed: f32) -> Vec<(f32, f32)> {
        pts.iter().enumerate().map(|(i, p)| (p.0 + (i as f32 * 1.7 + seed).sin() * 1.2, p.1 + (i as f32 * 2.3 + seed).cos() * 1.2)).collect()
    }

    const ZIG: [(f32, f32); 5] = [(0.0, 0.0), (30.0, 0.0), (0.0, 30.0), (30.0, 30.0), (30.0, 60.0)];
    const LINE: [(f32, f32); 2] = [(0.0, 0.0), (5.0, 60.0)];

    #[test]
    fn same_shape_is_closer_than_different_shape() {
        let mut store = PersonalStore::default();
        store.add("Z", &[&stroke(&ZIG)]);
        store.add("l", &[&stroke(&LINE)]);
        let d = store.distances(&[&stroke(&jitter(&ZIG, 0.3))]);
        let dz = d.iter().find(|x| x.0 == "Z").unwrap().1;
        let dl = d.iter().find(|x| x.0 == "l").unwrap().1;
        assert!(dz < dl, "{dz} vs {dl}");
    }

    #[test]
    fn invariant_to_scale_and_position() {
        let mut store = PersonalStore::default();
        store.add("Z", &[&stroke(&ZIG)]);
        let big: Vec<(f32, f32)> = ZIG.iter().map(|p| (p.0 * 3.0 + 100.0, p.1 * 3.0 - 40.0)).collect();
        let d = store.distances(&[&stroke(&big)]);
        assert!(d[0].1 < 0.05, "{}", d[0].1);
    }

    #[test]
    fn multi_stroke_samples_work() {
        let mut store = PersonalStore::default();
        let a = [stroke(&[(0.0, 0.0), (40.0, 0.0)]), stroke(&[(0.0, 15.0), (40.0, 15.0)])];
        store.add("=", &[&a[0], &a[1]]);
        store.add("-", &[&stroke(&[(0.0, 0.0), (40.0, 0.0)])]);
        let probe = [stroke(&[(1.0, 1.0), (41.0, 0.0)]), stroke(&[(0.0, 14.0), (40.0, 16.0)])];
        let d = store.distances(&[&probe[0], &probe[1]]);
        let best = d.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert_eq!(best.0, "=");
    }

    #[test]
    fn fusion_gives_priority_after_three_samples_even_if_cnn_disagrees() {
        let labels: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let cnn = [0.9, 0.05, 0.05]; // CNN confidently says "a"
        // one personal sample of "b" matching perfectly: only moderate influence
        let one = fuse(&labels, &cnn, &[("b".into(), 0.1, 1)]);
        assert!(one[0] > one[1], "{one:?}");
        // three samples of "b": personal knowledge wins
        let three = fuse(&labels, &cnn, &[("b".into(), 0.1, 3)]);
        assert!(three[1] > three[0], "{three:?}");
        // a poor personal match barely changes anything
        let poor = fuse(&labels, &cnn, &[("b".into(), 1.2, 5)]);
        assert!(poor[0] > 0.8, "{poor:?}");
        assert!((three.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sample_cap_and_json_roundtrip() {
        let mut store = PersonalStore::default();
        for _ in 0..40 {
            store.add("x", &[&stroke(&LINE)]);
        }
        assert_eq!(store.count("x"), 30);
        let back = PersonalStore::from_json(&store.to_json());
        assert_eq!(back.count("x"), 30);
        assert_eq!(PersonalStore::from_json("not json").count("x"), 0);
    }
}
