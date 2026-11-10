# Scrawl UI redesign: one calm page that types like Google Docs and takes ink

Status: **Phase 1 (investigation and plan). No application code has been changed yet.**
Everything below is the plan I will build in Phase 2, in the order given at the end.
Source brief: `prompts/sonnet-ui-redesign.md`. Hard rules that apply to every step: Rust + Slint only (no
Python), mouse/pen first with 40 px minimum targets, never lose user data, clippy `-D warnings` + full tests +
`--self-test` before every commit, no release tags.

## 1. Problems found

Screenshots were taken off-screen with `SCRAWL_SHOT_*` (no window opened) and are kept in
`docs/screenshots/before-*.png`.

### 1.1 Too much at once

On first launch at 1500 px wide the user sees **three toolbar rows, a candidate strip, a sidebar and a
"Page" panel together**. The code has 113 `ToolButton`s and 11 sliders in `crates/app/ui/main.slint`,
88 commands in `commands.rs`.

| Where | What is always on screen | Screenshot |
|---|---|---|
| Row 1 | Commands, Pen, Highlight, Eraser, Type here, Lasso, Math, Text, Drawing, Undo, Redo, Recognize, Insert, Type, Symbols, Train, Clear (17 buttons, no grouping) | `before-wide-draw.png` |
| Row 2 | Notes, back, forward, title box, star, template combo, + Page, Page, export combo, Export, Graph, Record, Tablet, Settings, collapse arrow (15 controls, two of them dropdowns) | same |
| Row 3 | 6 colour dots, + Colour, 3 favourite pens, Smooth, Width, Recognize-after (three sliders with − / + buttons), Paper, Spacing, Size | same |
| Strip | "Candidates" label and an empty row (always takes 64 px) | same |
| Left | Notebook combo, +, search box, filter combo, + Section, Trash, History, an instruction sentence, then the tree | same |
| Right | "Page" panel: typeset blocks, 10 formatting buttons, a grey 180 px text box, "Add text block" | same |

Specific bugs and confusions:

1. **Controls that mean nothing to a newcomer**: "Recognize", "Insert ↵", "Type", "Train", "Clear",
   "Math / Text / Drawing", "Recognize after 600 ms", "Paper: Lined", "Size: Infinite". Two different
   things are called "Type" / "Type here".
2. **Three unrelated ways to put text on a page** (see 1.2), and ink, text and math live in two different
   places (canvas vs. right panel). The right panel is not the page: you cannot click a word and type.
3. **Sidebar controls shout, content whispers**: the explanation sentence "Drag pages to reorder ... ▲▼ also
   work" and the ▲/▼ buttons on every selected row are bigger than the page names. (`before-wide-draw.png`)
4. **Narrow widths break**: at 940 px the third toolbar and the Export/Graph/… buttons are cut off at the
   edge and need a sideways scroll; the text panel shows "Table" half clipped and the page list header is
   cut ("Hist"). At 480 px (`before-tiny.png`) three toolbar rows plus the candidate strip use 220 px of an
   800 px window, the sidebar disappears entirely and the canvas is the only content.
5. **Light theme** (`before-light.png`) works but the editor box is dark-grey-on-white with very low contrast
   on the placeholder text.
6. **Candidates are a modal-like strip far from the ink** (`before-typehere.png`: the "lo" candidate sits in
   row 4, the ink is 400 px lower). You have to look away from your hand to accept.
7. Sliders always visible even though they are set once per session.
8. The state flags `show-doc`, `doc-on-narrow`, `show-sliders`, `show-tablet`, `graph-mode`, `focus-mode`,
   `present-mode`, `split-active`, `typing`, `training` make the layout a tangle of `if`s in one 1100-line
   file (hard to keep consistent, hence the clipped-layout bugs above).

### 1.2 Why text feels weird (confirmed in the code)

| Path | Code | Behaviour | Problem |
|---|---|---|---|
| Text block | `editor-text` TextEdit in `main.slint`, `editor_commit` in `actions.rs`, `Block::Text{markup}` in `store/model.rs`, rendered by `pagedoc.rs::BlockCache` (Typst → PNG) | You write **Typst markup** (`= Heading`, `*bold*`) in a grey box at the bottom of a side panel, press "Add text block", and an *image* of it appears above | Markup is visible, you cannot edit in place, no caret in the rendered text, each block is a picture |
| Canvas text | `textbox_*` in `actions.rs`, `Page.texts: Vec<CanvasText>`, `canvas_texts` in `main.rs` | One-line `LineEdit` floats over the ink canvas, becomes an image at fixed page x/y | No wrapping, no paragraphs, can overlap ink, not part of the document, not exported with the text |
| Typed math | `input::Editor` ("Type" mode) | Shorthand editor with its own strip | Third mental model |

Ink strokes are in `Page.strokes` in canvas coordinates, blocks in `Page.blocks` in list order: nothing links
a stroke to the text it was written next to.

## 2. Target design

One **page** in the middle. Everything else is out of the way until needed (progressive disclosure).

### 2.1 Wide window (≥ 1100 px), default state

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│ [☰] [◀][▶]   Lecture 4 – Chain rule  ☆                 [↶][↷] [＋ Insert ▾] [Share ▾] [⋯] [⌕]  │  top bar, 56 px
├───────────────┬──────────────────────────────────────────────────────────────────────────────┤
│ 🔍 Search     │  (formatting bar appears here only while the caret is in text)               │
│ ▾ My notebook │   [Normal ▾] [B][I][U] [•][1.][☑] [$x$][🔗]                                     │
│  ▾ Calculus   │ ┌──────────────────────────────────────────────────────────────────────────┐ │
│    Lecture 4 ◂│ │ Chain rule                                          ← heading            │ │
│    Limits     │ │ If f and g are differentiable then ▌                ← caret, wraps       │ │
│  ▾ Physics    │ │  • the derivative of f(g(x)) is  f'(g(x))·g'(x)     ← inline math        │ │
│    ...        │ │      ∫₀¹ x² dx = 1/3                                ← display math       │ │
│ ─────────     │ │  ✎ hand-written ink floats here, scrolls with the text                   │ │
│ Pinned        │ │                                   ┌───────────────┐                      │ │
│ Recent        │ │   brandy ✎                        │ “brandy”  ✓ ✎ │ ← suggestion chip    │ │
│ Trash         │ └──────────────────────────────────────────────────────────────────────────┘ │
│ ＋ New page   │                          ┌─ pen tray (floating, collapsible) ─┐              │
│               │                          │ ✎ ▮ ⌫ ◌   ● ● ● ●  ⚙             │              │
│               │                          └────────────────────────────────────┘              │
├───────────────┴──────────────────────────────────────────────────────────────────────────────┤
│ 312 words · saved                                                                            │  status 28 px
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

Top bar (the only always-visible chrome): sidebar toggle, back/forward, **editable page title**, favourite
star, undo/redo, **＋ Insert** menu, **Share** (export formats, print, copy as Typst/LaTeX), **⋯** (everything
else), and the Ctrl+K search button.

### 2.2 Half-width tile (~940 px) and narrow (~620 / 480 px)

```
940 px:  sidebar becomes a slide-over drawer (☰); top bar keeps title, undo/redo, ＋, ⋯;
         the page uses the full width; Share moves into ⋯.
620 px:  top bar:  [☰] [Title........] [↶][↷][＋][⋯]       (back/forward/star/search go to ⋯)
         pen tray is a single round ✎ button at the bottom right that opens the tray as a sheet.
480 px:  same as 620, page margins shrink to 12 px, formatting bar scrolls sideways (one row, 40 px).
short windows (<560 px high): formatting bar and pen tray collapse to the single ✎ / Aa buttons.
```

Everything uses `Theme.hit` (40 px × UI scale) as the minimum height/width, so HiDPI/UI scale and "tablet
mode" keep working. Slint properties `narrow`/`tiny`/`short` already exist and will drive these three modes.

### 2.3 Contextual UI (appears only when relevant)

| UI | Appears when |
|---|---|
| Formatting bar (paragraph style, B/I/U, lists, checklist, inline math, link) | caret or selection in text |
| Image/plot handles (resize corners, delete, crop) | an image/plot is selected |
| Math object editor popover (draw / symbols / type shorthand, copy LaTeX/Typst) | a math object is selected |
| **Pen tray** (pen, highlighter, eraser, lasso, colours, 3 favourite pens, ⚙ with width/smoothing/recognition delay/shape snap/paper) | floating; collapsed to one ✎ button; opens on click or when a pen touches the page |
| Handwriting suggestion chip | ~0.6 s after you stop writing (the existing delay setting) |
| Graph, MATLAB, tablet pairing, training, backups, shortcut rebinding | only from ⋯ / Settings / palette |
| Find & replace bar | Ctrl+F |

### 2.4 Plain-language labels

| Today | New |
|---|---|
| Recognize | (automatic suggestion) / "Convert to text" in the chip |
| Insert ↵ | "Accept" on the chip |
| Type / Type here | (gone: you just type) |
| Clear | "Clear all ink" in ⋯ |
| Math / Text / Drawing | "Handwriting becomes: Math · Words · Nothing (drawing)" auto by default, in the tray ⚙ |
| Train | "Teach Scrawl my handwriting" in Settings |
| Paper: Lined / Size: Infinite | "Page style" in ⋯ |

## 3. Where every existing command ends up

Checklist taken from `crates/app/src/commands.rs` (88 commands). All stay in the command palette
(Ctrl+K) and stay rebindable. "Menu" = where a human finds it without the palette. Keys do not change except
where noted.

| Group | Command ids | New home |
|---|---|---|
| General | palette, switcher, settings, shortcuts | top-bar ⌕ (palette + Ctrl+P switcher in one box), Settings in ⋯ |
| View | focus (F9), present (F5), fullscreen, toggle-sidebar, zoom-in/out/reset, theme-dark/light/contrast | ⋯ → View; sidebar ☰; themes in Settings |
| View | toggle-doc (show/hide typeset panel) | **retired**: there is no separate panel; command id kept as a no-op that scrolls to the page (so keymaps do not break) |
| View | split, split-close | ⋯ → View ("Open beside this page") |
| Ink | pen, highlighter, eraser, lasso, pen1-3, mode-math/-text/-drawing, snap-toggle, textbox (alt+t) | pen tray; mode and snap in tray ⚙; `textbox` becomes "put the caret at the end of the page" |
| Ink | undo, redo | top bar ↶ ↷ (one history across text and ink, see 4.6) |
| Ink | clear, delete-selection, insert-axes, insert-ruler | ⋯ → Ink; delete-selection also on the lasso selection bubble; axes/ruler in ＋ Insert → "Drawing aids" |
| Math | recognize, insert, typing, symbols, train, copy-latex/typst/unicode/image, tmpl-frac/sqrt/int/sum/lim/matrix/cases/deriv | recognize/insert = chip (Ctrl+Enter still accepts); typing = typing `$…$` in text; symbols and templates = ＋ Insert → Math and the math-object popover; train in Settings; copy-* in the math popover and ⋯ |
| Notes | new-page, new-section, new-notebook, new-encrypted, lock, duplicate, delete-page, trash, history, favorite, save-template, back, forward, close-tab, next-tab, search, find | sidebar ＋ New page and section/notebook menus; duplicate/trash/history/save-template/lock/delete in ⋯; back/forward and star in the top bar; tabs become a thin strip that only exists when ≥ 2 tabs are open |
| Page | text-block, callout, rule, image, paste-image, pdf, plot, graph, save-graph, record, play, paper, page-mode | ＋ Insert (callout, rule, image, paste image, PDF, plot, table, graph, audio); paper and page-mode in ⋯ → Page style; text-block becomes "insert paragraph here" |
| Export | export, export-notebook, print | Share menu |
| Data | backup, restore-backup | Settings → Backups (also palette) |
| Tablet | tablet | ⋯ → Tablet (panel unchanged, slide-over) |

A unit test will assert that every command id in `commands::ALL` is reachable (appears in a `menu_map()` table
or is palette-only by design), so nothing silently disappears.

### Sidebar

A clean tree: search box at top, then **notebook → section → page**. Pinned (favourites), Recent and Trash
are simple filters at the bottom, not dropdowns. Drag to reorder keeps `DragArea`/`DropArea`; the ▲▼ buttons
become part of a per-row "⋯" menu (still reachable without dragging, needed for pen/mouse users). Trash and
history are small links, not buttons.

## 4. The document editor (the core)

### 4.1 Data model: new crate `crates/doc` (pure Rust, no Slint, fully unit-tested)

```rust
pub struct Doc { pub paras: Vec<Para> }
pub struct Para { pub id: u64, pub kind: ParaKind, pub runs: Vec<Run> }
pub enum ParaKind { Body, Heading(u8 /*1-3*/), Bullet(u8 /*indent*/), Numbered(u8), Check{done:bool},
                    Quote, Code, Callout(CalloutKind), Rule, MathDisplay(Expr+source),
                    Image{file,w,h,crop}, Plot{..}, Table{rows} }
pub enum Run { Text{ text: String, style: Style /*bold|italic|underline|code*/, link: Option<String> },
               Math{ expr: Expr, source: Option<String> } }
```

`Doc` mirrors the existing `Block`s one-to-one for objects (Image/Plot/Rule/Callout/Math keep their fields),
and replaces `Block::Text` markup with structured runs. Page IDs on paragraphs give stable anchors for ink.

Typst remains the **storage and render format behind the scenes**: `Doc::to_typst()` generates it (with
`typst_escape`), `.typ` files and PDF/LaTeX/Markdown/HTML/PNG exports are produced from it. `Doc::from_typst_markup()`
parses the subset that today's text blocks use (`= h`, `- `, `+ `, `*b*`, `_i_`, `` `c` ``, `$m$`, `[[link]]`,
`#link`, tables, quotes) for migration.

### 4.2 Page format and migration (no data loss)

`store::Page` gets one new field: `#[serde(default)] pub doc: Option<doc::Doc>` (plus `ink_anchors`, 4.5).
Old files load unchanged because every new field has a default.

* On open, if `doc` is `None` and the page has `blocks` or `texts`, the app builds a `Doc` in memory:
  `Block::Text{markup}` → paragraphs; `Math` → `MathDisplay`; Image/Plot/Callout/Rule 1:1;
  `CanvasText` (page x,y) → a paragraph placed by y order (text) and, if it was far from the margin, nothing
  else is lost because ink strokes keep their own coordinates.
* **The first save after migration writes the old JSON into version history first** (history already exists),
  then saves `doc`, and keeps `blocks`/`texts` populated for one release so a downgrade still shows the content.
  `export`, `search` and `to_typst` read from `doc` when present, else from `blocks` (so un-opened pages still work).
* Tests: round-trip every `Block` variant; a fixture JSON of a page saved by the current version
  (`tests/fixtures/page_v1.json`, generated from the current binary) must open, migrate, export Typst/LaTeX/MD/HTML
  that still compile, and contain the same text; hostile markup (`#read("/etc/passwd")`) stays escaped.

### 4.3 Layout and rendering

Slint has no rich text widget, so layout is done in Rust.

**Decision: use `cosmic-text`** (pure Rust, MIT/Apache; already builds on `swash`/`rustybuzz`/`fontdb`, which are in our
dependency tree) for shaping, wrapping, caret motion (`Buffer::hit`, `Cursor`, `Motion`), selection rectangles
and rich attributes, instead of walking Typst frames for glyph positions (possible but fragile and slow per keystroke).
Typst stays the renderer for **math** and **images/plots/tables** (it already is), cached per object.

* One `cosmic_text::Buffer` per paragraph, cached by `(para hash, width, scale)`; only paragraphs that intersect the
  viewport are laid out and drawn. Paragraph heights are cached so total scroll height is cheap.
* Inline math: a placeholder run of fixed-advance spaces is laid out inline (width taken from the Typst render of the
  formula); the rendered image is blitted over it. Caret skips over it as one unit (like an inline object in Docs).
* Drawing target: the existing `raster::Canvas` pipeline (tiny-skia, one `Image` handed to Slint) gets a text layer:
  paper → paragraph text/objects → selection → **ink** → live stroke. The blinking caret is a small Slint `Rectangle`
  positioned from Rust (so blinking does not re-render the page).
* Fonts: bundled (typst-assets Libertinus / New Computer Modern / DejaVu Sans Mono) so layout and export agree and no
  system font is required; system sans is optional later.

### 4.4 Editing behaviour (Google Docs rules)

Pure functions on `Doc` + `Selection{anchor,head}` in `crates/doc` (no UI), so each is unit-tested:
insert text, Enter (split paragraph; Enter on an empty list item ends the list), Backspace/Delete (join,
un-list at start), arrows with Ctrl-word and Home/End, Shift-select, PageUp/Down, Select-all, double-click word,
triple-click paragraph, drag-select, Ctrl+B/I/U, typing `- `/`* ` → bullet, `1. ` → numbered, `[] ` → checklist,
`# `/`## ` → headings, `$…$` → inline math, `---` → rule, Ctrl+K link, cut/copy/paste plain text (and paste of
multi-line text becomes paragraphs), Tab/Shift+Tab list indent.

Input path: the existing `FocusScope` in `main.slint` already receives `key-pressed`; it forwards to a new
`doc_key` callback. Mouse: a `TouchArea` over the page forwards down/move/up/double/triple to Rust.
**Not supported initially: IME composition for CJK** (Slint only offers IME through its own `TextInput`);
the existing paste box ("Paste / IME") stays reachable in ⋯ for that. I will say so in the final notes.

### 4.5 Ink on the same page

`Stroke` ids stay as they are. New `ink_anchors: Vec<(stroke_id, para_id, dy)>` in `Page`: when a stroke ends, it is
anchored to the paragraph under (or nearest above) its first point with the offset `dy` from that paragraph's top;
position = para top + dy, so when earlier paragraphs grow, ink moves with its text. Strokes with no anchor (old pages,
strokes in the margin below the last paragraph) stay in absolute page coordinates exactly as today.
All current ink features keep working on top of this: smoothing, pressure, highlighter, pixel/stroke eraser,
lasso move (re-anchors), shape snapping in Drawing mode, scribble-to-delete, favourite pens, custom colours.
Pen input over text never moves the caret; click/tap with a mouse in text mode (no pen tool active) does.
The tray's active tool decides: when a drawing tool is selected the page draws; when none ("Aa" cursor tool) it types.

### 4.6 Undo/redo

One ordered history of "what was last edited": each edit pushes `Layer::Doc` or `Layer::Ink`; Ctrl+Z pops the most
recent and calls that layer's undo (doc snapshots with typing runs coalesced, existing ink undo). This replaces
`text_history`/`text_redo` for canvas text. Redo mirrors it.

### 4.7 Handwriting → text/math in place

The existing recognition pipeline (`session.rs`, `handwriting.rs`, candidate learning, Math/Text/Drawing modes)
stays. What changes is presentation: when recognition finishes the app computes the ink's bounding box, converts
to screen coordinates and shows a **non-modal chip next to the ink**: the best candidate, ✓ Accept, "Keep as ink",
and 4 alternatives (the number keys 1-5 still pick). Accept inserts a `Run::Text` (Text mode) or inline/display
math (Math mode; a lone expression on its own line becomes display) **into the document at the paragraph the
ink is anchored to**, removes the consumed strokes (undoable as one step), and records the personal sample as today.
The mode is "Auto" by default (suggest both; if the ink looks like a formula the math chip leads), with the
manual Math/Words/Drawing choice in the tray ⚙. Dismissing the chip (keep writing, Esc) never deletes ink.

### 4.8 Math objects

Clicking a math run/paragraph selects it and opens a popover with: handwriting area, symbol palette,
shorthand typing field, copy LaTeX/Typst/Unicode/image, delete. Existing `symbols.rs` and `input::Editor` are reused.

### 4.9 Insert menu (＋)

At the caret, one menu: Heading, Bullet list, Numbered list, Checklist, Table, Math (inline/display), Image from
file, Paste image, PDF, Plot, Graph (opens graph view), Callout (4 kinds), Rule, Audio recording, Axes/Ruler,
Link to page. Images/plots/PDF-pages are objects in the flow: click selects (outline + 4 corner handles + bar
with ✕ and Crop), Delete removes, drag a corner resizes (`Block::Image.w/h`).

## 5. New features (cheap and clearly useful; each justified)

| Feature | Why | Cost |
|---|---|---|
| **Checklists** | Docs/Notes parity and the most common note format; typing `[] ` | one `ParaKind`, one toggle |
| **Pinned notes** | Reuse `favorite`: pinned pages float to the top of the sidebar | sort key only |
| **Quick capture** (Ctrl+Shift+N / "＋ New page" in the sidebar) | a blank page with caret ready, no template choice | existing `new-page` + caret |
| **Today note** (⋯ and palette) | find-or-create a page titled with today's date in an "Inbox" section | ~40 lines |
| **Sort pages** (name / modified / created) | long sections need it | a setting + a sort |
| **Outline** | already exists; becomes a collapsible popover from the top bar so long notes are navigable | move only |
| **Word count on demand** | status line shows it; click toggles selection/page | trivial |
| **Back-links under the page** | already computed; shown below the page as "Linked from" | move only |
| **Welcome note** on first launch | shows typing, drawing and handwriting→text in a few lines | one generated page |

Left out on purpose: page colours and emoji (data format and theming cost, little value), cloud sync, collaborative
editing, rich tables with merged cells, fonts/sizes picker (headings and Normal cover notes; keeps files portable).

## 6. Risks and what I will watch

* The editor is the biggest piece. If `cosmic-text` cannot be added or its layout cannot match export closely
  enough, the fallback is Typst-measured paragraphs (documented here before I switch; no silent change).
* Slint re-entrancy: every Rust → Slint call that can call back (`ui.invoke_*`) will go through `try_borrow`
  or property setters, as already done in `close_overlay`/`pen_lifted`. New doc callbacks will not be invoked while
  `State` is mutably borrowed.
* Screenshots cannot test real drag, hover, a blinking caret, IME or pen pressure. New headless scenes can drive
  keys and pointer events through the callbacks, but the final summary will list a manual checklist for those.
* Window size at 480 px with UI scale 2.0 is the hardest case; I will check 1500 / 940 / 620 / 480, dark/light/high
  contrast, scale 1 and 2, and a short window after each layout step.
* Memory/CPU: text layout is cached per paragraph and only the visible ones are drawn; I will time a 500-paragraph
  note before and after and keep typing latency under one frame.

## 7. Build order (each step is a commit that builds, passes clippy -D warnings, all tests and `--self-test`)

1. **`crates/doc` model + Typst/markup conversion + migration + tests** (no UI change, `Page.doc` added with serde default).
2. **Layout and rendering** with cosmic-text; new headless scene `doc` and screenshot test; behind a hidden
   setting `new_editor` (default off) so the old UI keeps working while we build.
3. **Caret, selection, hit-testing, keyboard and mouse editing, clipboard, undo** (scenes `docedit`, `docselect`).
4. **Formatting bar, shortcuts, autoformat (lists, headings, `$…$`), links, checklists, inline math objects.**
5. **Ink on the document**: unified scroll, anchoring, pen tray with tools/colours/⚙ sliders, scenes `docink`.
6. **Handwriting suggestion chip** and accept-in-place; math object popover; Insert menu with images/plots/PDF.
7. **Calm layout**: new top bar, ＋ and ⋯ menus, sidebar tree, contextual bars, narrow/short modes, tabs strip, welcome
   note; switch `new_editor` on by default; command-reachability test; screenshots at 1500/940/620/480, dark/light.
8. **New features** (checklist done in 4; pinned, today, quick capture, sort, outline popover, back-links, word count).
9. **Remove dead code** (old text panel, `textbox_*`, `canvas_texts`, `Block::Text` write path), update `HANDOFF.md`,
   `README.md` (new screenshots in `docs/screenshots/`), `CHANGELOG.md`, and finish with the summary
   "what moved where" and the manual test checklist.

Steps 1-4 are independent of the visual overhaul, so the app is always releasable. If time runs short, the cut order is:
8 (new features), then inline-math objects inside paragraphs (they would stay display-only), then tables; steps 1-7
are the required path.
