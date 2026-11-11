//! A Scrawl plugin written in Rust. Build and install:
//!
//! ```text
//! rustup target add wasm32-unknown-unknown
//! cargo build --release --target wasm32-unknown-unknown
//! mkdir -p ~/Scrawl/plugins/text-tools
//! cp target/wasm32-unknown-unknown/release/text_tools.wasm plugin.json ~/Scrawl/plugins/text-tools/
//! ```
//!
//! then run "Reload plugins" in Scrawl. The plugin gets a JSON request and answers with JSON actions;
//! it cannot touch files, the network or anything else (see `docs/plugins.md`).

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct Request {
    pub command: String,
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    ReplaceSelection { text: String },
    Status { text: String },
}

#[derive(Serialize)]
pub struct Response {
    pub actions: Vec<Action>,
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = !c.is_alphanumeric() && c != '\'';
        }
    }
    out
}

/// The whole plugin: request JSON in, response JSON out. Plain Rust, so it is unit-tested natively.
pub fn handle(request_json: &str) -> String {
    let status = |t: &str| Response { actions: vec![Action::Status { text: t.into() }] };
    let resp = match serde_json::from_str::<Request>(request_json) {
        Err(e) => status(&format!("text-tools: bad request ({e})")),
        Ok(r) if r.selection.is_empty() && r.command != "count" => status("Select some text first"),
        Ok(r) => match r.command.as_str() {
            "upper" => Response { actions: vec![Action::ReplaceSelection { text: r.selection.to_uppercase() }] },
            "title-case" => Response { actions: vec![Action::ReplaceSelection { text: title_case(&r.selection) }] },
            "sort-lines" => {
                let mut lines: Vec<&str> = r.selection.lines().collect();
                lines.sort_by_key(|l| l.to_lowercase());
                Response { actions: vec![Action::ReplaceSelection { text: lines.join("\n") }] }
            }
            "lines-to-tasks" => {
                let tasks: Vec<String> = r.selection.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("- [ ] {}", l.trim())).collect();
                Response { actions: vec![Action::ReplaceSelection { text: tasks.join("\n") }] }
            }
            "count" => {
                let (what, t) = if r.selection.is_empty() { ("page", &r.text) } else { ("selection", &r.selection) };
                status(&format!("{} words, {} characters in the {what}", t.split_whitespace().count(), t.chars().filter(|c| !c.is_whitespace()).count()))
            }
            other => status(&format!("text-tools has no command {other}")),
        },
    };
    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into())
}

// ---- the WebAssembly interface Scrawl calls (see docs/plugins.md) ------------------------------------

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    let mut buf: Vec<u8> = Vec::with_capacity(len.max(0) as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf); // each call runs in a fresh instance, so leaking is fine
    ptr as i32
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn scrawl_run(ptr: i32, len: i32) -> i64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let out = handle(std::str::from_utf8(input).unwrap_or("{}")).into_bytes().into_boxed_slice();
    let (p, n) = (out.as_ptr() as i32 as i64, out.len() as i64);
    std::mem::forget(out);
    (p << 32) | n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cmd: &str, sel: &str, text: &str) -> String {
        handle(&serde_json::json!({ "command": cmd, "selection": sel, "text": text }).to_string())
    }

    #[test]
    fn commands_answer_with_actions() {
        assert_eq!(run("upper", "héllo", ""), r#"{"actions":[{"op":"replace_selection","text":"HÉLLO"}]}"#);
        assert_eq!(title_case("the quick BROWN fox's tale"), "The Quick Brown Fox's Tale");
        assert_eq!(run("sort-lines", "b\nA\nc", ""), r#"{"actions":[{"op":"replace_selection","text":"A\nb\nc"}]}"#);
        assert_eq!(run("lines-to-tasks", "one\n\n two ", ""), r#"{"actions":[{"op":"replace_selection","text":"- [ ] one\n- [ ] two"}]}"#);
        assert!(run("count", "", "a bc d").contains("3 words"));
        assert!(run("upper", "", "").contains("Select some text first"));
        assert!(run("nope", "x", "").contains("no command"));
        assert!(handle("garbage").contains("bad request"));
    }
}
