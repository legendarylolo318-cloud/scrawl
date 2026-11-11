# Changelog

## Unreleased: Obsidian features and plugins

### Added
- **Wiki links:** type `[[` for page suggestions; `[[Title]]` becomes a link; a **Linked from** strip lists backlinks.
- **Graph of linked notes** (force-directed, click to open), **Tags** screen, `#tags` typed in a note become page tags.
- **Obsidian vault export and import** (Markdown with front matter, tasks, callouts, wiki links, math, images);
  pasting Markdown keeps its structure. See `docs/obsidian.md`.
- **Plugins:** sandboxed WebAssembly plugins (pure-Rust `wasmi`) and no-code snippet packs, with a Plugins screen.
  See `docs/plugins.md`, `examples/plugins`, and the Plugins section of `SECURITY.md`.

### Fixed (bug check)
- Plugins: a module declaring a huge table (4 billion entries) made Scrawl allocate about 16 GB and freeze;
  tables are now capped at 100 000 entries per table.
- Markdown: text with `==` lost it, a line such as `2024.` came back as an empty numbered item, Typst display
  math was mangled on re-import, URL links with `(` `)` were cut and kept backslashes in their label, a byte-order
  mark hid the front matter, and a long line (100 000 characters) took 50 s to read. All fixed, with tests.
- Vault import: the new notebook no longer gets an extra "Lecture notes" page; `[[Folder/Note]]` and
  `[[Note.md]]` links now find their page; notes left out of very large or deep vaults are reported instead of
  silently dropped. Export avoids Windows-reserved file names (`con`, `aux`, `nul`, `com1`...).
- Typing: `costs $5 or $10` no longer turns `5 or` into math (no space just inside the dollars, `\$` is a
  dollar sign), and `$..$` or `[[` typed inside inline code stay code.
- Markdown found by a new random-editing test (`crates/doc/tests/fuzz_edit.rs`): Markdown starting with a `---`
  rule lost everything up to the next rule (taken for front matter), an empty heading came back as `###`, a
  link with a space at the end of its text glued words together, and italics inside a word (`wo*rd*s`) were
  not read. Typst: indented text such as ` - x` was exported as a bullet.

## 1.1.1 (2026-11-10): better handwriting-to-text

### Changed
- The dot of a handwritten i or j is joined to its stem instead of read as a separate full stop.
- More letter guesses per symbol and a wider dictionary search when reading words.
- Picking a different word on the suggestion chip teaches Scrawl the letters that changed.

## 1.1.0 (2026-11-10): one calm page for typing and handwriting

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
- Handwriting suggestions offer whole-word alternatives in Words mode, and an Auto mode tells words from math
  (As words / As math on the chip to override).
- Find (Ctrl+F) and Replace (Ctrl+H) on the page.

### Fixed
- Undo now undoes whatever changed last, text or ink.
- Empty callout boxes are kept when old pages are converted.
- Export, print and search always include the latest typed text.

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
