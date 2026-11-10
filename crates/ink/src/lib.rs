//! Ink capture model: points, strokes, smoothing, erasing, scribble detection
//! and an undo/redo document.

mod doc;
mod geom;
mod segment;
mod scribble;
mod smooth;

pub use doc::{Document, EraserMode};
pub use geom::{bbox, dist_point_segment, path_length, Rect};
pub use scribble::is_scribble;
pub use segment::{group_symbols, SegmentConfig};
pub use smooth::{chaikin, smooth_stroke, OneEuro, SmoothingConfig};

use serde::{Deserialize, Serialize};

/// One sampled input point. Coordinates are in page space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// 0..=1; 0.5 when the device reports none.
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Milliseconds since stroke start.
    pub t_ms: u32,
}

impl Point {
    pub fn new(x: f32, y: f32, t_ms: u32) -> Self {
        Self { x, y, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, t_ms }
    }
}

/// Whether ink is recognized (Math/Text) or stays as ink (Drawing).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum InkMode {
    #[default]
    MathText,
    Drawing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PenKind {
    #[default]
    Pen,
    /// Translucent, drawn underneath normal ink.
    Highlighter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub points: Vec<Point>,
    pub color: [u8; 4],
    pub width: f32,
    pub kind: PenKind,
    pub mode: InkMode,
    /// Absolute start time (ms, arbitrary epoch) used for temporal grouping.
    #[serde(default)]
    pub t0_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jittery_line() -> Vec<Point> {
        (0..80)
            .map(|i| {
                let jitter = if i % 2 == 0 { 3.0 } else { -3.0 };
                Point::new(i as f32 * 4.0, 100.0 + jitter, i * 8)
            })
            .collect()
    }

    fn roughness(p: &[Point]) -> f32 {
        p.windows(3).map(|w| (w[0].y - 2.0 * w[1].y + w[2].y).abs()).sum::<f32>() / p.len() as f32
    }

    #[test]
    fn smoothing_reduces_tremor() {
        let raw = jittery_line();
        let sm = smooth_stroke(&raw, SmoothingConfig { strength: 0.7 });
        assert!(roughness(&sm) < roughness(&raw) * 0.3, "{} vs {}", roughness(&sm), roughness(&raw));
    }

    #[test]
    fn zero_strength_is_identity_and_endpoints_kept() {
        let raw = jittery_line();
        assert_eq!(smooth_stroke(&raw, SmoothingConfig { strength: 0.0 }), raw);
        let sm = smooth_stroke(&raw, SmoothingConfig { strength: 1.0 });
        assert_eq!(sm.first().unwrap().x, raw.first().unwrap().x);
        assert_eq!(sm.last().unwrap().x, raw.last().unwrap().x);
    }

    #[test]
    fn scribble_detected_but_not_circle_or_line() {
        let zig: Vec<Point> = (0..60)
            .map(|i| {
                let x = if (i / 6) % 2 == 0 { (i % 6) as f32 * 10.0 } else { 50.0 - (i % 6) as f32 * 10.0 };
                Point::new(x, i as f32 * 1.5, i * 8)
            })
            .collect();
        assert!(is_scribble(&zig));
        let circle: Vec<Point> = (0..60)
            .map(|i| {
                let a = i as f32 / 60.0 * std::f32::consts::TAU;
                Point::new(50.0 + 40.0 * a.cos(), 50.0 + 40.0 * a.sin(), i * 8)
            })
            .collect();
        assert!(!is_scribble(&circle));
        let line: Vec<Point> = (0..60).map(|i| Point::new(i as f32 * 3.0, 0.0, i * 8)).collect();
        assert!(!is_scribble(&line));
    }
}
