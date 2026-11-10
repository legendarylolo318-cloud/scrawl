//! Symbol recognition: stroke rasterization + CNN inference with tract.

mod classifier;
mod raster;

pub use classifier::{Candidate, Classifier, RecogError};
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
