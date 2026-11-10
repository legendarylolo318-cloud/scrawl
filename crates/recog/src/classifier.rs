use crate::raster::{rasterize, Img, SIZE};
use ink::Stroke;
use tract_onnx::prelude::*;

#[derive(Debug)]
pub struct RecogError(pub String);

impl std::fmt::Display for RecogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "recognizer: {}", self.0)
    }
}
impl std::error::Error for RecogError {}

impl From<TractError> for RecogError {
    fn from(e: TractError) -> Self {
        RecogError(e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub label: String,
    /// Softmax probability, 0..=1.
    pub score: f32,
}

pub struct Classifier {
    model: std::sync::Arc<TypedRunnableModel>,
    labels: Vec<String>,
}

impl Classifier {
    pub fn from_bytes(onnx: &[u8], labels_json: &str) -> Result<Self, RecogError> {
        let labels: Vec<String> = serde_json::from_str(labels_json).map_err(|e| RecogError(e.to_string()))?;
        let model = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(onnx))?
            .with_input_fact(0, f32::fact([1, 1, SIZE, SIZE]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { model, labels })
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// Softmax probabilities for all classes.
    pub fn probabilities(&self, img: &Img) -> Result<Vec<f32>, RecogError> {
        let input = tract_ndarray::Array4::from_shape_vec((1, 1, SIZE, SIZE), img.to_vec())
            .map_err(|e| RecogError(e.to_string()))?;
        let out = self.model.run(tvec!(Tensor::from(input).into()))?;
        let logits = out[0].to_plain_array_view::<f32>()?;
        let logits: Vec<f32> = logits.iter().copied().collect();
        if logits.len() != self.labels.len() {
            return Err(RecogError(format!("model has {} outputs, {} labels", logits.len(), self.labels.len())));
        }
        let m = logits.iter().cloned().fold(f32::MIN, f32::max);
        let exps: Vec<f32> = logits.iter().map(|l| (l - m).exp()).collect();
        let sum: f32 = exps.iter().sum();
        Ok(exps.into_iter().map(|e| e / sum).collect())
    }

    pub fn top_k(&self, probs: &[f32], k: usize) -> Vec<Candidate> {
        let mut idx: Vec<usize> = (0..probs.len()).collect();
        idx.sort_by(|&a, &b| probs[b].total_cmp(&probs[a]));
        idx.into_iter().take(k).map(|i| Candidate { label: self.labels[i].clone(), score: probs[i] }).collect()
    }

    /// Classify the strokes of one symbol; returns the top `k` candidates.
    pub fn classify(&self, strokes: &[&Stroke], k: usize) -> Result<Vec<Candidate>, RecogError> {
        let probs = self.probabilities(&rasterize(strokes))?;
        Ok(self.top_k(&probs, k))
    }
}
