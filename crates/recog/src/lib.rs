//! Symbol recognition: stroke rasterization + CNN inference with tract.

mod classifier;
mod personal;
mod raster;

pub use classifier::{Candidate, Classifier, RecogError};
pub use personal::{fuse, PersonalStore, Sample, PRIORITY_MIN_SAMPLES};
pub use raster::{rasterize, Img, SIZE};

/// Model and labels produced by `training/train.py` and committed under `model/`.
pub const BUNDLED_MODEL: &[u8] = include_bytes!("../model/symbols.onnx");
pub const BUNDLED_LABELS: &str = include_str!("../model/labels.json");

impl Classifier {
    /// The classifier shipped inside the binary.
    pub fn bundled() -> Result<Self, RecogError> {
        Self::from_bytes(BUNDLED_MODEL, BUNDLED_LABELS)
    }
}

/// Everything the app needs: CNN + personal samples + fusion + corrections.
pub struct Recognizer {
    pub cnn: Classifier,
    pub personal: PersonalStore,
}

impl Recognizer {
    pub fn new(personal: PersonalStore) -> Result<Self, RecogError> {
        Ok(Self { cnn: Classifier::bundled()?, personal })
    }

    /// Top-`k` candidates for one symbol (CNN fused with personal samples).
    pub fn classify(&self, strokes: &[&ink::Stroke], k: usize) -> Result<Vec<Candidate>, RecogError> {
        let probs = self.cnn.probabilities(&rasterize(strokes))?;
        let dists = self.personal.distances(strokes);
        let fused = fuse(self.cnn.labels(), &probs, &dists);
        Ok(self.cnn.top_k(&fused, k))
    }

    /// The user picked `label` for these strokes (candidate click, number key, or training mode):
    /// remember them as a personal sample.
    pub fn learn(&mut self, label: &str, strokes: &[&ink::Stroke]) {
        self.personal.add(label, strokes);
    }
}
