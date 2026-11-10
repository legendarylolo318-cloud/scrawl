# Changelog

## UI redesign (unreleased)

### Changed
- **One page that types like a word processor and takes ink.** The side "Page" panel, the typed-text box and
  the floating canvas text boxes are gone from the main screen. Click anywhere in the text and type: caret,
  wrapping, Enter/Backspace paragraphs, arrows, Home/End, Shift and mouse selection, double and triple click,
  Ctrl+B/I/U, cut/copy/paste, undo/redo, lists (`- `, `1. `, `[] `), headings (`# `), quotes, links and inline
  math (`$x^2$`). No markup is shown; Typst stays the storage and export format.
- **Calm layout.** Three toolbar rows (113 buttons, 11 sliders) became a slim top bar (notes, back/forward, title,
  undo/redo, + Insert, Share, ⋯, Find), a formatting bar that only shows while typing, a floating pen tray with
  the pen settings in a pop-over, and a handwriting suggestion chip next to the ink. Narrow windows get a
  slide-over notes drawer.
- Ink draws over the text and follows the paragraph it was written next to. Accepting a handwriting
  suggestion puts typed text or a formula into the page at that spot.
- Lined paper is shown blank in the document view (rules would not line up with text).

### Added
- `notedoc` crate (document model, Typst conversion, editing rules) and `docview` crate (cosmic-text layout,
  hit-testing, rendering). Pages open in the new model automatically; the old version of the page is first saved
  in the page history. Old pages keep every block and canvas text.
- Checklists, outline of headings, word count, today's note, quick note, welcome page for new users,
  selected-object bar for images/plots/math (smaller, larger, trim, copy LaTeX, delete).
- Set `SCRAWL_OLD_EDITOR=1` to start with the previous layout (temporary).

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
- Desmos-style graph view and MATLAB (`.mat` import, `.m` script subset), all in Rust.
- Notes sidebar with templates, autosave, search and export; typing mode; tablet panel.
- `--self-test`, GitHub Actions release workflow (AppImage, Windows zip + installer, SHA256SUMS).

See `REVIEW_NOTES.md` for what is not finished yet.
