# Prompt for Claude Sonnet: make Scrawl a calm, intuitive note-taking app (keep every feature)

You are working on **Scrawl** (`/home/lolo/scrawl`, GitHub `legendarylolo318-cloud/scrawl`), a
note-taking app written in **Rust + Slint only**. It started as a handwritten-math app; it must now
feel first and foremost like a **general note-taking app**, as calm and obvious as Google Docs,
Apple Notes, OneNote or GoodNotes, that is *also* great at handwriting and handwritten math.

The owner has a hand disability: they use a **mouse or pen most of the time** and type sometimes.
Everything must stay reachable by mouse/pen alone, with big forgiving targets (at least 40 px), and
also by keyboard. Treat anything confusing or fiddly as a real bug.

## What the owner asked for, in their words (all of it is required)

1. "It should be super easy and intuitive. Keep all features, but right now all the stuff is
   overwhelming." Today there are three rows of toolbars and ~113 buttons in
   `crates/app/ui/main.slint`, sliders always on screen, a candidate strip, tabs, a sidebar and a
   separate "Page" panel, all visible at once.
2. "It should just be a general note-taking app."
3. "The way text is inserted is weird: it should act like Google Docs."
4. "Make sure people can still draw stuff in." Drawing and handwriting must stay first-class.
5. Investigate good UI choices and maybe some new features, then implement them.

## Why text feels weird today (read the code to confirm)

There are three unrelated ways to put text on a page, and none of them is a normal document:
- **Text blocks** typed into a separate `TextEdit` box at the bottom of the right-hand panel,
  written as Typst markup (`= Heading`, `*bold*`), then shown as rendered images in a list
  (`pagedoc.rs`, `editor_commit` in `actions.rs`).
- **Canvas text** ("Type here" / just start typing): one-line `LineEdit` boxes that float on the
  ink canvas and become images (`textbox_*` in `actions.rs`, `canvas_texts` in `main.rs`).
- **Typed math** in a separate typing mode with a shorthand editor (`input` crate).
Ink, typed text and typeset math live in different places (canvas vs. side panel).

## Target: one page that behaves like Google Docs *and* takes ink

Design (then build) a single page view where:
- **Typing works like a word processor.** Click anywhere in the text and a blinking caret appears
  there. Typing flows and wraps; Enter makes a new paragraph; Backspace joins paragraphs; arrow
  keys, Home/End, Shift-selection, mouse drag-selection, double-click (word) and triple-click
  (paragraph) select; Ctrl+B / Ctrl+I / Ctrl+U and a small formatting bar work on the selection;
  headings, bullet and numbered lists (typing "- " or "1. " starts one), checklists, links, inline
  math (typing `$...$` or a button), cut/copy/paste of plain text, and undo/redo per edit. No markup
  syntax visible to the user (Typst stays the *storage and render* format behind the scenes).
- **Drawing works on the same page.** A pen/highlighter/eraser/lasso tray (floating, collapsible,
  near the page) lets you write or sketch anywhere on top of the text, like OneNote/GoodNotes.
  Ink is a layer over the document that scrolls with it (anchor strokes to the paragraph they
  were drawn next to, so they stay next to the right text when earlier paragraphs grow). Keep
  every ink feature: smoothing, pressure, highlighter, pixel/stroke eraser, lasso move, shape
  snapping in Drawing mode, scribble-to-delete, favourite pens, custom colours.
- **Handwriting turns into text or math in place.** Write words or math with the pen, pause, and a
  small, non-modal suggestion chip appears next to the ink ("brandy" / typeset formula) with
  Accept, Keep as ink and alternatives. Accepting inserts the text or an inline/display math object
  into the document at that spot. Keep the candidate learning (personal samples) and the Math /
  Text / Drawing modes, but make the mode obvious and usually automatic (suggest, don't force).
- **Math objects** are part of the document: click one to edit it (by handwriting, the symbol
  palette or typing shorthand), with copy as LaTeX/Typst.
- **Images, PDFs, plots, callouts, tables, rules, graphs and audio** are inserted from one "+"
  menu at the caret and behave like objects in Google Docs (select, resize handles, delete).

Slint has no rich-text editor widget, so the document editor must be built in Rust: a document
model (paragraphs of styled runs plus inline objects), a layout pass (Typst can lay out and give
glyph/line positions, or lay out with your own measurement), caret/selection hit-testing, and
rendering of visible paragraphs only (cache per paragraph). Plan this carefully; it is the core of
the work. Migrate existing pages: today's `Block::Text` markup, `Block::Math`, other blocks and
`page.texts` canvas text must load into the new model without losing anything (write a migration
with tests), and Typst/LaTeX/Markdown/HTML/PDF export must keep working.

## Calm layout (progressive disclosure, nothing removed)

- One slim top bar with only essentials: sidebar toggle, back/forward, page title, undo/redo,
  "+" insert menu, share/export, and a "…" menu for the rest (history, template, duplicate,
  print, page size, paper, delete). Command palette button and Ctrl+K stay as "find anything".
- Contextual UI only when relevant: formatting bar when text is selected or the caret is in text;
  image handles only on a selected image; math candidates only after recognition; sliders
  (width, smoothing, recognition delay) inside the pen tray's popover, not always on screen.
- Sidebar: a clean notebook → section → page tree with search at the top, drag to reorder, and
  favourites/recent as simple filters. Trash and history are reachable but not shouting.
- Graph view, MATLAB, tablet pairing, training, backups and rebinding stay, in menus and Settings.
- First launch: a friendly welcome note that shows how to type, how to draw, and how handwriting
  becomes text or math, in a few lines.
- Plain-language labels; icons only where the meaning is universal, always with a tooltip/label.
- Must work at full screen, at a half-width tile (~940 px), down to ~480 px wide, short windows
  and HiDPI, in dark, light, high-contrast and custom themes, with UI scale.

## New features (only cheap, clearly useful ones; justify each)

Ideas to evaluate: checklists, pinned notes, quick capture of a new note, a "today" note, page
colours or emoji, sorting the page list, word count on demand, outline/table of contents for long
notes, back-links shown under the page, keyboard-free formatting via the selection bar.

## Phase 1: investigate and write the plan first

1. Read `HANDOFF.md`, `README.md`, `SECURITY.md`, and the app crate: `crates/app/ui/main.slint`,
   `crates/app/src/{main,actions,pagedoc,commands,settings,workspace,raster,session,handwriting}.rs`,
   plus `crates/store/src/model.rs` (page format) and `crates/export`.
2. Screenshot every screen off-screen (no window opens, the real desktop is never touched):
   ```bash
   cargo build --release -p scrawl
   SCRAWL_NOTES_DIR=/tmp/scrawl-shot SCRAWL_SHOT=out.png SCRAWL_SHOT_SIZE=1500x900 \
     SCRAWL_SHOT_SCENE=draw,insert,text ./target/release/scrawl
   ```
   Scenes (comma-separated, see `run_scene` in `main.rs`): `draw insert undo text typing justtype
   typehere graph matlab tablet notes palette pick symbols settings shortcuts focus lasso nodoc`.
   Variables: `SCRAWL_SHOT_THEME=light`, `SCRAWL_SHOT_SCALE=2`. Always use a temp
   `SCRAWL_NOTES_DIR`. Add scenes for every new interaction (typing paragraphs, selection,
   formatting, drawing over text, accepting a handwriting suggestion).
3. Write `docs/ui-redesign.md`: problems found (with screenshots), the target layout as ASCII
   sketches for wide and narrow windows, where every existing command ends up (use
   `crates/app/src/commands.rs` as the checklist), the document-editor architecture and data
   migration, the new features you will add and what you leave out, and the order you will build it.

## Phase 2: build it in safe steps

Build in small, always-working commits: document model + migration + tests first, then rendering
and caret, then editing and selection, then ink on the document, then handwriting suggestions,
then the calm layout and menus, then new features. After each step re-take the screenshots,
look at them, and fix anything cramped, clipped, overlapping or unclear.

## Hard rules

- **Rust + Slint only. No Python anywhere** (app, scripts, CI, or your own helpers: use Rust,
  cargo and shell).
- **Slint re-entrancy gotcha (caused two crashes already):** never call a Slint callback/function
  from Rust (`ui.invoke_*`) while `State` is mutably borrowed if that Slint code calls back into
  Rust; it re-borrows the `RefCell` and panics. Use `try_borrow` or set properties directly (see
  `close_overlay` in `actions.rs`, the `pen_lifted` handler in `main.rs`).
- Never lose user data: autosave, crash recovery, history, trash, encryption and export must keep
  working; test the migration on pages created by the current version.
- Before every commit: `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --release`, `./target/release/scrawl --self-test` all pass.
- Commit as `legendarylolo318-cloud <274669742+legendarylolo318-cloud@users.noreply.github.com>`
  (never a personal e-mail). Push to `main` when green. Do not tag a release unless asked.
- The screenshot mode cannot test real dragging, hover or a live caret blink: say what you could
  not verify, and give the owner a short manual test list.
- Update `HANDOFF.md`, `README.md` (new screenshots in `docs/screenshots/`) and `CHANGELOG.md`.

## Done means

Typing behaves like Google Docs, drawing and handwriting work on the same page, the main screen
is calm at 1500 / 940 / 620 px in dark and light themes, every old feature is still reachable,
old notes open unchanged, the chosen new features work, all checks are green, and the owner gets
a short summary of what moved where plus a manual test checklist.
