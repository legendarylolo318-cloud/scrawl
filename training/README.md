# Training the Inkwell symbol recognizer (pure Rust)

The trainer is the `inkwell-train` crate (`crates/train`), written with
[burn](https://burn.dev). No Python anywhere. It trains a ~0.7 M-parameter CNN
(32x32 input, 191 classes) and writes an ONNX file that the app runs with
`tract-onnx` on the CPU. **End users need no GPU, driver or runtime.**

## Hardware

`--device auto` (default) uses the GPU through wgpu (Vulkan on Linux, DX12 on
Windows): **AMD, NVIDIA and Intel including integrated graphics all work, with no
vendor SDK (no ROCm/CUDA install).** If no usable GPU is found it falls back to the
pure-Rust CPU backend. Force one with `--device gpu|cpu`.
On Arch you only need a Vulkan driver (`vulkan-radeon`, `nvidia-utils`, or
`vulkan-intel`).

## Run

```bash
cargo run --release -p inkwell-train -- download            # fetch + md5-verify HASYv2 into training/data
cargo run --release -p inkwell-train -- train --epochs 30   # writes training/out/{symbols.onnx,labels.json,metrics.json}
cp training/out/symbols.onnx training/out/labels.json crates/recog/model/
```

Training verifies that the exported ONNX reproduces the trained model's logits in
tract before it finishes (and fails otherwise).

## Data and licensing

* **HASYv2** (Thoma, 2017), <https://zenodo.org/records/259444>, ODC Open Database
  License v1.0. Commercial use is allowed; attribution and share-alike (for the
  database itself, not for programs using a model trained on it) apply. We train on the
  official `classification-task/fold-1` train split and report accuracy on its test split.
* **Synthetic data** for classes HASYv2 lacks (rows with an empty second column in
  `crates/train/src/symbols.tsv`) is rendered from locally installed fonts (DejaVu /
  Libertinus, freely licensed) with random warp (tremor), slant, stroke weight and blur.
  Nothing else is downloaded; no non-commercial data is used.
* Online augmentation (`imgops::augment`): rotation, scale, shear, shift, low-frequency
  warp (hand tremor), stroke thickening/thinning.
