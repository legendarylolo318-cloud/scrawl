//! Scrawl plugins: small, sandboxed programs (WebAssembly) and no-code snippet packs that add commands.
//!
//! A plugin lives in its own folder with a `plugin.json` manifest:
//!
//! ```json
//! { "name": "shout", "version": "1.0.0", "description": "Upper-case the selection",
//!   "wasm": "plugin.wasm",
//!   "commands": [ { "id": "shout", "title": "Shout the selection" } ],
//!   "snippets": [ { "id": "meeting", "title": "Meeting notes", "markdown": "# Meeting\n\n- [ ] item" } ] }
//! ```
//!
//! **Sandbox.** The WebAssembly module may not import anything (no files, no network, no clock, no
//! environment): it only receives a JSON request and returns JSON actions. Each call runs in a fresh
//! instance with a fuel (instruction) budget and a memory cap, so a broken plugin can neither hang nor
//! exhaust the app, and cannot read other notes. `wasm` may be a `.wasm` binary or a `.wat` text file.
//!
//! **ABI** (exports of the module): `memory`; `alloc(len: i32) -> i32`; `scrawl_run(ptr: i32, len: i32) -> i64`
//! where the argument is the request JSON in memory and the result packs `(out_ptr << 32) | out_len` of the
//! response JSON `{"actions":[...]}`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

pub const MAX_WASM_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const FUEL: u64 = 50_000_000;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const MAX_ACTIONS: usize = 100;
const MAX_TEXT: usize = 1_000_000;
const MAX_REQUEST_TEXT: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub markdown: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// `.wasm` or `.wat` file inside the plugin folder.
    #[serde(default)]
    pub wasm: Option<String>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

impl Manifest {
    pub fn parse(json: &str) -> Result<Manifest, String> {
        let m: Manifest = serde_json::from_str(json).map_err(|e| format!("plugin.json: {e}"))?;
        if !valid_id(&m.name) {
            return Err("name must be 1-40 characters: a-z, 0-9, - or _".into());
        }
        if m.commands.len() + m.snippets.len() > 100 {
            return Err("too many commands".into());
        }
        let mut seen = std::collections::HashSet::new();
        for id in m.commands.iter().map(|c| &c.id).chain(m.snippets.iter().map(|s| &s.id)) {
            if !valid_id(id) {
                return Err(format!("command id {id:?} must be 1-40 characters: a-z, 0-9, - or _"));
            }
            if !seen.insert(id.clone()) {
                return Err(format!("command id {id:?} is used twice"));
            }
        }
        for t in m.commands.iter().map(|c| &c.title).chain(m.snippets.iter().map(|s| &s.title)) {
            if t.trim().is_empty() || t.chars().count() > 80 {
                return Err("every command needs a title of up to 80 characters".into());
            }
        }
        if m.snippets.iter().any(|s| s.markdown.len() > MAX_TEXT) {
            return Err("a snippet is too long".into());
        }
        if !m.commands.is_empty() && m.wasm.is_none() {
            return Err("commands need a \"wasm\" file (use \"snippets\" for no-code plugins)".into());
        }
        if let Some(w) = &m.wasm {
            let p = Path::new(w);
            if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                return Err("\"wasm\" must be a file name inside the plugin folder".into());
            }
        }
        Ok(m)
    }
}

/// What the plugin is told about the page.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Request {
    pub command: String,
    pub selection: String,
    pub page_title: String,
    /// Plain text of the page (cut to 200 000 characters).
    pub text: String,
}

/// What a plugin may ask the app to do. Nothing else is possible.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    /// Type text at the caret.
    InsertText { text: String },
    /// Replace the selection (or insert at the caret when nothing is selected).
    ReplaceSelection { text: String },
    /// Insert Markdown (headings, lists, tasks, [[links]], math) as paragraphs after the caret.
    InsertMarkdown { markdown: String },
    /// Show a message in the status line.
    Status { text: String },
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Clone)]
pub struct Plugin {
    pub manifest: Manifest,
    pub dir: PathBuf,
    wasm: Option<Vec<u8>>,
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > max {
        return Err(format!("{} is larger than {} bytes", path.display(), max));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

impl Plugin {
    /// Load `<dir>/plugin.json` and its module. Nothing runs until a command is called.
    pub fn load(dir: &Path) -> Result<Plugin, String> {
        let manifest = Manifest::parse(&String::from_utf8(read_limited(&dir.join("plugin.json"), MAX_MANIFEST_BYTES)?).map_err(|_| "plugin.json is not UTF-8".to_string())?)?;
        let wasm = match &manifest.wasm {
            Some(f) => {
                let bytes = read_limited(&dir.join(f), MAX_WASM_BYTES)?;
                let bytes = if f.ends_with(".wat") { wat::parse_bytes(&bytes).map_err(|e| format!("{f}: {e}"))?.into_owned() } else { bytes };
                // fail early on a module that is broken or wants imports
                let engine = Engine::default();
                let module = Module::new(&engine, &bytes[..]).map_err(|e| format!("{f}: {e}"))?;
                if let Some(i) = module.imports().next() {
                    return Err(format!("{f}: imports {}::{}, but plugins may not import anything", i.module(), i.name()));
                }
                Some(bytes)
            }
            None => None,
        };
        Ok(Plugin { manifest, dir: dir.to_path_buf(), wasm })
    }

    /// Run command `id` (a manifest command or `snippet:<id>`) and return what it asks for.
    pub fn run(&self, id: &str, mut req: Request) -> Result<Vec<Action>, String> {
        if let Some(s) = id.strip_prefix("snippet:").and_then(|sid| self.manifest.snippets.iter().find(|s| s.id == sid)) {
            return Ok(vec![Action::InsertMarkdown { markdown: s.markdown.clone() }]);
        }
        if !self.manifest.commands.iter().any(|c| c.id == id) {
            return Err(format!("{} has no command {id}", self.manifest.name));
        }
        let bytes = self.wasm.as_ref().ok_or("this plugin has no code")?;
        req.command = id.to_string();
        if req.text.chars().count() > MAX_REQUEST_TEXT {
            req.text = req.text.chars().take(MAX_REQUEST_TEXT).collect();
        }
        let input = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
        let out = call(bytes, &input)?;
        parse_response(&out)
    }
}

fn parse_response(out: &[u8]) -> Result<Vec<Action>, String> {
    let r: Response = serde_json::from_slice(out).map_err(|e| format!("the plugin answered with something that is not valid: {e}"))?;
    if r.actions.len() > MAX_ACTIONS {
        return Err("the plugin asked for too many actions".into());
    }
    for a in &r.actions {
        let n = match a {
            Action::InsertText { text } | Action::ReplaceSelection { text } | Action::Status { text } => text.len(),
            Action::InsertMarkdown { markdown } => markdown.len(),
        };
        if n > MAX_TEXT {
            return Err("the plugin produced too much text".into());
        }
    }
    Ok(r.actions)
}

struct Host {
    limits: StoreLimits,
}

fn call(wasm: &[u8], input: &[u8]) -> Result<Vec<u8>, String> {
    let mut cfg = Config::default();
    cfg.consume_fuel(true);
    let engine = Engine::new(&cfg);
    let module = Module::new(&engine, wasm).map_err(|e| e.to_string())?;
    if module.imports().next().is_some() {
        return Err("plugins may not import anything".into());
    }
    let mut store = Store::new(&engine, Host { limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(1).memories(1).tables(4).build() });
    store.limiter(|h| &mut h.limits);
    store.set_fuel(FUEL).map_err(|e| e.to_string())?;
    let linker: Linker<Host> = Linker::new(&engine);
    let inst = linker.instantiate_and_start(&mut store, &module).map_err(|e| trap(e.to_string()))?;
    let mem = inst.get_memory(&store, "memory").ok_or("the plugin does not export `memory`")?;
    let alloc = inst.get_typed_func::<i32, i32>(&store, "alloc").map_err(|_| "the plugin does not export `alloc(len: i32) -> i32`".to_string())?;
    let run = inst.get_typed_func::<(i32, i32), i64>(&store, "scrawl_run").map_err(|_| "the plugin does not export `scrawl_run(ptr: i32, len: i32) -> i64`".to_string())?;
    let len = i32::try_from(input.len()).map_err(|_| "request too large".to_string())?;
    let ptr = alloc.call(&mut store, len).map_err(|e| trap(e.to_string()))?;
    mem.write(&mut store, usize::try_from(ptr).map_err(|_| "bad pointer")?, input).map_err(|_| "the plugin gave an invalid buffer".to_string())?;
    let packed = run.call(&mut store, (ptr, len)).map_err(|e| trap(e.to_string()))? as u64;
    let (op, ol) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    if ol > 4 * MAX_TEXT {
        return Err("the plugin answer is too large".into());
    }
    let mut out = vec![0u8; ol];
    mem.read(&store, op, &mut out).map_err(|_| "the plugin returned an invalid buffer".to_string())?;
    Ok(out)
}

fn trap(msg: String) -> String {
    if msg.to_lowercase().contains("fuel") {
        "the plugin ran too long and was stopped".into()
    } else {
        format!("the plugin failed: {msg}")
    }
}

/// Load every plugin folder under `dir`. Returns the plugins and a message for each folder that failed.
pub fn load_all(dir: &Path) -> (Vec<Plugin>, Vec<String>) {
    let (mut ok, mut errors) = (Vec::new(), Vec::new());
    let Ok(rd) = std::fs::read_dir(dir) else { return (ok, errors) };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.is_symlink()).collect();
    dirs.sort();
    for d in dirs.into_iter().take(100) {
        match Plugin::load(&d) {
            Ok(p) if ok.iter().any(|q: &Plugin| q.manifest.name == p.manifest.name) => errors.push(format!("{}: a plugin named {} is already loaded", d.display(), p.manifest.name)),
            Ok(p) => ok.push(p),
            Err(e) => errors.push(format!("{}: {e}", d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
        }
    }
    (ok, errors)
}

#[cfg(test)]
mod tests;
