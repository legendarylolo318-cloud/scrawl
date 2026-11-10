//! Synthesize handwriting-like samples for classes HASYv2 lacks, from system fonts.

use crate::imgops::{blur, morph, normalize, resample, Warp};
use crate::rng::Rng;
use ab_glyph::{point, Font, FontRef, GlyphId, PxScale, ScaleFont};

const FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSerif.ttf",
    "/usr/share/fonts/libertinus/LibertinusMath-Regular.otf",
    "/usr/share/fonts/libertinus/LibertinusSerif-Regular.otf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "C:/Windows/Fonts/seguisym.ttf",
    "C:/Windows/Fonts/arial.ttf",
];
const N: usize = 256;

pub struct Fonts(Vec<Vec<u8>>);

impl Fonts {
    pub fn load() -> Self {
        Self(FONT_PATHS.iter().filter_map(|p| std::fs::read(p).ok()).collect())
    }
}

fn render_glyph(font: &FontRef, ch: char) -> Option<Vec<f32>> {
    if font.glyph_id(ch) == GlyphId(0) {
        return None;
    }
    let scaled = font.as_scaled(PxScale::from(150.0));
    let g = scaled.scaled_glyph(ch);
    let og = font.outline_glyph(g)?;
    let b = og.px_bounds();
    let (w, h) = (b.width(), b.height());
    let (ox, oy) = ((N as f32 - w) / 2.0 - b.min.x, (N as f32 - h) / 2.0 - b.min.y);
    let mut img = vec![0.0f32; N * N];
    og.draw(|x, y, c| {
        let (px, py) = (x as f32 + b.min.x + ox, y as f32 + b.min.y + oy);
        if px >= 0.0 && py >= 0.0 && (px as usize) < N && (py as usize) < N {
            img[py as usize * N + px as usize] = c;
        }
    });
    let _ = point(0.0, 0.0);
    Some(img)
}

/// Returns `count` normalized 32x32 samples (flattened) for `label`, or None if no font has it.
pub fn render_class(fonts: &Fonts, label: &str, count: usize, seed: u64) -> Option<Vec<Vec<f32>>> {
    let glyph = match label {
        "ᵀ" => 'T',
        l => l.chars().next()?,
    };
    let refs: Vec<FontRef> = fonts.0.iter().filter_map(|b| FontRef::try_from_slice(b).ok()).collect();
    let bases: Vec<Vec<f32>> = refs.iter().filter_map(|f| render_glyph(f, glyph)).collect();
    if bases.is_empty() {
        return None;
    }
    let mut rng = Rng::new(seed);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let base = &bases[rng.below(bases.len())];
        let sh = rng.range(-0.25, 0.25);
        let m = [1.0, sh, -sh * 128.0, 0.0, 1.0, 0.0];
        let amp = rng.range(2.0, 9.0);
        let warp = Warp::random(&mut rng, 6, amp);
        let mut img = resample(base, N, m, Some(&warp));
        for _ in 0..rng.below(4) {
            img = morph(&img, N, true);
        }
        let img = blur(&img, N, rng.range(0.5, 2.0));
        let img: Vec<f32> = img.iter().map(|v| ((v - 0.3) * 2.5).clamp(0.0, 1.0)).collect();
        out.push(normalize(&img, N, N));
    }
    Some(out)
}
