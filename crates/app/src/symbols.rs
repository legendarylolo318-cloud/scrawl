//! The searchable symbol palette: every class the recognizer knows, every typing shorthand,
//! structure templates, accents and upright units, so anything can be entered by mouse alone.

use input::Out;
use layout::{Accent, BigOp, Expr, Fence, MatrixKind};

#[derive(Clone, Debug)]
pub struct Sym {
    pub display: String,
    pub name: String,
    pub out: Insert,
}

#[derive(Clone, Debug)]
pub enum Insert {
    Expr(Expr),
    Template(Out),
}

const UNITS: &[&str] = &["m", "cm", "mm", "km", "s", "ms", "kg", "g", "N", "J", "W", "Pa", "Hz", "V", "A", "Ω", "C", "K", "mol", "eV", "L"];

/// All palette entries, in a sensible browsing order.
pub fn all(model_labels: &[String]) -> Vec<Sym> {
    let table = input::shorthand::defaults();
    let mut out: Vec<Sym> = Vec::new();
    let mut push = |display: String, name: String, ins: Insert| {
        if !out.iter().any(|s| s.display == display) {
            out.push(Sym { display, name, out: ins });
        }
    };
    // structures first: the hardest things to draw
    let templates: Vec<(&str, &str, Out)> = vec![
        ("□⁄□", "fraction", Out::Frac),
        ("√□", "square root", Out::Sqrt),
        ("∫", "integral", Out::Big(BigOp::Int)),
        ("∬", "double integral", Out::Big(BigOp::IInt)),
        ("∭", "triple integral", Out::Big(BigOp::IIInt)),
        ("∮", "contour integral", Out::Big(BigOp::OInt)),
        ("∯", "surface integral", Out::Big(BigOp::OIInt)),
        ("∑", "sum", Out::Big(BigOp::Sum)),
        ("∏", "product", Out::Big(BigOp::Prod)),
        ("lim", "limit", Out::Big(BigOp::Lim)),
        ("d/dx", "derivative", Out::Deriv { partial: false }),
        ("∂/∂x", "partial derivative", Out::Deriv { partial: true }),
        ("(2×2)", "matrix 2x2", Out::Matrix { kind: MatrixKind::Paren, rows: 2, cols: 2 }),
        ("[3×3]", "matrix 3x3", Out::Matrix { kind: MatrixKind::Bracket, rows: 3, cols: 3 }),
        ("|2×2|", "determinant", Out::Matrix { kind: MatrixKind::Det, rows: 2, cols: 2 }),
        ("{ cases", "cases piecewise", Out::Cases),
        ("|□|", "absolute value", Out::Fence(Fence::Abs)),
        ("‖□‖", "norm", Out::Fence(Fence::Norm)),
        ("⟨□⟩", "angle brackets inner product", Out::Fence(Fence::Angle)),
        ("⌊□⌋", "floor", Out::Fence(Fence::Floor)),
        ("⌈□⌉", "ceiling", Out::Fence(Fence::Ceil)),
        ("□|ₐᵇ", "evaluate at bounds", Out::EvalBar),
        ("v⃗", "vector arrow accent", Out::Accent(Accent::Arrow)),
        ("v̂", "hat accent unit vector", Out::Accent(Accent::Hat)),
        ("v̄", "bar accent mean", Out::Accent(Accent::Bar)),
        ("ṽ", "tilde accent", Out::Accent(Accent::Tilde)),
        ("v̇", "dot accent time derivative", Out::Accent(Accent::Dot)),
        ("v̈", "double dot accent", Out::Accent(Accent::DDot)),
        ("𝐯", "bold vector", Out::Accent(Accent::Bold)),
    ];
    for (d, n, o) in templates {
        push(d.into(), n.into(), Insert::Template(o));
    }
    // recognizer classes, named by their shorthand where one exists
    let name_of = |label: &str| -> String {
        table.iter().find(|(_, o)| matches!(o, Out::Sym(s) if s == label)).map(|(k, _)| k.clone()).unwrap_or_default()
    };
    for l in model_labels {
        let n = name_of(l);
        push(l.clone(), n, Insert::Expr(Expr::Sym(l.clone())));
    }
    // shorthand symbols the model cannot draw-recognize (e.g. ≅ variants) and functions
    for (k, o) in &table {
        match o {
            Out::Sym(s) => push(s.clone(), k.clone(), Insert::Expr(Expr::Sym(s.clone()))),
            Out::Func(f) => push(f.clone(), format!("{f} function"), Insert::Expr(Expr::Func(f.clone()))),
            _ => {}
        }
    }
    for u in UNITS {
        push(format!("{u} (unit)"), format!("unit {u}"), Insert::Expr(Expr::Unit(u.to_string())));
    }
    out
}

/// Filter by every query word appearing in the name or being the symbol itself.
pub fn filter(all: &[Sym], q: &str) -> Vec<Sym> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return all.to_vec();
    }
    all.iter().filter(|s| s.display == q || q.split_whitespace().all(|w| s.name.to_lowercase().contains(w) || s.display.to_lowercase().contains(w))).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_covers_every_model_class_and_finds_by_name() {
        let labels: Vec<String> = serde_json::from_str(recog::BUNDLED_LABELS).unwrap();
        let all = all(&labels);
        for l in &labels {
            assert!(all.iter().any(|s| &s.display == l), "{l} missing from the palette");
        }
        let hits = filter(&all, "integral");
        assert!(hits.iter().any(|s| s.display == "∫") && hits.iter().any(|s| s.display == "∮"));
        assert!(filter(&all, "alpha").iter().any(|s| s.display == "α"));
        assert!(filter(&all, "unit kg").iter().any(|s| matches!(&s.out, Insert::Expr(Expr::Unit(u)) if u == "kg")));
        assert!(filter(&all, "parallel").iter().any(|s| s.display == "∥"));
    }
}
