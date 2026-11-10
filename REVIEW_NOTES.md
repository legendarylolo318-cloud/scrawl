# Review notes

Status: all 12 phases have working code, but the app is **not feature-complete**: many features
exist as tested libraries without UI yet. Nothing has been run on Windows, no AppImage has been
built, the GitHub Actions workflows have never run, and the tablet client has never been tried on a
real iPad/Android device. Read the "Not verified" and checklist sections before releasing.

## Recognition accuracy (HASYv2)
Measured by `scrawl-train` after 30 epochs (`crates/recog/model/metrics.json`):

| Test set | n | top-1 | top-5 |
|---|---|---|---|
| HASYv2 classification-task fold-1 test, restricted to our 191 classes | 11,593 | **90.5 %** | **99.5 %** |
| Held-out synthetic samples of the 13 font-only classes | 780 | 99.9 % | 100 % |

Caveats: the synthetic number is inflated (same generator as training), so classes with no HASYv2 data
(`t = ( ) ≪ ≫ ∬ ∭ ∯ ″ ᵀ υ ς`) are the least trustworthy until personal samples exist. HASYv2 holds
single images, so accuracy on live multi-stroke ink is only checked with hand-built shapes
(`+ = 0 x`, plus the `--self-test`). Latency 0.94 ms/symbol on CPU (budget 30 ms). Model 2.85 MB (budget 5 MB).

## Deviations from the brief
- The model is trained in **Rust (burn, wgpu)**, not PyTorch/ROCm (requested later in the session). It runs on any GPU via Vulkan or on CPU.
- The app is named **Scrawl** (the brief's "Inkwell" was a placeholder). The repo URL in `Cargo.toml` is `.../scrawl`: confirm it.
- Tablet protocol is JSON over WSS, not a binary format.
- PDF import uses `hayro` (pure Rust). Audio uses `opus-pure` (pure Rust Opus). Neither needs bundled native libraries.

## Not verified
- Windows build, the Inno Setup installer, the Linux AppImage and all GitHub Actions workflows have **never been run**.
- iOS 26 / Android 16 CA-install steps come from web search results, not a device. One report says iOS 26.5.2 rejected a self-made CA; if so the `.mobileconfig` path may need changes.
- Tablet end-to-end latency (< 30 ms) not measured; idle RAM (< 100 MB), tablet-server RAM (< 15 MB) and page-open time (< 200 ms) not measured.
- The tablet CA needs a running OS keyring (Secret Service on Linux). If none is available the server refuses to start (by design: no plaintext key fallback).
- Slint licensing: Slint is GPL-3 / royalty-free (with attribution) / commercial. An MIT app using it under the royalty-free terms must follow those terms; check before release.
- UI is checked visually only, via the off-screen screenshot mode (`SCRAWL_SHOT=out.png SCRAWL_SHOT_SIZE=940x900 SCRAWL_SHOT_SCENE=draw,typing|graph|matlab|tablet|notes scrawl`, software-rendered, no display needed). Layout adapts to the window width (toolbar rows scroll, panels scale, sidebar hides when narrow); checked at 1500, 940 and 620 px wide. Not checked: very small heights, HiDPI scaling, light theme.

## Checklist
Legend: **done** / **partial** (library or subset) / **missing**.

### 1. Ink canvas
- done: mouse/trackpad capture, adjustable smoothing (1€ + Chaikin), Math/Text vs Drawing modes, auto-recognize delay slider + Recognize button, zoom/pan, stroke width, scribble-to-delete, undo/redo (buttons + Ctrl+Z/Y)
- partial: pressure/tilt (stored in points and sent by the tablet client; Slint `TouchArea` exposes none for desktop pens; width is not pressure-varying)

### 2. Recognition
- done: symbol grouping (multi-stroke), CNN via tract, HASYv2 + synthetic + tremor augmentation, licence attribution in README, $P+ personal samples fused with CNN (>= 3 samples = priority), automatic saving of corrections, "Train my handwriting" mode
- partial: candidate strip shows Unicode glyphs (the expression line is Typst-rendered)

### 3. Layout to math
- done: expression tree; Typst + LaTeX + Unicode serialization (compiles in real Typst, tested)
- partial: spatial parser handles scripts, fractions, roots/nth roots, ∑∏∫ bounds, function names; no primes, `lim` bounds, matrices/cases/accents from ink; structure templates only via typing shorthand and tablet buttons (no desktop toolbar); aligned derivations exist in the tree and serializers but have no UI

### 4. Symbol set
- partial: 191 classes cover digits, Latin, Greek (incl. ϵ ε ϑ φ ϕ ħ), operators, relations, calculus, sets/logic, arrows, blackboard bold, brackets; units-upright, drawn accents and a searchable symbol palette are missing

### 5. Typing
- done: keyboard "Type" mode in the app and from the tablet; shorthand (Greek, calculus, arrows, sets, `^ _ /`, sqrt/vec/hat/mat templates, `mat3x3`), Tab between slots, shared cursor with drawn input
- partial: autocomplete + user shorthand table exist in the library but have no popup/settings UI
- missing: source toggle with Typst/LaTeX parsing, IME, selection/clipboard

### 6. Rendering and storage
- done: Typst rendering with content cache, Typst + JSON sidecar storage, atomic writes, autosave (2 s debounce) + crash recovery
- partial: external-change detection (fingerprint in the library, not wired to a reload prompt)

### 7. UI and accessibility
- done: Tokyo Night theme, 40 px hit targets, keyboard shortcuts for undo/redo/candidates
- missing: light theme and theme editor, UI scale, rebindable shortcuts + reference screen, command palette, settings screen (slider values are not persisted)

### 8. Phone/tablet
- done (library + panel): off-by-default HTTPS/WSS server, TLS 1.3 only, name-constrained CA with key in the OS keyring, 30-day leaf certs, one-shot HTTP bootstrap, QR + 6-digit confirmation + Approve, per-device rotating secrets, revoke, 5-failure lockout, strict validation/rate limits, Origin check, CSP/HSTS, idle auto-stop, "running" indicator with Stop, SECURITY.md; 24 automated tests incl. end-to-end over real TLS
- partial: client UI (pen/highlighter/eraser, colour, templates, typing, undo/redo, candidates strip, pen-only toggle, wake lock). Not sent back: the active-block render image (`Frame` message exists, the app never sends it). Guided setup is text in the panel, not a wizard. Leaf-cert auto-renewal logic exists but runs only at server start.

### 9. Notes app
- done: notebooks/pages storage, page templates (blank, lecture, Cornell, problem set, lab report, formula sheet), backgrounds (blank/lined/grid/dots/engineering/polar, spacing), pen/highlighter/eraser, global full-text search (sidebar), export UI (PDF/PNG/Markdown/Typst/LaTeX/HTML), trash (30-day purge in the library), duplicate/history/backups/age encryption (libraries), function plotter, **Desmos-style graph view** (sliders, implicit/parametric/inequalities/points, roots/extrema/intersections), **MATLAB**: `.mat` v5 import and a `.m` subset interpreter (no control flow/functions/cells; v7.3 HDF5 files unsupported)
- partial (library only, no UI): sections beyond the default, tags, favorites/recent, restore from trash, duplicate, version-history timeline, backups scheduling, per-notebook encryption, image import, PDF import and text extraction, audio recording/sync, print, copy-as image/LaTeX/Typst/Unicode, user templates
- missing: drag-and-drop tree, fixed Letter/A4 page mode, lasso, shape snapping, ruler/axes insert, rich-text editing UI (headings, lists, tables, code, callouts), spell check, find/replace, word count, links/backlinks, outline, tabs, split view, back/forward, quick switcher, focus/presentation mode, remembering window size and tabs

## Other known issues
- Typed + drawn mixing: after typing, newly recognized ink is inserted at the cursor, but re-recognition cannot edit previously inserted symbols.
- Graph view colours/labels are fixed; no save of graph state into pages yet.
- The search index is in memory and rebuilt on first search (fast: 0.05 ms/query at 1000 pages, but built per launch).
