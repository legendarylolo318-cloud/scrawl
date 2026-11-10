//! Serialize an `Expr` to Typst math (the text between `$ ... $`).

use crate::expr::*;

const TYPST_FUNCS: &[&str] = &[
    "arccos", "arcsin", "arctan", "arg", "cos", "cosh", "cot", "coth", "csc", "deg", "det", "dim", "exp", "gcd",
    "im", "inf", "ker", "lg", "lim", "ln", "log", "max", "min", "sec", "sin", "sinh", "sup", "tan", "tanh",
];

fn sym(s: &str) -> String {
    match s {
        // characters that are syntax in Typst math need names
        "∑" => "sum".into(),
        "∏" => "product".into(),
        "∫" => "integral".into(),
        "′" => "'".into(),
        "″" => "''".into(),
        "∂" => "partial".into(),
        "∇" => "nabla".into(),
        "∞" => "infinity".into(),
        "√" => "sqrt".into(),
        "∅" => "emptyset".into(),
        "|" => "bar.v".into(),
        "‖" => "bar.v.double".into(),
        "∥" => "parallel".into(),
        "⟨" => "angle.l".into(),
        "⟩" => "angle.r".into(),
        "ᵀ" => "upright(T)".into(),
        "/" => "slash".into(),
        "&" => "amp".into(),
        "ℝ" => "RR".into(),
        "ℂ" => "CC".into(),
        "ℕ" => "NN".into(),
        "ℤ" => "ZZ".into(),
        "ℚ" => "QQ".into(),
        other => other.to_string(),
    }
}

fn group(e: &Expr) -> String {
    // Wrap in parens when the child is not a single token.
    match e {
        Expr::Sym(_) | Expr::Num(_) if e_len1(e) => ser(e),
        _ => format!("({})", ser(e)),
    }
}

fn e_len1(e: &Expr) -> bool {
    match e {
        Expr::Sym(s) | Expr::Num(s) => s.chars().count() == 1,
        _ => false,
    }
}

fn big_name(op: BigOp) -> &'static str {
    match op {
        BigOp::Sum => "sum",
        BigOp::Prod => "product",
        BigOp::Int => "integral",
        BigOp::IInt => "integral.double",
        BigOp::IIInt => "integral.triple",
        BigOp::OInt => "integral.cont",
        BigOp::OIInt => "integral.surf",
        BigOp::Lim => "lim",
    }
}

fn accent_name(a: Accent) -> &'static str {
    match a {
        Accent::Arrow => "arrow",
        Accent::Hat => "hat",
        Accent::Bold => "bold",
        Accent::Bar => "macron",
        Accent::Tilde => "tilde",
        Accent::Dot => "dot",
        Accent::DDot => "dot.double",
    }
}

pub fn ser(e: &Expr) -> String {
    match e {
        Expr::Row(v) => v.iter().map(ser).collect::<Vec<_>>().join(" "),
        Expr::Sym(s) => sym(s),
        Expr::Num(s) => s.clone(),
        Expr::Func(f) if TYPST_FUNCS.contains(&f.as_str()) => f.clone(),
        Expr::Func(f) => format!("op(\"{f}\")"),
        Expr::Unit(u) => format!("upright(\"{u}\")"),
        Expr::Text(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
        Expr::Frac(a, b) => format!("frac({}, {})", ser(a), ser(b)),
        Expr::Sqrt { index: None, body } => format!("sqrt({})", ser(body)),
        Expr::Sqrt { index: Some(i), body } => format!("root({}, {})", ser(i), ser(body)),
        Expr::Sup(b, x) => format!("{}^{}", ser_base(b), group(x)),
        Expr::Sub(b, x) => format!("{}_{}", ser_base(b), group(x)),
        Expr::SubSup(b, lo, hi) => format!("{}_{}^{}", ser_base(b), group(lo), group(hi)),
        Expr::Big { op, lower, upper, body } => {
            let mut s = big_name(*op).to_string();
            if let Some(l) = lower {
                s += &format!("_{}", group(l));
            }
            if let Some(u) = upper {
                s += &format!("^{}", group(u));
            }
            if !body.is_empty() {
                s += &format!(" {}", ser(body));
            }
            s
        }
        Expr::Matrix { kind, rows, augment } => {
            let body = rows
                .iter()
                .map(|r| r.iter().map(ser).collect::<Vec<_>>().join(", "))
                .collect::<Vec<_>>()
                .join("; ");
            let delim = match kind {
                MatrixKind::Paren => None,
                MatrixKind::Bracket => Some("\"[\""),
                MatrixKind::Det => Some("\"|\""),
                MatrixKind::Norm => Some("\"||\""),
                MatrixKind::Brace => Some("\"{\""),
            };
            let mut opts = String::new();
            if let Some(d) = delim {
                opts += &format!("delim: {d}, ");
            }
            if let Some(a) = augment {
                opts += &format!("augment: #{a}, ");
            }
            format!("mat({opts}{body})")
        }
        Expr::Accent(a, b) => format!("{}({})", accent_name(*a), ser(b)),
        Expr::Fence(f, b) => match f {
            Fence::Paren => format!("({})", ser(b)),
            Fence::Bracket => format!("[{}]", ser(b)),
            Fence::Brace => format!("{{{}}}", ser(b)),
            Fence::Abs => format!("abs({})", ser(b)),
            Fence::Norm => format!("norm({})", ser(b)),
            Fence::Angle => format!("angle.l {} angle.r", ser(b)),
            Fence::Floor => format!("floor({})", ser(b)),
            Fence::Ceil => format!("ceil({})", ser(b)),
        },
        Expr::Cases(rows) => {
            let body: Vec<String> = rows.iter().map(|(v, c)| format!("{} quad {}", ser(v), ser(c))).collect();
            format!("cases({})", body.join(", "))
        }
        Expr::Deriv { partial, order, var, body } => {
            let d = if *partial { "partial" } else { "d" };
            let (num, den) = match order {
                Some(n) => (format!("{d}^{}", group(n)), format!("{d} {}^{}", ser(var), group(n))),
                None => (d.to_string(), format!("{d} {}", ser(var))),
            };
            format!("frac({num}, {den}) {}", ser(body))
        }
        Expr::EvalBar { body, lower, upper } => {
            let mut s = format!("{} bar.v", ser(body));
            if let Some(l) = lower {
                s += &format!("_{}", group(l));
            }
            if let Some(u) = upper {
                s += &format!("^{}", group(u));
            }
            s
        }
        Expr::Raw(r) => r.clone(),
        Expr::Align(lines) => lines
            .iter()
            .map(|l| {
                let mut s = format!("{} &{} {}", ser(&l.lhs), sym(&l.rel), ser(&l.rhs));
                if let Some(n) = &l.note {
                    s += &format!(" quad \"({})\"", n.replace('"', "'"));
                }
                s
            })
            .collect::<Vec<_>>()
            .join(" \\\n"),
    }
}

/// A base that is itself a script or big operator needs grouping before another script.
fn ser_base(e: &Expr) -> String {
    match e {
        Expr::Sym(_) | Expr::Num(_) | Expr::Func(_) | Expr::Accent(..) | Expr::Fence(..) | Expr::Matrix { .. } => ser(e),
        _ => format!("({})", ser(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Expr::*;

    fn b(e: Expr) -> Box<Expr> {
        Box::new(e)
    }

    #[test]
    fn basics() {
        let e = Sup(b(Expr::sym("x")), b(Expr::num("2")));
        assert_eq!(ser(&e), "x^2");
        let e = Frac(b(Expr::sym("a")), b(Row(vec![Expr::sym("b"), Expr::sym("+"), Expr::num("1")])));
        assert_eq!(ser(&e), "frac(a, b + 1)");
        assert_eq!(ser(&Sup(b(Expr::sym("e")), b(Row(vec![Expr::sym("−"), Expr::sym("x")])))), "e^(− x)");
    }

    #[test]
    fn big_operators() {
        let e = Big {
            op: BigOp::Sum,
            lower: Some(b(Row(vec![Expr::sym("i"), Expr::sym("="), Expr::num("0")]))),
            upper: Some(b(Expr::sym("n"))),
            body: b(Sub(b(Expr::sym("a")), b(Expr::sym("i")))),
        };
        assert_eq!(ser(&e), "sum_(i = 0)^n a_i");
        let e = Big { op: BigOp::Int, lower: Some(b(Expr::num("0"))), upper: Some(b(Expr::sym("∞"))), body: b(Expr::empty()) };
        assert_eq!(ser(&e), "integral_0^infinity");
    }

    #[test]
    fn matrices_and_funcs() {
        let m = Matrix { kind: MatrixKind::Det, rows: vec![vec![Expr::num("1"), Expr::num("2")], vec![Expr::num("3"), Expr::num("4")]], augment: None };
        assert_eq!(ser(&m), "mat(delim: \"|\", 1, 2; 3, 4)");
        let m = Matrix { kind: MatrixKind::Bracket, rows: vec![vec![Expr::num("1"), Expr::num("0")]], augment: Some(1) };
        assert_eq!(ser(&m), "mat(delim: \"[\", augment: #1, 1, 0)");
        assert_eq!(ser(&Func("sin".into())), "sin");
        assert_eq!(ser(&Func("rank".into())), "op(\"rank\")");
        assert_eq!(ser(&Unit("kg".into())), "upright(\"kg\")");
    }

    #[test]
    fn align_with_note() {
        let e = Align(vec![
            AlignLine { lhs: Expr::sym("x"), rel: "=".into(), rhs: Expr::num("1"), note: None },
            AlignLine { lhs: Expr::empty(), rel: "=".into(), rhs: Expr::num("2"), note: Some("simplify".into()) },
        ]);
        assert_eq!(ser(&e), "x &= 1 \\\n &= 2 quad \"(simplify)\"");
    }
}
