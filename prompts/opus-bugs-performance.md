# Prompt for Claude Opus: find and fix bugs, then cut CPU / RAM / VRAM use without hurting quality

You are working on **Scrawl** (`/home/lolo/scrawl`, GitHub `legendarylolo318-cloud/scrawl`), a
note-taking app for handwriting, typing and handwritten math, written in **Rust + Slint only**
(workspace crates: `app` (Slint UI), `ink`, `recog`, `layout`, `input`, `render`, `store`,
`search`, `media`, `export`, `remote`, `train`). It runs on Linux (incl. Wayland/Hyprland) and
Windows. The owner has a hand disability and relies on mouse/pen input, so a crash, a lost note or
a laggy pen is a serious bug. Read `HANDOFF.md`, `README.md` and `SECURITY.md` first.

Two jobs, in this order: **(A) bug hunt**, then **(B) performance**. Performance work must not
reduce usability or quality: no lower-quality ink, no less accurate recognition, no blurrier
rendering, no slower response to the pen, no lost features.

## A. Bug hunt (find, reproduce, fix, add a regression test)

Look systematically, not just at obvious spots. For each bug: a failing test or screenshot scene
first, then the fix, then the test passing.

1. **RefCell re-entrancy panics.** Two already happened (Ctrl+Z, and closing dialogs): Rust held
   `State` mutably and called `ui.invoke_*`, whose Slint code called back into Rust and borrowed
   again. Audit *every* `invoke_*`, every Slint callback that calls another callback, every timer
   and the tablet event pump in `crates/app/src/{main,actions,tablet}.rs` and `ui/main.slint`.
   Consider a structural fix (a queue of deferred actions, or `try_borrow` everywhere with a
   retry via `slint::invoke_from_event_loop`) so this class of bug cannot come back.
2. **Panics on user input or bad files.** `unwrap`/`expect`/indexing/slicing on user-controlled
   data: page JSON, settings, `.mat` files, MATLAB scripts, pasted LaTeX, Typst markup, PDFs,
   images, audio, zip backups, clipboard contents, the tablet protocol. Fuzz the parsers with
   `cargo test` property-style loops (no Python). The app must show an error, never crash.
3. **Data safety.** Autosave atomicity (kill the process mid-save on Linux and with a simulated
   failure), crash recovery, external-change reload, trash restore/purge, history restore, backup
   restore (zip-slip already handled), and **encrypted notebooks**: wrong password vs. other
   errors, no plaintext in temp files, recovery files, the search index (memory-only for encrypted
   notebooks), print temp files, logs, or `recent.json`. The scrypt cost was just fixed to a
   fixed `WORK_FACTOR`; confirm old files still open.
4. **Recognition and layout correctness.** Multi-stroke symbols (=, ≠, i, j, ±, x, ∄),
   superscript/subscript placement, ∫ ∑ lim bounds, fractions inside scripts, Typst and LaTeX
   serialization (escaping, nesting, brackets), the LaTeX→Typst paste converter, the Text-mode
   word recognizer (`handwriting.rs`). A drawn "+" is ranked "±" first: find out why (raster
   normalization? training data?) and fix it without hurting other classes.
5. **UI state bugs.** Mode/tool switching, undo/redo across ink + typed canvas text, lasso moves,
   tabs and back/forward after deleting or moving pages, notebook switching with an open dialog,
   narrow-window layout, HiDPI, theme switching while dialogs are open.
6. **Windows-specific.** Paths, the keyring (Credential Manager), the GUI subsystem
   (`windows_subsystem = "windows"` in release: stdout is only visible when redirected), file
   dialogs, clipboard, audio.
7. **Security regression check of the tablet server** (`crates/remote`): run its e2e tests, and
   check nothing you change weakens TLS 1.3-only, origin checks, pairing, revocation or limits.
8. Run `cargo audit` and `cargo deny check` (create `deny.toml`): `HANDOFF.md` lists the current
   advisories (`zip` 0.6.6, `zstd-safe` 5.0.2, `quick-xml` via Typst, `crypto-bigint` via `age`,
   `rustybuzz`/`ttf-parser`, `zbus_names`/`zvariant`). Update what semver allows, otherwise bump
   the parent crate, otherwise document an ignore with a reason. Find which crate pulls the old
   `zip`/`zstd-safe` (`cargo tree -i zip@0.6.6 -e all --target all` printed nothing).

## B. Performance: measure first, then optimize, then measure again

**Measure before touching anything** and keep a table in `docs/performance.md` with before/after
numbers, how you measured, and the machine. Budgets from the original brief: idle RSS under
100 MB with near-zero idle CPU; recognition under 30 ms per symbol; page open under 200 ms;
search under 100 ms on a 1,000-page notebook; model under 5 MB; tablet server under 15 MB extra.

How to measure (Rust/shell only, no Python):
- RSS / CPU: `/proc/<pid>/status` (VmRSS, VmHWM), `ps -o %cpu,rss`, `pidstat` if present,
  `perf stat` / `perf record` or `cargo flamegraph` if available; on Windows use CI logs or
  `Get-Process` in the workflow.
- VRAM: identify which Slint renderer is used at runtime (femtovg/OpenGL, Skia, or software; the
  crate enables `software-renderer-systemfonts`). Measure GPU memory with `nvidia-smi`,
  `radeontop`, `/sys/class/drm/*/device/mem_info_vram_used` or `intel_gpu_top`, whichever exists.
  Compare renderers (`SLINT_BACKEND=winit-software`, `winit-femtovg`, `winit-skia`) for RAM,
  VRAM, CPU and smoothness, and pick a default deliberately (allow overriding in Settings).
- Add a hidden `--bench` mode (or `cargo bench`) that generates a 1,000-page notebook in a temp
  folder (`SCRAWL_NOTES_DIR`) and times page open, search, autosave, recognition, canvas redraw
  with a few thousand strokes, and Typst block rendering. Use it for before/after numbers.

Known suspects to check (verify each with numbers; fix only what the numbers justify):
- **Canvas:** `raster::Canvas` caches the page layer, but every pointer move still copies the
  whole frame (`base.clone()` + a fresh `SharedPixelBuffer`, ~5 MB at 1500×900, 4× on HiDPI) and
  re-renders overlays. Consider drawing only the live stroke's dirty rectangle, reusing buffers,
  or letting Slint draw the live stroke as a `Path` element on top of a static image.
- **Typst rendering:** block renders (`pagedoc::BlockCache`), canvas text (`canvas_texts`,
  cache cleared wholesale at 400 entries), the expression strip (`render::MathCache`, unbounded?)
  and exports. Check cache sizes, memory, and that zooming does not re-render everything.
- **Page refresh:** `refresh_doc` runs on every DOC change and calls `ws.backlinks()`, which
  searches and loads pages each time. `favorites_cache` in `main.rs` loads every page in the
  sidebar the first time. Make these lazy, incremental or cached.
- **Timers:** the 1 s autosave/housekeeping tick (settings comparison and disk fingerprint every
  tick), the 15 ms tablet poll (only while the server runs?), the recognize timer. Idle CPU should
  be ~0 %: verify with the window open and untouched for a minute.
- **Search:** tantivy writer memory (now 1 thread / 15 MB), the persisted index and manifest,
  first-search latency on 1,000 pages.
- **Startup and binary:** model and font loading (Typst fonts are embedded; are fonts loaded twice,
  once for Slint and once for Typst?), lazy init of recognizer, speller (550 KB dictionary) and
  search; binary size (~120 MB) and whether `lto`, `codegen-units = 1`, `opt-level`,
  `panic = "abort"` or dropping unused Typst/Slint features help without breaking anything.
- **Recognition:** tract model optimization, avoid re-classifying unchanged symbols on every
  recognition run.
- **Allocations in hot paths** (pointer move, smoothing, segmentation): avoid per-event Vec clones.

## Hard rules

- **Rust + Slint only. No Python anywhere** (app, scripts, CI, helpers).
- Don't change how the app looks or behaves except to fix bugs; a separate session is redesigning
  the UI, so keep UI edits minimal and coordinate through `HANDOFF.md`.
- Never lose user data; never weaken security or encryption to gain speed.
- Off-screen screenshots for visual checks (no window opens):
  `SCRAWL_NOTES_DIR=/tmp/x SCRAWL_SHOT=out.png SCRAWL_SHOT_SIZE=1500x900 SCRAWL_SHOT_SCENE=draw,undo,pick ./target/release/scrawl`
  (scenes are in `run_scene` in `crates/app/src/main.rs`; add scenes that exercise fixed bugs).
- Before every commit: `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --release`, `./target/release/scrawl --self-test` all pass.
- Commit as `legendarylolo318-cloud <274669742+legendarylolo318-cloud@users.noreply.github.com>`
  (never a personal e-mail), one topic per commit with clear messages. Push to `main` when green;
  CI runs Ubuntu and Windows. Do not tag a release unless the owner asks.

## Done means

Every bug found has a regression test and a fix; `cargo audit` and `cargo deny` pass or have
documented, justified ignores; `docs/performance.md` shows before/after numbers for idle RAM, idle
CPU, VRAM, pen latency/redraw cost, page open, search and startup, all within budget or with a
clear explanation; `HANDOFF.md` updated; and a short summary for the owner: bugs fixed, numbers
gained, anything not verified (be honest, e.g. "not measured on Windows").
