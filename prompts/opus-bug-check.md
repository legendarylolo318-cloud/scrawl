# Prompt for Claude Opus: bug check the Obsidian features, plugins and the new editor

You are reviewing **Scrawl** (`/home/lolo/scrawl`, GitHub `legendarylolo318-cloud/scrawl`), a note-taking app in
**Rust + Slint only (no Python anywhere: not in the app, scripts, CI, or your own helpers; use Rust, cargo and shell)**.
The owner has a hand disability and uses a mouse or pen most of the time: every control must stay reachable by mouse or
pen with targets of at least 40 px, and also by keyboard. Treat anything confusing, fiddly, or data-losing as a real bug.

Your job: **find real bugs, prove each one, fix it with a test, and report.** Do not rewrite working code for style.
Do not add features. Prefer small, always-working commits.

## What was just added (review all of it)

1. **Obsidian features**
   * Markdown <-> document: `crates/doc/src/md.rs` (`Doc::from_markdown`, `to_markdown`, `front_matter`, `hashtags`).
   * `[[` page-link suggestions and `[[Title]]` auto-link: `crates/doc/src/edit.rs` (`link_query`, `accept_link`,
     `autoformat_link`), `crates/app/src/docedit.rs` (`link_suggestions`, key handling for Tab/Enter/Esc), pop-up in `main.slint`.
   * Linked-from strip (`Workspace::backlinks`, uses the search index and saved pages only).
   * Tags: `#tags` merged into `page.tags` in `docedit::after_edit`; Tags overlays (`tags`, `tagpages`) in `actions.rs`.
   * Graph of linked notes: `crates/app/src/notegraph.rs`, overlay `notegraph` in `main.slint`.
   * Vault export/import: `crates/app/src/vault.rs`, commands `export-vault` / `import-vault` (folder pick or prompt).
   * Paste of Markdown keeps structure: `docedit::paste`, `looks_like_markdown`.
2. **Plugins**: `crates/plugin` (manifest validation, wasmi sandbox, actions), app glue in `main.rs`
   (`reload_plugins`, `plugin_commands`), `actions.rs` (`run_plugin`, Plugins overlay), `docedit.rs`
   (`apply_plugin_actions`, `insert_paras`), `crates/app/starter/plugin.json`, docs `docs/plugins.md`, `docs/obsidian.md`,
   `SECURITY.md` (Plugins section), examples in `examples/plugins/`.
3. **The earlier rewrite** (still young, lightly tested by hand): the document editor (`crates/doc`, `crates/docview`,
   `crates/app/src/docedit.rs`), ink anchored to paragraphs, handwriting chip, Auto words/math mode, undo order, Find/Replace,
   calm layout (`main.slint`, `menus.rs`). `docs/ui-redesign.md` and `HANDOFF.md` describe it and list known gaps.

Read `HANDOFF.md`, `README.md`, `SECURITY.md`, `docs/plugins.md`, `docs/obsidian.md` first.

## Hard rules

* **Slint re-entrancy** (caused crashes before): never call a Slint callback or function from Rust (`ui.invoke_*`) while
  `State` is mutably borrowed if that Slint code calls back into Rust (it re-borrows the `RefCell` and panics). Use
  `try_borrow` or set properties directly. Check every new callback (`open-menu`, `note-graph-*`, `doc-key`, `choose`,
  `run-command` from the new Slint pop-ups) for this.
* **Never lose user data.** Autosave, crash recovery, history, trash, encryption, export, and the one-time conversion of
  old pages to `Page.doc` must keep working. Convert real-looking old pages and check nothing is lost (there is an ignored
  test: `SCRAWL_MIGRATE_CHECK=<notes copy> cargo test -p store --release migrate_real_notes -- --ignored`). **Never run
  anything destructive on `~/Scrawl`**: always copy it first or set `SCRAWL_NOTES_DIR` to a temp folder.
* Before every commit: `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --release`,
  `./target/release/scrawl --self-test` all pass.
* Commit as `legendarylolo318-cloud <274669742+legendarylolo318-cloud@users.noreply.github.com>` (never a personal e-mail).
  End commit messages with the attribution line the environment gives you. Push to `main` when green.
  **Do not create or move release tags** unless asked.
* Screenshot mode cannot test real dragging, hover, pen pressure, caret blink or IME. Say what you could not verify.
  Off-screen screenshots: `SCRAWL_NOTES_DIR=/tmp/x SCRAWL_SHOT=out.png SCRAWL_SHOT_SIZE=940x800 SCRAWL_SHOT_SCENE=<scene> ./target/release/scrawl`
  (scenes are in `run_scene` in `crates/app/src/main.rs`; new ones: `linksuggest plugins notegraph undoorder docword docink`).
  Look at the images you produce.

## Where to look hardest (suspected weak spots)

**Plugins and the sandbox (security first).** Try to break out or hang the app:
* A module that imports anything; a module whose `alloc` returns a pointer outside memory; `scrawl_run` returning a huge or
  negative packed length; a start function that loops; memory growth; deep recursion; huge tables; a `.wat` bomb; a manifest with
  path traversal, symlinks (`plugin.json`, the wasm file, the plugin folder), duplicate names, 100+ commands, huge titles.
* Does every call really use a fresh instance and fuel? Can fuel be exhausted during `instantiate_and_start`
  (start function) and is that reported cleanly? Can a plugin make the UI thread block long (fuel 50 M ≈ how many seconds in
  debug and release)? If it is too slow, propose a lower budget or running off the UI thread, with a test.
* `apply_plugin_actions`: a plugin returning thousands of paragraphs, huge text, Markdown with images (`![[x]]`) or
  `Markup` paragraphs: does layout/render stay responsive? Is one Ctrl+Z enough? Does a failing plugin leave the document half
  edited or the undo stack wrong?
* Disabled plugins must never run (also via the palette and stale menu rows). Plugin ids with `:` or odd characters.

**Markdown and vault.**
* Round trips: Scrawl -> Markdown -> Scrawl for every paragraph kind, inline style, link and math, including text that
  merely looks like Markdown (`# x`, `1. x`, `- x`, `> x`, `*a*`, `[x](y)`, `$5 and $6`, `\`, `|`, `<u>`), nested lists, tabs,
  CRLF files, a BOM, very long lines, empty files, front matter without a closing `---`.
* Titles that need sanitising (`a/b`, `con`, `aux.`, trailing dots/spaces, very long, only dots, unicode, case-only
  duplicates on Windows): file names, link targets, `[[Name|alias]]`, collisions, and links after import (`title_of_stem`).
* Import safety: symlink loops, `..`, huge folders (>5000 files), deep trees, 20 MB images, corrupt images, non-UTF-8, files
  changing while read. Does `import` leave a half-created notebook when it fails midway? Does it switch notebooks (and save the
  current page first)? Encrypted notebook currently open?
* Export: `ws.page` vs disk copies, pages never opened (no `doc`), media missing, ink-only pages, name clashes in `attachments/`.

**Links, tags, graph.**
* `[[` pop-up: wrong position near the edges/while scrolled/zoomed, stale suggestions after moving the caret, Tab/Enter
  swallowed when no popup is visible, Esc dismiss state, IME/dead keys, `[[` inside code, math, headings, lists, selection active.
  Does typing `]]` inside inline math or code convert wrongly? Undo after accepting a link?
* Backlinks: case, aliases, links with `#heading`, pages whose blocks are stale until saved, huge notebooks (speed).
* Tags: `#` rules (`C#`, `#1`, `a#b`, URLs with `#`, Typst-escaped text), tags never removed, unicode, tags with `/`.
* Graph: duplicate/blank titles, 600-node cap, ghost nodes, clicking a ghost creating a page with a weird title, window resize
  while open, HiDPI, empty notebook, single page, very long titles, layout time for 600 nodes.

**The editor (still young).**
* `notedoc::edit` invariants under random operations (try a property-style test: random sequences of insert/enter/backspace/
  delete/format/indent/object operations never panic, never leave an empty `paras`, offsets always valid, `to_typst()` ->
  `from_markup()` stable). Inline math and objects at paragraph edges, selections across objects, Unicode (combining marks,
  emoji, RTL, wide chars), very long paragraphs and 5000-paragraph documents (layout time, memory, scrolling).
* Ink anchors: `sync_anchors` shifting strokes while the pen is down, lasso drag, eraser, undo/redo, deleting a paragraph,
  pages with PDF background, zoom. Strokes jumping or drifting after repeated edits? Anchors after `ensure_doc` migration.
* Undo order (`undo_latest`/`note_ink`): mix typing, ink, lasso, eraser, snippet/plugin inserts, handwriting accept; redo chain;
  switching pages (stacks reset?). Text undo restoring a doc from another page?
* Autosave/crash recovery with `Page.doc`: kill the process mid-edit and reopen; `sync_blocks` on every save path
  (recovery file, history snapshot, template, duplicate, export, search index).
* Handwriting chip and Auto mode: accept, "As words/As math", alternatives, learning (`learn_symbol`) on wrong labels,
  dismiss, switching tools, stale chip after undo, pages with `ink_mode` set.
* Menus and overlays: every menu item runs the command it names; Esc closes; the tray, chip, link pop-up and menu never cover
  each other or go off-screen at 1500/940/620/480 px wide, short windows, HiDPI (`SCRAWL_SHOT_SCALE=2`), light, high-contrast
  and custom themes, UI scale 2.0. All targets >= 40 px.
* Keyboard: Tab/Enter/Esc are claimed by the link pop-up only when it is visible; Ctrl shortcuts do not fire while a text
  field (title, search, prompt) has focus; Ctrl+Z/Y consistent.
* Anything else you notice: crashes, panics (`unwrap`/indexing on user data), O(n²) hot paths, memory growth, leaking timers.

## Method

1. Build and run the checks once to get a green baseline.
2. For each area above write **failing tests first** (unit tests next to the code; for app-level flows use a headless scene
   with asserts like the existing ones). A bug is real only if you can show a failing test or a screenshot.
3. Fix, re-run the three checks, commit small, push to `main`.
4. Fuzz where cheap: `cargo test` with randomized inputs seeded deterministically (no extra dependencies unless essential).
5. Keep `HANDOFF.md`, `CHANGELOG.md` and the docs honest if behaviour changes.

## Report format (final message)

A table: `# | severity (data loss / crash / security / wrong behaviour / polish) | where | how to reproduce | fix commit`.
Then: what you checked and found clean, what you could not verify (and why), risky areas you would test by hand with a pen,
and a short manual test list for the owner.
