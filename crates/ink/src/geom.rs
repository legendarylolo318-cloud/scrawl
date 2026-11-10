use crate::Point;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn diagonal(&self) -> f32 {
        self.width().hypot(self.height())
    }
}

pub fn bbox(points: &[Point]) -> Option<Rect> {
    let first = points.first()?;
    let mut r = Rect { x0: first.x, y0: first.y, x1: first.x, y1: first.y };
    for p in points {
        r.x0 = r.x0.min(p.x);
        r.y0 = r.y0.min(p.y);
        r.x1 = r.x1.max(p.x);
        r.y1 = r.y1.max(p.y);
    }
    Some(r)
}

pub fn path_length(points: &[Point]) -> f32 {
    points.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum()
}

pub fn dist_point_segment(px: f32, py: f32, a: &Point, b: &Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - a.x) * dx + (py - a.y) * dy) / len2).clamp(0.0, 1.0) };
    (px - (a.x + t * dx)).hypot(py - (a.y + t * dy))
}
