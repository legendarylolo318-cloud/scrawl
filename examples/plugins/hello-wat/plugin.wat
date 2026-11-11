;; The smallest Scrawl plugin: ignores the request and answers with a fixed response.
(module
  (memory (export "memory") 2)
  (global $heap (mut i32) (i32.const 1024))
  ;; 86 bytes of JSON at offset 64
  (data (i32.const 64) "{\"actions\":[{\"op\":\"status\",\"text\":\"hello from wasm\"},{\"op\":\"insert_text\",\"text\":\"!\"}]}")
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $n)))
    (local.get $p))
  (func (export "scrawl_run") (param i32 i32) (result i64)
    ;; (pointer << 32) | length
    (i64.or (i64.shl (i64.const 64) (i64.const 32)) (i64.const 86))))
