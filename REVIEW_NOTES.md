# Review notes (work in progress)

Status: **Phase 1 of 12 complete.** Phases 2–12 are not started; the other crates
(`recog layout input render store search media export remote`) are empty stubs.

## Phase 1 checklist (section 1: Ink canvas)
- [x] Mouse capture, stroke smoothing (1€ filter + Chaikin, adjustable strength)
- [x] Math/Text vs Drawing mode toggle (stored per stroke; no recognition yet)
- [x] Pen, highlighter (drawn under ink), eraser (stroke + pixel modes)
- [x] Zoom (Ctrl+scroll) and pan (scroll), stroke width, 6 preset colors
- [x] Scribble-to-delete, undo/redo (toolbar buttons)
- [ ] Ctrl+Z / Ctrl+Y shortcuts
- [ ] Pen pressure/tilt: model carries them, but Slint's `TouchArea` does not expose
      them; needs a platform-specific input path (winit tablet events or the remote client)
- [ ] Auto-recognize delay slider, recognize-now (button is a stub)
- [ ] Custom colors, 3 favorite pens, lasso, shape snapping, ruler
- [ ] Whole-page re-rasterize on every pointer event (fine for now; needs caching later)

Everything in sections 2–9 is **missing**.
