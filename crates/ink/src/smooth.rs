use crate::Point;

/// Adjustable smoothing. `strength` in 0..=1 (0 = raw input).
#[derive(Clone, Copy, Debug)]
pub struct SmoothingConfig {
    pub strength: f32,
}

impl Default for SmoothingConfig {
    fn default() -> Self {
        Self { strength: 0.5 }
    }
}

/// 1€ filter (Casiez et al. 2012) for a single scalar channel.
#[derive(Clone, Debug)]
pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    d_cutoff: f32,
    prev: Option<(f32, f32)>, // (filtered x, filtered dx)
}

fn alpha(cutoff: f32, dt: f32) -> f32 {
    let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

impl OneEuro {
    pub fn new(min_cutoff: f32, beta: f32) -> Self {
        Self { min_cutoff, beta, d_cutoff: 1.0, prev: None }
    }

    /// `dt` in seconds (clamped to avoid division by zero).
    pub fn filter(&mut self, x: f32, dt: f32) -> f32 {
        let dt = dt.max(1e-3);
        let Some((px, pdx)) = self.prev else {
            self.prev = Some((x, 0.0));
            return x;
        };
        let dx = (x - px) / dt;
        let dx_hat = pdx + alpha(self.d_cutoff, dt) * (dx - pdx);
        let cutoff = self.min_cutoff + self.beta * dx_hat.abs();
        let x_hat = px + alpha(cutoff, dt) * (x - px);
        self.prev = Some((x_hat, dx_hat));
        x_hat
    }
}

/// One Chaikin corner-cutting pass; keeps endpoints.
pub fn chaikin(points: &[Point]) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let lerp = |a: &Point, b: &Point, t: f32| Point {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
        pressure: a.pressure + (b.pressure - a.pressure) * t,
        tilt_x: a.tilt_x + (b.tilt_x - a.tilt_x) * t,
        tilt_y: a.tilt_y + (b.tilt_y - a.tilt_y) * t,
        t_ms: (a.t_ms as f32 + (b.t_ms as f32 - a.t_ms as f32) * t) as u32,
    };
    let mut out = Vec::with_capacity(points.len() * 2);
    out.push(points[0]);
    for w in points.windows(2) {
        out.push(lerp(&w[0], &w[1], 0.25));
        out.push(lerp(&w[0], &w[1], 0.75));
    }
    out.push(*points.last().unwrap());
    out
}

/// Smooth a finished stroke: 1€ filter (offline, in time order) then
/// 0–3 Chaikin passes depending on strength.
pub fn smooth_stroke(points: &[Point], cfg: SmoothingConfig) -> Vec<Point> {
    let s = cfg.strength.clamp(0.0, 1.0);
    if s <= 0.0 || points.len() < 3 {
        return points.to_vec();
    }
    // Strong smoothing => low cutoff. 0.1 .. 3.0 Hz-ish range, log-mapped.
    let min_cutoff = 3.0 * (0.1f32 / 3.0).powf(s);
    let (mut fx, mut fy) = (OneEuro::new(min_cutoff, 0.02), OneEuro::new(min_cutoff, 0.02));
    let mut out = Vec::with_capacity(points.len());
    let mut last_t = points[0].t_ms;
    for (i, p) in points.iter().enumerate() {
        let dt = (p.t_ms.saturating_sub(last_t)) as f32 / 1000.0;
        last_t = p.t_ms;
        let mut q = *p;
        q.x = fx.filter(p.x, if i == 0 { 0.016 } else { dt.max(0.004) });
        q.y = fy.filter(p.y, if i == 0 { 0.016 } else { dt.max(0.004) });
        out.push(q);
    }
    // Keep the exact final point so strokes end where the pen lifted.
    if let (Some(o), Some(p)) = (out.last_mut(), points.last()) {
        o.x = p.x;
        o.y = p.y;
    }
    let passes = (s * 3.0).round() as usize;
    for _ in 0..passes {
        out = chaikin(&out);
    }
    out
}
