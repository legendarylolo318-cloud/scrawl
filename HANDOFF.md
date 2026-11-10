# Scrawl handoff

Snapshot of the review/finish/release work, so it can be picked up later. The original task brief is
in `REVIEWER_PROMPT.md` (untracked). Read `REVIEW_NOTES.md` for the older checklist; this file
supersedes it where they disagree.

## State right now

- Everything is pushed to GitHub `main` (originally committed as "WIP: UI overhaul, security fixes, export/store
  fixes"). The repo was created and pushed by you on 2026-11-10 (CI runs 42785029618 and 42785146887).
- Linux release build: OK. `scrawl --self-test`: PASSES.
- `cargo test --workspace --release`: all pass (the search bug below is fixed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo audit` (run 2026-11-10, both tools now installed): **fails**. Real advisories: `zip` 0.6.6
  (RUSTSEC-2026-0367/0368/0369/0429: extraction symlink/SUID issues), `zstd-safe` 5.0.2 (several, incl.
  RUSTSEC-2026-0403), `quick-xml` 0.38.4 via typst → hayagriva → citationberg (RUSTSEC-2026-0194/0195),
  `crypto-bigint` 0.5.5 via age → p256 (RUSTSEC-2026-0387, timing), `rustybuzz`/`ttf-parser`
  (RUSTSEC-2026-0206/0192), `zbus_names`/`zvariant` (RUSTSEC-2026-0370/0371/0372, D-Bus for rfd/keyring).
  Unmaintained warnings: basic-toml, bincode, paste, pin-utils, unsafe-libyaml, yaml-rust.
  `cargo tree -i zip@0.6.6 -e all --target all` prints nothing, so it is probably only the trainer
  (burn) or a build tool: check with `cargo tree -i zip@0.6.6 -p scrawl-train`. Our own zip
  extraction (backup restore) does not use `ZipArchive::extract` and checks `enclosed_name`.
  Plan: `cargo update -p <crate>` where semver allows, otherwise bump the parent crate, otherwise add
  documented ignores in `deny.toml`/`.cargo/audit.toml` with a reason. `cargo deny` not run yet.
- OLD NOTE (superseded): installing them was started (`cargo install --locked cargo-audit cargo-deny`)
  but has not been checked, and neither has been run. There is no `deny.toml` yet.
- Windows: first build is the CI runs above (results not read yet). No local Windows toolchain.

## Done in this session

### Security (tablet server, `crates/remote`)
- **Revocation now cuts off live sessions**. Previously a revoked device stayed connected until it
  reconnected. Uses a `watch` channel bumped on revoke/revoke-all; the socket loop re-checks trust.
  Covered by the e2e test `revoking_a_device_cuts_off_its_live_session`.
- **Leaf certificate renews while running.** It was renewed only at start. Leaf lifetime is now 7 days, renewed in
  the last 2 days, via a swappable `ResolvesServerCert` checked by the 2 s watchdog. Covered by the e2e test
  `leaf_certificate_is_renewed_while_running`.
- New pen-test e2e test: path traversal on static routes, oversized frames, 5000-point strokes and
  floods from unpaired sockets. Nothing leaks and the server stays healthy.
- **`Frame` message implemented.** The app renders the active expression (Typst → PNG) and sends it.
  The web client shows it (`#frame` img; CSP already allows `data:`).
- Still TODO from the brief: run `openssl x509 -text` on a real CA, check the keyring on Windows, test
  on real iOS/Android devices, update SECURITY.md (7-day leaf, live revoke, Frame).

### Storage / export correctness
- Typst injection/escaping: titles and course names are now escaped (`store::typst_escape`). A title like
  `#read("/etc/passwd") $x$` previously broke or altered exports.
- LaTeX export rewritten: full escaping (`latex_escape`, keeps `$..$`), itemize/enumerate lists, graphicx.
- PDF/PNG export now includes **images** (with crop), **plots** (rendered SVG), Letter/A4 page size,
  PDF-background pages (ink composited). PNG is one tall page instead of only page 1.
  HTML embeds images as data URIs and plots as SVG.
- Media storage sealed with the notebook vault (`Store::write_media/read_media`; names validated).
- Backup restore (`store::backup::restore_backup`), zip-slip safe, takes a safety backup first.
- Trash: the `.typ` copy now moves to the trash and is purged too (it used to linger as plaintext).
- Page model: `audio: Option<AudioClip>` and an image `crop`, both `serde(default)`, so old files still load.

### Ink
- `ink::snap_shape` (line/ellipse/rect), applied **only in Drawing mode**, so O/0/∘ in math are
  never snapped. Tests cover small circles staying untouched.
- Lasso selection and move as one undoable edit (`Document::translate_ids/replace_ids`).
- Cached canvas renderer (`raster::Canvas`): paper, background and ink are rasterized once; pen moves
  only draw the live stroke. (Before/after timings not measured yet.)
- Ink colour adapts to the light theme.

### App (`crates/app`): large UI rewrite (`ui/main.slint`, `main.rs`, new `actions.rs`)
New modules: `settings.rs` (persisted, sanitized), `commands.rs` (one table drives the palette,
rebindable shortcuts and the reference screen), `pagedoc.rs` (typeset blocks, formatting, outline,
links, word count, find/replace, spell check with the bundled SCOWL en_US dictionary in `app/dict/`),
`symbols.rs` (palette covering every model class, shorthand, templates, accents and units),
`workspace.rs` (rewritten).

Wired in the UI, but **most of it only checked by compiling plus one screenshot**:
command palette (Ctrl+K / Ctrl+Shift+P / toolbar button), quick switcher (Ctrl+P), settings screen
(theme dark/light/high-contrast/custom, UI scale, tablet mode, snapping, spell check, paper colour,
backups), shortcut reference with rebinding, symbol palette, typeset "Page" panel with per-block
render/move/edit/delete/copy LaTeX/Typst, image resize/crop, text blocks with formatting buttons
(headings, lists, tables, code, quote, inline math, links, callout, rule), spelling suggestions,
find/replace, word count, outline, `[[links]]` (clicking a missing link creates the page), backlinks,
tabs, back/forward, split view, focus mode (F9), presentation mode (F5), sidebar with sections and
drag-and-drop (Slint `DragArea`/`DropArea`) plus ▲▼ buttons, favourites/recent filters, trash restore,
version history with preview and restore, notebooks incl. encrypted create/unlock/lock with
wrong-password handling, user templates, paper/page-size toggles, custom colours plus 3 favourite pens,
axes/ruler insert, lasso, Insert ↵ (recognized math → page block), typing autocomplete chips,
IME/paste box (LaTeX → Typst via `layout::latex_to_typst`), image insert/paste (file dialog via `rfd`,
clipboard via `arboard`), PDF import (one page per PDF page as an annotatable background), audio
record with ink sync and play-from-selection, save graph into the page, drawn/typed expression →
graph, print (temporary PDF in `~/Scrawl/.print`, deleted after 1 h), daily backups, external-change
reload prompt, persisted search index for unencrypted notebooks (memory-only for encrypted), and
remembered window size, tabs and last page.

Files dropped on the window go through winit's `DroppedFile` (feature `unstable-winit-030`). This works on
X11/Windows only. **winit has no Wayland DnD, so on Hyprland use paste or the file picker.**

## Next session: start here (updated 2026-11-10, end of day)

Done since the first handoff: repo created and pushed (https://github.com/legendarylolo318-cloud/scrawl),
clippy `-D warnings` clean, all tests pass locally, search filter bug fixed, SECURITY.md updated,
release.yml names fixed, version 1.0.0. Local and remote `main` are in sync.

Do these in order:

1. **Read the first CI results.** `gh run list --limit 3`, then `gh run view <id> --log-failed`
   for any failed run. Run `42785029618` is the first-ever CI run (Ubuntu + Windows).
   Likely problems: missing apt packages for winit/arboard (`libxkbcommon-x11-dev`, `libxcb*`),
   Windows-only compile errors, tests that assume Unix paths or a keyring, and `--self-test`
   printing nothing on Windows (release builds use `windows_subsystem = "windows"`; CI can only
   rely on the exit code). Fix, push, repeat until green.
2. **Click through the UI** (see "Known broken" item 3): overlays, widths 1500/940/620/480, short
   heights, `SCRAWL_SHOT_SCALE=2`, `SCRAWL_SHOT_THEME=light`. Fix layout bugs found.
3. **MATLAB `for` loops** (item 6): implement in `crates/media/src/matlab.rs` or the `matlab`
   screenshot scene stays broken.
4. **Recognition** (item 7): a drawn "+" is ranked "±" first. Add an eval mode with shaky/tremor
   noise to `scrawl-train`, measure top-1/top-5, retrain if top-5 < 97 %, update `metrics.json`.
5. **`cargo audit` and `cargo deny check`**: both still unrun. Add a `deny.toml` (licences: MIT,
   Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0, ODbL data note, Slint royalty-free licence).
6. Performance numbers (item 9), README/CHANGELOG update with screenshots and numbers, Slint
   licence attribution.
7. **Release**: tag `v1.0.0` only when CI is green and audit/deny pass, `git push --tags`,
   `gh run watch`, check the four artifact names plus SHA256SUMS, test the AppImage locally
   (`--self-test` and a manual launch on Hyprland), then publish the draft release with notes.

## Known broken / not verified

1. FIXED: search with filters matched every page in the notebook (the text clause was optional next to `+nb:`; now `+(...)`). Old note:
   `search("eigenvalues")` returns 2 hits instead of 1. Suspect: the persisted index adds the page in
   `ensure_index` and again in `save()`. `delete_term` is applied after the add, or the page is
   indexed under both the old and the new commit. Check `SearchIndex::add` and the manifest logic in
   `workspace.rs::ensure_index`.
2. Run clippy and fix the lints.
3. **Manually click through the UI.** Only one off-screen screenshot of the main view was checked. Check:
   overlays (palette, symbols, settings, shortcuts, trash/history/backups, prompt, colours), narrow
   widths (940, 620, 480), short heights and `SCRAWL_SHOT_SCALE=2` (HiDPI), the light theme
   (`SCRAWL_SHOT_THEME=light`). Scenes: `SCRAWL_SHOT_SCENE=draw,insert,text,typing,graph,matlab,
   tablet,notes,palette,symbols,settings,shortcuts,focus,lasso,nodoc`. Set `SCRAWL_NOTES_DIR` to a temp
   dir so screenshots don't touch real notes.
4. The custom-theme swatches call `set-custom-theme(part, c.red, c.green, c.blue)`. Check that Slint's
   colour channels are 0–255 ints here.
5. The DragArea needs `data` (set from Rust as plain text "scrawl-page"). Confirm a drag actually starts.
6. The MATLAB screenshot scene uses a `for` loop. **`for` is NOT implemented in `media/src/matlab.rs` yet**
   (the brief asks for it). Implement it or change the scene.
7. **Recognition**: the self-test shows a drawn "+" ranked as "±" first (with "+" second), and "+ =" in the
   screenshot became "± =". The shaky-set top-1/top-5 measurement, real multi-stroke ink eval and a possible
   retrain (`cargo run --release -p scrawl-train -- train`) have not been started. The trainer has no
   eval/shaky mode yet: add one in `crates/train/src/main.rs`.
8. Symbol coverage table (model / palette / shorthand / Typst / LaTeX) not produced. The palette
   test proves every model class is in the palette.
9. Performance numbers (idle RAM/CPU, page open, search on 1000 pages, canvas before/after) not measured.
   The tantivy writer is now 1 thread / 15 MB to cut RAM.

## Release work still to do

- DONE (untested): `release.yml` now strips the `v` from the tag and produces `Scrawl-1.0.0-x86_64.AppImage`, `Scrawl-1.0.0-windows-x64.exe` (portable) and `Scrawl-1.0.0-windows-x64-setup.exe` (Inno `OutputBaseFilename` updated); workspace version bumped to 1.0.0. Nothing has run in Actions yet. Original note:
  Strip the `v`. Required names: `Scrawl-1.0.0-x86_64.AppImage`, `Scrawl-1.0.0-windows-x64.exe` (portable,
  replaces the zip), `Scrawl-1.0.0-windows-x64-setup.exe`, `SHA256SUMS`. Check `packaging/scrawl.iss`
  output name. CI on Ubuntu also needs the new deps: `libxkbcommon-x11-dev libxcb*` (arboard/winit),
  and possibly `libgtk-3-dev` is NOT needed (rfd uses xdg-portal). The Windows build now has
  `windows_subsystem = "windows"` in release builds: `--self-test` prints nothing to a console there,
  so CI must rely on the exit code (or attach a console when `--self-test` is passed).
- Update README (features, screenshots, numbers), CHANGELOG
  and SECURITY.md (add SCOWL dictionary attribution: `crates/app/dict/LICENSE-SCOWL.txt`).
- Check Slint's licence terms (royalty-free licence requires attribution in the app/README).
- Push, tag `v1.0.0` only after tests + clippy + audit + deny pass, then `gh run watch`, fix and re-tag
  until green, verify the artifacts, and publish the release.

## Useful commands

```bash
cargo build --release -p scrawl && ./target/release/scrawl --self-test
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
SCRAWL_NOTES_DIR=/tmp/shot SCRAWL_SHOT=out.png SCRAWL_SHOT_SIZE=940x900 SCRAWL_SHOT_SCENE=draw,insert ./target/release/scrawl
```
