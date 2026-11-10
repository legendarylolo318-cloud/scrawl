//! Image import: decode, resize, crop. (Annotation ink is stored separately as strokes on the page.)

use image::{imageops, DynamicImage, ImageFormat, RgbaImage};

pub struct Img(pub RgbaImage);

#[derive(Debug)]
pub struct ImgError(pub String);

impl Img {
    /// Decode PNG / JPEG / WebP / GIF bytes (clipboard paste, drag-and-drop or file picker).
    pub fn decode(bytes: &[u8]) -> Result<Self, ImgError> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(ImgError("image too large".into()));
        }
        image::load_from_memory(bytes).map(|d| Img(d.to_rgba8())).map_err(|e| ImgError(e.to_string()))
    }

    pub fn size(&self) -> (u32, u32) {
        self.0.dimensions()
    }

    /// Fit inside `max_w` x `max_h` keeping aspect ratio (never upscales).
    pub fn fit(&self, max_w: u32, max_h: u32) -> Img {
        let (w, h) = self.size();
        let s = (max_w as f32 / w as f32).min(max_h as f32 / h as f32).min(1.0);
        let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
        Img(imageops::resize(&self.0, nw, nh, imageops::FilterType::Lanczos3))
    }

    /// Crop to the rectangle (clamped to the image).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Img {
        let (iw, ih) = self.size();
        let (x, y) = (x.min(iw.saturating_sub(1)), y.min(ih.saturating_sub(1)));
        let (w, h) = (w.min(iw - x).max(1), h.min(ih - y).max(1));
        Img(imageops::crop_imm(&self.0, x, y, w, h).to_image())
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(self.0.clone()).write_to(&mut out, ImageFormat::Png).expect("png encode");
        out.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Img {
        let mut i = RgbaImage::new(40, 20);
        for (x, _, p) in i.enumerate_pixels_mut() {
            *p = image::Rgba([x as u8 * 6, 10, 200, 255]);
        }
        Img(i)
    }

    #[test]
    fn decode_resize_crop_roundtrip() {
        let png = sample().to_png();
        let d = Img::decode(&png).unwrap();
        assert_eq!(d.size(), (40, 20));
        assert_eq!(d.fit(20, 20).size(), (20, 10));
        assert_eq!(d.fit(400, 400).size(), (40, 20), "never upscales");
        let c = d.crop(10, 5, 100, 100);
        assert_eq!(c.size(), (30, 15), "clamped to the image");
        assert!(Img::decode(b"not an image").is_err());
    }
}
