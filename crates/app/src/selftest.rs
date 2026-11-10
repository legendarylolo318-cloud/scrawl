//! `scrawl --self-test`: headless smoke test used by CI on every release artifact.
//! Loads the model, recognizes bundled sample ink, renders a page, round-trips storage, search,
//! export, encryption and the tablet CA, then exits 0 (all good) or 1.

use ink::{InkMode, PenKind, Point, Stroke};

fn stroke(id: u64, t0: u64, a: (f32, f32), b: (f32, f32)) -> Stroke {
    let pts = (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            Point::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, i as u32 * 8)
        })
        .collect();
    Stroke { id, points: pts, color: [255; 4], width: 3.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: t0 }
}

/// Bundled sample ink: "+" and "=" drawn as multi-stroke symbols.
fn samples() -> Vec<(&'static str, Vec<Stroke>)> {
    vec![
        ("+", vec![stroke(1, 0, (0.0, 20.0), (40.0, 20.0)), stroke(2, 300, (20.0, 0.0), (20.0, 40.0))]),
        ("=", vec![stroke(3, 0, (0.0, 0.0), (40.0, 0.0)), stroke(4, 300, (0.0, 14.0), (40.0, 14.0))]),
    ]
}

fn check(name: &str, ok: bool, detail: impl std::fmt::Display) -> bool {
    println!("[{}] {name}: {detail}", if ok { " ok " } else { "FAIL" });
    ok
}

pub fn run() -> i32 {
    let mut all = true;

    // 1. model + recognition of bundled ink
    match recog::Classifier::bundled() {
        Ok(clf) => {
            all &= check("model", clf.labels().len() == 191, format!("{} classes, {} bytes", clf.labels().len(), recog::BUNDLED_MODEL.len()));
            for (want, strokes) in samples() {
                let refs: Vec<&Stroke> = strokes.iter().collect();
                let c = clf.classify(&refs, 5).unwrap_or_default();
                let got: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
                all &= check(&format!("recognize {want}"), got.contains(&want), format!("top5 = {got:?}"));
            }
        }
        Err(e) => all &= check("model", false, e),
    }

    // 2. typeset + render a page
    let e = layout::Expr::Frac(Box::new(layout::Expr::sym("α")), Box::new(layout::Expr::Sup(Box::new(layout::Expr::sym("x")), Box::new(layout::Expr::num("2")))));
    match render::compile(&render::math_source(&e.to_typst(), 14.0, [0, 0, 0]), &[]).ok().and_then(|d| render::png(&d, 0, 2.0)) {
        Some(png) => all &= check("render", png.starts_with(&[0x89, b'P', b'N', b'G']), format!("{} byte PNG", png.len())),
        None => all &= check("render", false, "typst compile failed"),
    }

    // 3. storage + encryption + history + export + search
    let dir = std::env::temp_dir().join(format!("scrawl-selftest-{}", std::process::id()));
    let result = (|| -> Result<(), String> {
        let s = store::Store::open(&dir).map_err(|e| e.to_string())?;
        for vault in [store::Vault::none(), store::Vault::with_password("selftest")] {
            let mut nb = s.create_notebook("Self test", &vault).map_err(|e| e.to_string())?;
            nb.add_section("s", "Section");
            let mut p = store::Page::new("p1", "Gaussian integral", "2026-01-01");
            p.blocks.push(store::Block::Math { expr: e.clone(), source: None });
            s.create_page(&mut nb, "s", p, &vault).map_err(|e| e.to_string())?;
            let back = s.load_page(&nb.id, "p1", &vault).map_err(|e| e.to_string())?;
            if back.title != "Gaussian integral" {
                return Err("page roundtrip mismatch".into());
            }
            let pdf = export::export_page(&back, export::Format::Pdf)?;
            if !pdf.starts_with(b"%PDF") {
                return Err("pdf export".into());
            }
            let mut ix = search::SearchIndex::in_memory().map_err(|e| e.to_string())?;
            ix.add(&search::doc_from_page(&nb.id, &back, "")).map_err(|e| e.to_string())?;
            ix.commit().map_err(|e| e.to_string())?;
            if ix.search("gaussian", &search::Filters::default(), 5).map_err(|e| e.to_string())?.len() != 1 {
                return Err("search".into());
            }
        }
        Ok(())
    })();
    std::fs::remove_dir_all(&dir).ok();
    all &= check("storage/encryption/export/search", result.is_ok(), result.err().unwrap_or_else(|| "ok".into()));

    // 4. typing engine
    let mut ed = input::Editor::default();
    ed.insert_str("alpha + sqrt ");
    ed.insert_str("x");
    all &= check("typing", ed.expr().to_typst() == "α + sqrt(x)", ed.expr().to_typst());

    // 5. tablet CA with name constraints (no network, no keyring)
    let keys = remote::ca::MemStore::default();
    let ca = remote::ca::ensure_ca(&keys, None, store::now_ms());
    let leaf = ca.as_ref().ok().and_then(|c| remote::ca::issue_leaf(&keys, &c.cert_pem, &["192.168.1.2".parse().unwrap()], store::now_ms()).ok());
    all &= check("tablet CA", leaf.is_some(), ca.map(|c| c.fingerprint()).unwrap_or_else(|e| e));

    println!("{}", if all { "SELF-TEST PASSED" } else { "SELF-TEST FAILED" });
    if all {
        0
    } else {
        1
    }
}
