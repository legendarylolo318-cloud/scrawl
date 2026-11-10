# Changelog

## Unreleased (0.1.0 in progress)

### Added
- Ink canvas (Slint): pen, highlighter, stroke/pixel eraser, smoothing (1€ filter + Chaikin),
  scribble-to-delete, undo/redo, zoom/pan, paper backgrounds (lined, grid, dots, engineering, polar).
- Symbol recognition: 191-class CNN trained in Rust (burn), run with tract; personal `$P+`
  samples fused with the CNN; candidate strip with 1-5 keys; "Train my handwriting" mode.
- Expression tree with Typst, LaTeX and Unicode output; spatial layout parser.
- Typing engine: shorthand, autocomplete, template navigation (library; UI wiring in progress).
- Storage: notebooks/sections/pages as Typst + JSON, atomic writes, trash, history, daily backups,
  per-notebook age encryption, crash recovery, templates.
- Search (tantivy), Typst rendering, export (PDF/PNG/Markdown/Typst/LaTeX/HTML).
- Media: function plotter, image import, PDF import (pure Rust), Opus lecture audio.
- Tablet server (HTTPS/WSS, TLS 1.3, name-constrained private CA, pairing) - off by default.
- `--self-test`, GitHub Actions release workflow (AppImage, Windows zip + installer, SHA256SUMS).

See `REVIEW_NOTES.md` for what is not finished yet.
