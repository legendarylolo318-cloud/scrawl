# Obsidian features in Scrawl

Scrawl keeps your notes as local files and now speaks Obsidian's language.

| Obsidian | Scrawl |
|---|---|
| `[[Wiki links]]` | Type `[[` for a pick-list of your pages (Tab/Enter takes the first, Esc closes it), or type `[[Title]]`. Click a link to open it; a link to a missing page creates it. |
| Backlinks | A **Linked from** strip under the page lists the pages that link here. |
| Graph view | **⋯ → Graph of linked notes**: dots are pages, lines are links, grey dots are links to pages that do not exist yet. Click one to open it. |
| `#tags` | Type `#idea` in a note and it becomes a page tag. **⋯ → Tags** lists tags and their pages; `tag:idea` also works in the sidebar search. |
| Quick switcher / command palette | Ctrl+P and Ctrl+K (unchanged). |
| Daily note | **⋯ → Today's note**. |
| Templates / snippets | Plugins with `snippets` (see `docs/plugins.md`). |
| Community plugins | Sandboxed WebAssembly plugins (`docs/plugins.md`). |
| Vault of Markdown files | **Share → Export as Obsidian vault** and **⋯ → Import an Obsidian vault**. |
| Paste from Obsidian / GitHub | Pasting Markdown keeps headings, lists, tasks, links and code. |

## Vault export

Writes `<folder>/<notebook>/<Section>/<Title>.md` plus `attachments/`. Each file starts with front matter
(`title`, `tags`, `created`). Page content uses `# headings`, `- [ ]` tasks, `[[links]]` (file names and `|alias`
when the title had characters files cannot contain), `$math$`, `$$` blocks, `> [!note]` callouts (Definition → note,
Theorem → tip, Example → example, Warning → warning), `![[image.png]]`, tables kept as a `typst` code block.
Ink (handwriting) and audio are **not** exported: they have no Markdown form.

## Vault import

Reads every `.md` below the chosen folder into a **new notebook** (the folder name + "(imported)"); top-level folders
become sections. Dot-folders (`.obsidian`, `.git`, `.trash`), symbolic links, files over 5 MB and non-UTF-8 files are
skipped (and counted in the status line). Supported: front matter `title`/`tags`, `#tags`, wiki links with `|alias`
and `#heading` targets, tasks, callouts, tables, code fences, `$..$` and `$$..$$` math (LaTeX is converted), `![[image]]`
and `![](path)` images found in the vault (re-encoded as PNG). Not supported: embeds of other notes (`![[Note]]`),
Dataview, Canvas files, block ids (`^id`), footnotes, strike-through and highlight marks (the text is kept).
Single line breaks are separate paragraphs, like Obsidian's default.
