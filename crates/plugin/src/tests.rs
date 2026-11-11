use super::*;

/// A plugin written by hand in WebAssembly text: it answers every request with a fixed response.
const FIXED: &str = r#"
(module
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const 1024))
  (data (i32.const 64) "{\"actions\":[{\"op\":\"status\",\"text\":\"hello from wasm\"},{\"op\":\"insert_text\",\"text\":\"!\"}]}")
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  (func (export "scrawl_run") (param i32 i32) (result i64)
    (i64.or (i64.shl (i64.const 64) (i64.const 32)) (i64.const 86))))
"#;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("scrawl-plugin-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_plugin(dir: &Path, manifest: &str, wat: Option<&str>) {
    std::fs::write(dir.join("plugin.json"), manifest).unwrap();
    if let Some(w) = wat {
        std::fs::write(dir.join("plugin.wat"), w).unwrap();
    }
}

const MANIFEST: &str = r##"{"name":"hello","version":"1.0.0","wasm":"plugin.wat","commands":[{"id":"hi","title":"Say hi"}],"snippets":[{"id":"meeting","title":"Meeting","markdown":"# Meeting\n\n- [ ] item"}]}"##;

#[test]
fn a_wasm_plugin_runs_and_returns_actions() {
    let d = tmp("run");
    write_plugin(&d, MANIFEST, Some(FIXED));
    let p = Plugin::load(&d).unwrap();
    let a = p.run("hi", Request { selection: "x".into(), ..Request::default() }).unwrap();
    assert_eq!(a, vec![Action::Status { text: "hello from wasm".into() }, Action::InsertText { text: "!".into() }]);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn snippets_need_no_code_and_unknown_commands_are_refused() {
    let d = tmp("snip");
    write_plugin(&d, r#"{"name":"snips","snippets":[{"id":"a","title":"A","markdown":"- [ ] x"}]}"#, None);
    let p = Plugin::load(&d).unwrap();
    assert_eq!(p.run("snippet:a", Request::default()).unwrap(), vec![Action::InsertMarkdown { markdown: "- [ ] x".into() }]);
    assert!(p.run("snippet:nope", Request::default()).is_err());
    assert!(p.run("hi", Request::default()).is_err());
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn a_plugin_that_loops_forever_is_stopped() {
    let d = tmp("loop");
    let wat = r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "scrawl_run") (param i32 i32) (result i64) (loop $l (br $l)) (i64.const 0)))"#;
    write_plugin(&d, r#"{"name":"spin","wasm":"plugin.wat","commands":[{"id":"go","title":"Go"}]}"#, Some(wat));
    let started = std::time::Instant::now();
    let err = Plugin::load(&d).unwrap().run("go", Request::default()).unwrap_err();
    assert!(err.contains("too long"), "{err}");
    assert!(started.elapsed().as_secs() < 20);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn a_plugin_that_eats_memory_is_stopped() {
    let d = tmp("mem");
    let wat = r#"(module (memory (export "memory") 1)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "scrawl_run") (param i32 i32) (result i64)
        (drop (memory.grow (i32.const 60000)))
        (i32.store (i32.const 4000000000) (i32.const 1))
        (i64.const 0)))"#;
    write_plugin(&d, r#"{"name":"hog","wasm":"plugin.wat","commands":[{"id":"go","title":"Go"}]}"#, Some(wat));
    assert!(Plugin::load(&d).unwrap().run("go", Request::default()).is_err());
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn imports_are_refused_so_there_is_no_way_out_of_the_sandbox() {
    let d = tmp("imp");
    let wat = r#"(module (import "env" "read_file" (func (param i32) (result i32))) (memory (export "memory") 1))"#;
    write_plugin(&d, r#"{"name":"spy","wasm":"plugin.wat","commands":[{"id":"go","title":"Go"}]}"#, Some(wat));
    let err = Plugin::load(&d).err().unwrap();
    assert!(err.contains("may not import"), "{err}");
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn bad_manifests_and_answers_are_rejected_with_a_reason() {
    for (m, why) in [
        (r#"{"name":"Has Space"}"#, "name"),
        (r#"{"name":"a","commands":[{"id":"x","title":"X"}]}"#, "wasm"),
        (r#"{"name":"a","wasm":"../evil.wasm"}"#, "inside the plugin folder"),
        (r#"{"name":"a","wasm":"/etc/passwd"}"#, "inside the plugin folder"),
        (r#"{"name":"a","wasm":"sub/p.wasm"}"#, "inside the plugin folder"),
        (r#"{"name":"a","snippets":[{"id":"x","title":"X","markdown":""},{"id":"x","title":"Y","markdown":""}]}"#, "twice"),
        (r#"{"name":"a","snippets":[{"id":"BAD","title":"X","markdown":""}]}"#, "command id"),
        ("not json", "plugin.json"),
    ] {
        let e = Manifest::parse(m).unwrap_err();
        assert!(e.contains(why), "{m}: {e}");
    }
    assert!(parse_response(br#"{"actions":[{"op":"delete_everything"}]}"#).is_err());
    assert!(parse_response(b"garbage").is_err());
    assert_eq!(parse_response(b"{}").unwrap(), vec![]);
    let many = format!("{{\"actions\":[{}]}}", vec![r#"{"op":"status","text":"x"}"#; 101].join(","));
    assert!(parse_response(many.as_bytes()).is_err());
}

#[test]
fn load_all_collects_good_plugins_and_reports_bad_ones() {
    let root = tmp("all");
    for (n, ok) in [("good", true), ("bad", false), ("dup", true)] {
        let d = root.join(n);
        std::fs::create_dir_all(&d).unwrap();
        if ok {
            write_plugin(&d, r#"{"name":"same","snippets":[{"id":"a","title":"A","markdown":"x"}]}"#, None);
        } else {
            std::fs::write(d.join("plugin.json"), "{").unwrap();
        }
    }
    std::fs::write(root.join("loose-file.txt"), "ignored").unwrap();
    let (ok, errors) = load_all(&root);
    assert_eq!(ok.len(), 1);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(load_all(&root.join("missing")).0.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn the_shipped_hello_example_loads_and_runs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/hello-wat");
    let p = Plugin::load(&dir).unwrap();
    let a = p.run("hello", Request::default()).unwrap();
    assert_eq!(a, vec![Action::Status { text: "hello from wasm".into() }, Action::InsertText { text: "!".into() }]);
}

#[test]
fn the_shipped_text_tools_manifest_is_valid() {
    let m = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/text-tools/plugin.json")).unwrap();
    let m = Manifest::parse(&m).unwrap();
    assert_eq!(m.commands.len(), 5);
}

fn run_wat(name: &str, wat: &str) -> (Result<Vec<Action>, String>, std::time::Duration) {
    let d = tmp(name);
    write_plugin(&d, r#"{"name":"evil","wasm":"plugin.wat","commands":[{"id":"go","title":"Go"}]}"#, Some(wat));
    let t = std::time::Instant::now();
    let r = Plugin::load(&d).and_then(|p| p.run("go", Request::default()));
    let el = t.elapsed();
    std::fs::remove_dir_all(d).ok();
    (r, el)
}

#[test]
fn a_plugin_with_a_huge_table_is_refused_quickly_without_eating_memory() {
    // Before the fix this allocated ~16 GB and blocked for seconds (no table size limit).
    let (r, el) = run_wat("bigtable", r#"(module (memory (export "memory") 1) (table 4294967295 funcref)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0)))"#);
    assert!(r.is_err());
    assert!(el.as_millis() < 500, "took {el:?}");
    let (r, _) = run_wat("growtable", r#"(module (memory (export "memory") 1) (table 1 funcref)
      (func (export "alloc") (param i32) (result i32) (i32.const 0))
      (func (export "scrawl_run") (param i32 i32) (result i64)
        (if (i32.ne (table.grow (ref.null func) (i32.const 100000000)) (i32.const -1)) (then unreachable)) (i64.const 0)))"#);
    assert!(r.unwrap_err().contains("not valid"), "table.grow must fail, not trap");
}

#[test]
fn hostile_modules_fail_cleanly_and_fast() {
    for (n, w, why) in [
        ("startloop", r#"(module (memory (export "memory") 1) (func $s (loop $l (br $l))) (start $s)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0)))"#, "too long"),
        ("badalloc", r#"(module (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const 65530))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0)))"#, "invalid buffer"),
        ("negalloc", r#"(module (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const -5))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0)))"#, "pointer"),
        ("neglen", r#"(module (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const -1)))"#, "too large"),
        ("outside", r#"(module (memory (export "memory") 1)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0x0000FFFF00000010)))"#, "invalid buffer"),
        ("recurse", r#"(module (memory (export "memory") 1) (func $r (call $r))
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scrawl_run") (param i32 i32) (result i64) (call $r) (i64.const 0)))"#, "failed"),
        ("bigmem", r#"(module (memory (export "memory") 2000)
          (func (export "alloc") (param i32) (result i32) (i32.const 0))
          (func (export "scrawl_run") (param i32 i32) (result i64) (i64.const 0)))"#, ""),
    ] {
        let (r, el) = run_wat(n, w);
        let e = r.expect_err(n);
        assert!(e.contains(why), "{n}: {e}");
        // 50 M fuel is ~35 ms in release; allow plenty for debug builds
        assert!(el.as_secs() < 5, "{n} took {el:?}");
    }
}

#[cfg(unix)]
#[test]
fn the_module_cannot_be_reached_through_a_symlinked_folder() {
    let outside = tmp("outside");
    std::fs::write(outside.join("p.wat"), FIXED).unwrap();
    let d = tmp("symdir");
    std::os::unix::fs::symlink(&outside, d.join("sub")).unwrap();
    write_plugin(&d, r#"{"name":"sneaky","wasm":"sub/p.wat","commands":[{"id":"hi","title":"Hi"}]}"#, None);
    assert!(Plugin::load(&d).is_err(), "a module outside the plugin folder must not load");
    std::fs::remove_dir_all(d).ok();
    std::fs::remove_dir_all(outside).ok();
}
