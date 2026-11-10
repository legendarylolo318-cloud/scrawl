use crate::{bbox, path_length, Point};

/// Detects a scribble-out gesture: many sharp direction reversals along a
/// path much longer than its bounding box is wide.
pub fn is_scribble(points: &[Point]) -> bool {
    if points.len() < 12 {
        return false;
    }
    let Some(b) = bbox(points) else { return false };
    let diag = b.diagonal();
    if diag < 10.0 {
        return false;
    }
    // Resample by distance to ignore sampling-rate effects.
    let step = (diag / 24.0).max(2.0);
    let mut pts: Vec<(f32, f32)> = vec![(points[0].x, points[0].y)];
    for p in points {
        let l = pts.last().unwrap();
        if (p.x - l.0).hypot(p.y - l.1) >= step {
            pts.push((p.x, p.y));
        }
    }
    let mut reversals = 0;
    for w in pts.windows(3) {
        let (ax, ay) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let (bx, by) = (w[2].0 - w[1].0, w[2].1 - w[1].1);
        let cos = (ax * bx + ay * by) / (ax.hypot(ay) * bx.hypot(by)).max(1e-6);
        if cos < -0.3 {
            reversals += 1;
        }
    }
    reversals >= 4 && path_length(points) > 3.0 * diag
}
