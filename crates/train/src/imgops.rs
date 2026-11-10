//! Image helpers shared by data preparation, synthesis and augmentation.
//! `normalize` MUST match `crates/recog/src/raster.rs`.

use crate::rng::Rng;
use image::{imageops, GrayImage, Luma};

pub const SIZE: usize = 32;
const FIT: f32 = 28.0;

/// Crop to the ink bbox, fit longest side to 28 px keeping aspect, centre in 32x32. Ink = 1.
pub fn normalize(img: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; SIZE * SIZE];
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if img[y * w + x] > 0.1 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return out;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut g = GrayImage::new(cw as u32, ch as u32);
    for y in 0..ch {
        for x in 0..cw {
            g.put_pixel(x as u32, y as u32, Luma([(img[(y0 + y) * w + x0 + x].clamp(0.0, 1.0) * 255.0) as u8]));
        }
    }
    let s = FIT / cw.max(ch) as f32;
    let (nw, nh) = (((cw as f32 * s).round() as u32).max(1), ((ch as f32 * s).round() as u32).max(1));
    let r = imageops::resize(&g, nw, nh, imageops::FilterType::Triangle);
    let (ox, oy) = ((SIZE - nw as usize) / 2, (SIZE - nh as usize) / 2);
    for y in 0..nh as usize {
        for x in 0..nw as usize {
            out[(oy + y) * SIZE + ox + x] = r.get_pixel(x as u32, y as u32).0[0] as f32 / 255.0;
        }
    }
    out
}

fn bilinear(src: &[f32], n: usize, x: f32, y: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x > (n - 1) as f32 || y > (n - 1) as f32 {
        return 0.0;
    }
    let (xi, yi) = (x.floor() as usize, y.floor() as usize);
    let (xf, yf) = (x - xi as f32, y - yi as f32);
    let (x1, y1) = ((xi + 1).min(n - 1), (yi + 1).min(n - 1));
    let a = src[yi * n + xi] * (1.0 - xf) + src[yi * n + x1] * xf;
    let b = src[y1 * n + xi] * (1.0 - xf) + src[y1 * n + x1] * xf;
    a * (1.0 - yf) + b * yf
}

/// Random smooth displacement field sampled on a coarse `g`x`g` grid, interpolated bilinearly.
pub struct Warp {
    g: usize,
    dx: Vec<f32>,
    dy: Vec<f32>,
}

impl Warp {
    pub fn random(rng: &mut Rng, g: usize, amp: f32) -> Self {
        let mk = |rng: &mut Rng| (0..g * g).map(|_| rng.range(-amp, amp)).collect::<Vec<_>>();
        let (dx, dy) = (mk(rng), mk(rng));
        Self { g, dx, dy }
    }
    fn at(&self, f: &[f32], u: f32, v: f32) -> f32 {
        // u, v in [0,1]
        let (gx, gy) = (u * (self.g - 1) as f32, v * (self.g - 1) as f32);
        bilinear(f, self.g, gx, gy)
    }
}

/// Resample `src` (n x n) through an affine map (output->source, in pixel coords about the
/// centre) plus an optional displacement warp (in pixels).
pub fn resample(src: &[f32], n: usize, m: [f32; 6], warp: Option<&Warp>) -> Vec<f32> {
    let c = (n as f32 - 1.0) / 2.0;
    let mut out = vec![0.0; n * n];
    for y in 0..n {
        for x in 0..n {
            let (px, py) = (x as f32 - c, y as f32 - c);
            let mut sx = m[0] * px + m[1] * py + m[2] + c;
            let mut sy = m[3] * px + m[4] * py + m[5] + c;
            if let Some(w) = warp {
                let (u, v) = (x as f32 / (n - 1) as f32, y as f32 / (n - 1) as f32);
                sx += w.at(&w.dx, u, v);
                sy += w.at(&w.dy, u, v);
            }
            out[y * n + x] = bilinear(src, n, sx, sy);
        }
    }
    out
}

/// 3x3 max (dilate) or min (erode) filter.
pub fn morph(src: &[f32], n: usize, dilate: bool) -> Vec<f32> {
    let mut out = vec![0.0; n * n];
    for y in 0..n {
        for x in 0..n {
            let mut v = if dilate { f32::MIN } else { f32::MAX };
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                    let s = if xx < 0 || yy < 0 || xx >= n as i32 || yy >= n as i32 { 0.0 } else { src[yy as usize * n + xx as usize] };
                    v = if dilate { v.max(s) } else { v.min(s) };
                }
            }
            out[y * n + x] = v;
        }
    }
    out
}

/// Training-time augmentation: affine jitter, hand-tremor warp, stroke weight change.
pub fn augment(src: &[f32], rng: &mut Rng) -> Vec<f32> {
    let n = SIZE;
    let ang = rng.range(-12.0, 12.0).to_radians();
    let sc = rng.range(0.88, 1.12);
    let sh = rng.range(-0.15, 0.15);
    let (c, s) = (ang.cos() / sc, ang.sin() / sc);
    let m = [c, -s + sh, rng.range(-2.0, 2.0), s, c, rng.range(-2.0, 2.0)];
    let amp = rng.range(0.0, 1.8);
    let warp = Warp::random(rng, 5, amp);
    let mut out = resample(src, n, m, Some(&warp));
    let r = rng.f32();
    if r < 0.25 {
        out = morph(&out, n, true);
    } else if r > 0.85 {
        out = morph(&out, n, false);
    }
    for v in out.iter_mut() {
        *v = v.clamp(0.0, 1.0);
    }
    out
}

pub fn blur(src: &[f32], n: usize, sigma: f32) -> Vec<f32> {
    let mut g = GrayImage::new(n as u32, n as u32);
    for (i, v) in src.iter().enumerate() {
        g.put_pixel((i % n) as u32, (i / n) as u32, Luma([(v.clamp(0.0, 1.0) * 255.0) as u8]));
    }
    let b = imageops::blur(&g, sigma);
    b.pixels().map(|p| p.0[0] as f32 / 255.0).collect()
}
