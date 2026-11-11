# Scrawl plugins

Plugins add commands to Scrawl. They are small, sandboxed programs (WebAssembly, so you can write them in Rust)
or no-code **snippet packs**. Everything on the Scrawl side is Rust (`crates/plugin`, runtime: [wasmi](https://github.com/wasmi-labs/wasmi),
a pure-Rust interpreter).

Plugins live in `~/Scrawl/plugins/<name>/` (Windows: `%USERPROFILE%\Scrawl\plugins\`). Open **⋯ → Plugins** to see
them, switch them on or off, and read why one failed to load. **⋯ → Reload plugins** picks up changes. Their commands
appear at the bottom of the ⋯ menu and in the command palette (Ctrl+K). One Ctrl+Z undoes a plugin's whole change.
On first start Scrawl creates `starter-snippets` (meeting notes, daily review, Cornell notes, definition, problem entry).

## No-code: snippets

`plugin.json`:

```json
{
  "name": "my-snippets",
  "version": "1.0.0",
  "description": "My note layouts",
  "snippets": [
    { "id": "standup", "title": "Stand-up", "markdown": "## Stand-up\n\n- [ ] yesterday\n- [ ] today\n- [ ] blockers" }
  ]
}
```

The Markdown can use headings, lists, `- [ ]` tasks, `> [!note]` callouts, `[[links]]`, `$math$`, `$$ blocks $$`,
code fences and tables.

## Code: a Rust (or WAT) plugin

```json
{ "name": "text-tools", "wasm": "text_tools.wasm",
  "commands": [ { "id": "upper", "title": "Selection to UPPER CASE" } ] }
```

`wasm` is a `.wasm` file or a `.wat` text file inside the plugin folder. The module must export:

| export | meaning |
|---|---|
| `memory` | linear memory |
| `alloc(len: i32) -> i32` | give Scrawl room for `len` bytes |
| `scrawl_run(ptr: i32, len: i32) -> i64` | handle a request; return `(out_ptr << 32) \| out_len` |

**Request** (JSON at `ptr`): `{"command": "upper", "selection": "...", "page_title": "...", "text": "<page text, up to 200 000 chars>"}`.

**Response** (JSON): `{"actions": [ ... ]}` with up to 100 of:

| action | effect |
|---|---|
| `{"op":"insert_text","text":"..."}` | type text at the caret |
| `{"op":"replace_selection","text":"..."}` | replace the selection (or insert at the caret) |
| `{"op":"insert_markdown","markdown":"..."}` | add paragraphs after the caret |
| `{"op":"status","text":"..."}` | show a message in the status line |

Examples (both tested): `examples/plugins/hello-wat` (the smallest plugin) and `examples/plugins/text-tools` (Rust:
upper/title case, sort lines, lines to tasks, word count). The Rust example's logic has native unit tests; **building it for
`wasm32-unknown-unknown` was not done in the environment that wrote it** (no wasm target installed), so try
`cargo build --release --target wasm32-unknown-unknown` and report problems.

## Sandbox (what a plugin cannot do)

* The module may **not import anything**: no files, network, clock, environment, no WASI. A module with imports is refused.
* It only sees the request above (the selection and the current page's text), nothing from other notes.
* Every call runs in a **fresh instance** with a 50 million instruction budget and a 64 MB memory cap; a plugin that loops
  or allocates too much is stopped with a message.
* It can only return the four actions above. Text size and action count are capped.
* Plugin folders are read only: manifest ≤ 256 KB, module ≤ 8 MB, no symbolic links, `wasm` must be a plain file name inside the folder.

Plugins are still code you chose to install: a plugin can write anything into your page (undo removes it). Install only
plugins you trust. Disabled plugins never run.
