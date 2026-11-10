# Review notes (work in progress)

Status: phases 1-2 complete; groundwork for phases 4-5 (`layout`, `input` crates) in place.
Not started: phases 3 (UI half), 6-12. Crates `render store search media export remote` are stubs.

## Recognition accuracy (HASYv2)
Measured by `inkwell-train` after 30 epochs (see `crates/recog/model/metrics.json`):

| Test set | n | top-1 | top-5 |
|---|---|---|---|
| HASYv2 classification-task fold-1 test, restricted to our 191 classes | 11,593 | **90.5 %** | **99.5 %** |
| Held-out synthetic samples of the 13 font-only classes | 780 | 99.9 % | 100 % |

Caveats: the synthetic number is inflated (same generator as training, not real handwriting),
so classes without HASYv2 data (`t = ( ) ≪ ≫ ∬ ∭ ∯ ″ ᵀ υ ς`) are the least trustworthy until
personal samples (phase 3) exist. HASYv2 is single-stroke-rendered images, so accuracy on
live drawn strokes (esp. multi-stroke) is unmeasured beyond a few hand-built smoke tests.
Latency: 0.94 ms/symbol on CPU (budget 30 ms). Model: 2.85 MB (budget 5 MB).

## Checklist
### 1. Ink canvas
- [x] mouse capture, smoothing (1€ + Chaikin), Math/Text vs Drawing mode, pen/highlighter/eraser
- [x] zoom/pan, stroke width, scribble-delete, undo/redo buttons
- [x] auto-recognize delay slider (200-2000 ms) + recognize-now (result shown as status text)
- [ ] pressure/tilt: model has them but Slint `TouchArea` does not expose them
- [x] Ctrl+Z / Ctrl+Y shortcuts (rebindable shortcuts screen is missing)
- [ ] whole page is re-rasterized per pointer event (needs caching)
### 2. Recognition
- [x] stroke grouping into symbols (`ink::group_symbols`, multi-stroke tested)
- [x] CNN trained in **Rust (burn, wgpu: AMD/NVIDIA/Intel iGPU or CPU)**, exported to ONNX, run with tract
- [x] HASYv2 (ODbL, attributed in training/README.md) + font-synthesized data with tremor augmentation
- [ ] HASYv2 license/attribution in top-level README (README not written yet)
- [x] $P+ personal recognizer fused with CNN (>=3 samples = priority), corrections auto-saved to `~/.config/inkwell/personal.json` (atomic write), "Train" mode, candidate strip with 1-5 keys + click (tested headlessly in `app::session`)
- [ ] candidate strip shows unicode glyphs in a Slint `Text`, not Typst-rendered (needs phase 6 renderer)
- [ ] function-name recognition from drawn letter sequences: done in `layout::parse` (sin, cos, ...), not wired to UI
### 3. Layout / expression tree
- [x] `Expr` tree, Typst + LaTeX serializers (tested)
- [x] spatial parse: scripts, fractions, sqrt/nth root, sum/prod/integral bounds, function names
- [ ] primes, limits above `lim`, matrices/cases/accents from ink, aligned-derivation UI, template toolbar - **missing**
### 5. Typing
- [x] `input::Editor`: cursor tree editor, shorthand (Greek, calculus, arrows, sets, RR..), `^ _ /`, sqrt/vec/hat/mat templates, Tab navigation, fuzzy autocomplete, user shorthand table
- [ ] UI integration, source-toggle parsers (Typst/LaTeX -> tree), IME, selection/clipboard - **missing**
### 4, 6-9 and the rest
- missing (not started)
