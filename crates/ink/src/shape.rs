//! Shape snapping and lasso selection. Snapping is only ever applied to Drawing-mode ink, so
//! handwritten symbols like O, 0 or ∘ in Math/Text mode are never turned into circles.

use crate::{bbox, path_length, Point};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line,
    Ellipse,
    Rect,
}

fn pt(x: f32, y: f32, t: u32, p: &Point) -> Point {
    Point { x, y, t_ms: t, ..*p }
}

/// If a freehand stroke is clearly a straight line, ellipse or rectangle, return the clean shape.
pub fn snap_shape(points: &[Point]) -> Option<(Shape, Vec<Point>)> {
    if points.len() < 4 {
        return None;
    }
    let r = bbox(points)?;
    let diag = r.diagonal();
    if diag < 20.0 {
        return None; // too small: probably a dot or a symbol
    }
    let len = path_length(points);
    let (a, b) = (points[0], points[points.len() - 1]);
    let chord = (b.x - a.x).hypot(b.y - a.y);
    let t_end = b.t_ms;
    // Straight line: path is barely longer than its chord and every point is close to it.
    if chord > 0.0 && len / chord < 1.06 {
        let max_dev = points.iter().map(|p| crate::dist_point_segment(p.x, p.y, &a, &b)).fold(0.0f32, f32::max);
        if max_dev < chord * 0.04 + 2.0 {
            return Some((Shape::Line, vec![a, pt(b.x, b.y, t_end, &b)]));
        }
    }
    // Closed shapes: the end comes back near the start.
    if chord > diag * 0.2 || len < diag * 1.8 {
        return None;
    }
    let (cx, cy) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    if rx < 5.0 || ry < 5.0 {
        return None;
    }
    // Ellipse fit: normalized radius of every point should be ~1.
    let ell_err = points.iter().map(|p| ((((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2)).sqrt() - 1.0).abs()).sum::<f32>() / points.len() as f32;
    // Rectangle fit: distance to the nearest side, relative to size.
    let rect_err = points.iter().map(|p| (p.x - r.x0).abs().min((p.x - r.x1).abs()).min((p.y - r.y0).abs()).min((p.y - r.y1).abs())).sum::<f32>() / points.len() as f32 / rx.min(ry);
    if rect_err < 0.08 && rect_err < ell_err {
        let c = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1), (r.x0, r.y0)];
        return Some((Shape::Rect, c.iter().enumerate().map(|(i, &(x, y))| pt(x, y, t_end * i as u32 / 4, &a)).collect()));
    }
    if ell_err < 0.09 {
        let n = 64;
        let out = (0..=n)
            .map(|i| {
                let th = i as f32 / n as f32 * std::f32::consts::TAU;
                pt(cx + rx * th.cos(), cy + ry * th.sin(), t_end * i / n, &a)
            })
            .collect();
        return Some((Shape::Ellipse, out));
    }
    None
}

/// Even-odd point-in-polygon test.
pub fn point_in_polygon(x: f32, y: f32, poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi + f32::EPSILON) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Strokes with at least 60 % of their points inside the lasso polygon.
pub fn lasso_select<'a>(strokes: impl IntoIterator<Item = &'a crate::Stroke>, poly: &[(f32, f32)]) -> Vec<u64> {
    strokes
        .into_iter()
        .filter(|s| {
            let n = s.points.len().max(1);
            let inside = s.points.iter().filter(|p| point_in_polygon(p.x, p.y, poly)).count();
            inside * 10 >= n * 6
        })
        .map(|s| s.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(f: impl Fn(f32) -> (f32, f32), n: usize) -> Vec<Point> {
        (0..=n).map(|i| {
            let (x, y) = f(i as f32 / n as f32);
            Point::new(x, y, i as u32 * 10)
        }).collect()
    }

    #[test]
    fn snaps_lines_circles_and_rectangles() {
        let wobbly = pts(|t| (t * 200.0, 50.0 + (t * 40.0).sin() * 1.5), 40);
        assert_eq!(snap_shape(&wobbly).unwrap().0, Shape::Line);
        let circle = pts(|t| (100.0 + 60.0 * (t * std::f32::consts::TAU).cos() + (t * 50.0).sin(), 100.0 + 60.0 * (t * std::f32::consts::TAU).sin()), 60);
        assert_eq!(snap_shape(&circle).unwrap().0, Shape::Ellipse);
        let rect: Vec<Point> = [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0), (0.0, 2.0)]
            .windows(2)
            .flat_map(|w| (0..20).map(move |i| { let t = i as f32 / 20.0; (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t) }))
            .enumerate()
            .map(|(i, (x, y))| Point::new(x, y, i as u32))
            .collect();
        assert_eq!(snap_shape(&rect).unwrap().0, Shape::Rect);
    }

    #[test]
    fn leaves_freehand_and_tiny_strokes_alone() {
        let squiggle = pts(|t| (t * 100.0, (t * 12.0).sin() * 30.0), 60);
        assert!(snap_shape(&squiggle).is_none());
        let small_o = pts(|t| (5.0 * (t * std::f32::consts::TAU).cos(), 5.0 * (t * std::f32::consts::TAU).sin()), 30);
        assert!(snap_shape(&small_o).is_none(), "a small handwritten o is not a circle");
    }

    #[test]
    fn lasso_selects_strokes_inside() {
        let inside = crate::Stroke { id: 1, points: pts(|t| (10.0 + t * 10.0, 10.0), 5), color: [255; 4], width: 2.0, kind: crate::PenKind::Pen, mode: crate::InkMode::MathText, t0_ms: 0 };
        let mut outside = inside.clone();
        outside.id = 2;
        outside.points.iter_mut().for_each(|p| p.x += 500.0);
        let poly = [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)];
        assert_eq!(lasso_select([&inside, &outside], &poly), vec![1]);
    }
}
