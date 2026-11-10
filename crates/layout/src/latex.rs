//! Serialize an `Expr` to LaTeX math.

use crate::expr::*;

fn sym(s: &str) -> String {
    let m = match s {
        "α" => "\\alpha", "β" => "\\beta", "γ" => "\\gamma", "δ" => "\\delta", "ε" => "\\varepsilon",
        "ϵ" => "\\epsilon", "ζ" => "\\zeta", "η" => "\\eta", "θ" => "\\theta", "ϑ" => "\\vartheta",
        "ι" => "\\iota", "κ" => "\\kappa", "λ" => "\\lambda", "μ" => "\\mu", "ν" => "\\nu", "ξ" => "\\xi",
        "π" => "\\pi", "ρ" => "\\rho", "σ" => "\\sigma", "ς" => "\\varsigma", "τ" => "\\tau", "υ" => "\\upsilon",
        "φ" => "\\varphi", "ϕ" => "\\phi", "χ" => "\\chi", "ψ" => "\\psi", "ω" => "\\omega", "ϖ" => "\\varpi",
        "ϰ" => "\\varkappa", "ϱ" => "\\varrho",
        "Γ" => "\\Gamma", "Δ" => "\\Delta", "Θ" => "\\Theta", "Λ" => "\\Lambda", "Ξ" => "\\Xi", "Π" => "\\Pi",
        "Σ" => "\\Sigma", "Φ" => "\\Phi", "Ψ" => "\\Psi", "Ω" => "\\Omega",
        "−" => "-", "×" => "\\times", "·" => "\\cdot", "÷" => "\\div", "±" => "\\pm", "∓" => "\\mp",
        "∘" => "\\circ", "⊗" => "\\otimes", "⊕" => "\\oplus", "∗" => "\\ast",
        "≠" => "\\neq", "≈" => "\\approx", "≡" => "\\equiv", "∝" => "\\propto", "≤" => "\\leq", "≥" => "\\geq",
        "≪" => "\\ll", "≫" => "\\gg", "∼" => "\\sim", "≅" => "\\cong",
        "∂" => "\\partial", "∇" => "\\nabla", "∞" => "\\infty", "′" => "'", "″" => "''", "√" => "\\surd",
        "∈" => "\\in", "∉" => "\\notin", "⊂" => "\\subset", "⊆" => "\\subseteq", "⊃" => "\\supset",
        "⊇" => "\\supseteq", "∪" => "\\cup", "∩" => "\\cap", "∅" => "\\emptyset", "∖" => "\\setminus",
        "∀" => "\\forall", "∃" => "\\exists", "∄" => "\\nexists", "¬" => "\\neg", "∧" => "\\wedge", "∨" => "\\vee",
        "⇒" => "\\Rightarrow", "⇔" => "\\Leftrightarrow", "→" => "\\to", "←" => "\\leftarrow",
        "↔" => "\\leftrightarrow", "↦" => "\\mapsto", "∴" => "\\therefore", "∵" => "\\because",
        "ℝ" => "\\mathbb{R}", "ℂ" => "\\mathbb{C}", "ℕ" => "\\mathbb{N}", "ℤ" => "\\mathbb{Z}", "ℚ" => "\\mathbb{Q}",
        "ᵀ" => "^{\\mathsf{T}}", "∥" => "\\parallel", "⊥" => "\\perp", "∠" => "\\angle", "⟨" => "\\langle",
        "⟩" => "\\rangle", "|" => "|", "‖" => "\\|", "°" => "^\\circ", "ℓ" => "\\ell", "ħ" => "\\hbar",
        "Å" => "\\text{\\AA}", "∑" => "\\sum", "∏" => "\\prod", "∫" => "\\int",
        "{" => "\\{", "}" => "\\}", "⌊" => "\\lfloor", "⌋" => "\\rfloor", "⌈" => "\\lceil", "⌉" => "\\rceil",
        "%" => "\\%", "&" => "\\&", "#" => "\\#", "$" => "\\$", "_" => "\\_",
        other => return other.to_string(),
    };
    m.to_string()
}

fn braces(e: &Expr) -> String {
    format!("{{{}}}", ser(e))
}

fn big_name(op: BigOp) -> &'static str {
    match op {
        BigOp::Sum => "\\sum",
        BigOp::Prod => "\\prod",
        BigOp::Int => "\\int",
        BigOp::IInt => "\\iint",
        BigOp::IIInt => "\\iiint",
        BigOp::OInt => "\\oint",
        BigOp::OIInt => "\\oiint",
        BigOp::Lim => "\\lim",
    }
}

pub fn ser(e: &Expr) -> String {
    match e {
        Expr::Row(v) => v.iter().map(ser).collect::<Vec<_>>().join(" "),
        Expr::Sym(s) => sym(s),
        Expr::Num(s) => s.clone(),
        Expr::Func(f) => match f.as_str() {
            "sin" | "cos" | "tan" | "sec" | "csc" | "cot" | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh"
            | "tanh" | "ln" | "log" | "exp" | "det" | "dim" | "ker" | "max" | "min" | "lim" => format!("\\{f}"),
            other => format!("\\operatorname{{{other}}}"),
        },
        Expr::Unit(u) => format!("\\mathrm{{{u}}}"),
        Expr::Text(t) => format!("\\text{{{t}}}"),
        Expr::Frac(a, b) => format!("\\frac{}{}", braces(a), braces(b)),
        Expr::Sqrt { index: None, body } => format!("\\sqrt{}", braces(body)),
        Expr::Sqrt { index: Some(i), body } => format!("\\sqrt[{}]{}", ser(i), braces(body)),
        Expr::Sup(b, x) => format!("{}^{}", base(b), braces(x)),
        Expr::Sub(b, x) => format!("{}_{}", base(b), braces(x)),
        Expr::SubSup(b, lo, hi) => format!("{}_{}^{}", base(b), braces(lo), braces(hi)),
        Expr::Big { op, lower, upper, body } => {
            let mut s = big_name(*op).to_string();
            if let Some(l) = lower {
                s += &format!("_{}", braces(l));
            }
            if let Some(u) = upper {
                s += &format!("^{}", braces(u));
            }
            if !body.is_empty() {
                s += &format!(" {}", ser(body));
            }
            s
        }
        Expr::Matrix { kind, rows, augment } => {
            let env = match kind {
                MatrixKind::Paren => "pmatrix",
                MatrixKind::Bracket => "bmatrix",
                MatrixKind::Det => "vmatrix",
                MatrixKind::Norm => "Vmatrix",
                MatrixKind::Brace => "Bmatrix",
            };
            let body = rows
                .iter()
                .map(|r| r.iter().map(ser).collect::<Vec<_>>().join(" & "))
                .collect::<Vec<_>>()
                .join(" \\\\ ");
            match augment {
                None => format!("\\begin{{{env}}} {body} \\end{{{env}}}"),
                Some(a) => {
                    // Augmented matrices need an array with a rule before column `a`.
                    let cols = rows.first().map_or(0, Vec::len);
                    let spec: String = (0..cols).map(|i| if i == *a && i > 0 { "|c" } else { "c" }).collect();
                    let (l, r) = match kind {
                        MatrixKind::Paren => ("\\left(", "\\right)"),
                        MatrixKind::Bracket => ("\\left[", "\\right]"),
                        MatrixKind::Det => ("\\left|", "\\right|"),
                        MatrixKind::Norm => ("\\left\\|", "\\right\\|"),
                        MatrixKind::Brace => ("\\left\\{", "\\right\\}"),
                    };
                    format!("{l}\\begin{{array}}{{{spec}}} {body} \\end{{array}}{r}")
                }
            }
        }
        Expr::Accent(a, b) => {
            let cmd = match a {
                Accent::Arrow => "\\vec",
                Accent::Hat => "\\hat",
                Accent::Bold => "\\mathbf",
                Accent::Bar => "\\bar",
                Accent::Tilde => "\\tilde",
                Accent::Dot => "\\dot",
                Accent::DDot => "\\ddot",
            };
            format!("{cmd}{}", braces(b))
        }
        Expr::Fence(f, b) => {
            let (l, r) = match f {
                Fence::Paren => ("(", ")"),
                Fence::Bracket => ("[", "]"),
                Fence::Brace => ("\\{", "\\}"),
                Fence::Abs => ("|", "|"),
                Fence::Norm => ("\\|", "\\|"),
                Fence::Angle => ("\\langle", "\\rangle"),
                Fence::Floor => ("\\lfloor", "\\rfloor"),
                Fence::Ceil => ("\\lceil", "\\rceil"),
            };
            format!("\\left{l} {} \\right{r}", ser(b))
        }
        Expr::Cases(rows) => {
            let body: Vec<String> = rows.iter().map(|(v, c)| format!("{} & {}", ser(v), ser(c))).collect();
            format!("\\begin{{cases}} {} \\end{{cases}}", body.join(" \\\\ "))
        }
        Expr::Deriv { partial, order, var, body } => {
            let d = if *partial { "\\partial" } else { "d" };
            let (n, den) = match order {
                Some(o) => (format!("{d}^{}", braces(o)), format!("{d} {}^{}", ser(var), braces(o))),
                None => (d.to_string(), format!("{d} {}", ser(var))),
            };
            format!("\\frac{{{n}}}{{{den}}} {}", ser(body))
        }
        Expr::EvalBar { body, lower, upper } => {
            let mut s = format!("\\left. {} \\right|", ser(body));
            if let Some(l) = lower {
                s += &format!("_{}", braces(l));
            }
            if let Some(u) = upper {
                s += &format!("^{}", braces(u));
            }
            s
        }
        Expr::Raw(r) => r.clone(),
        Expr::Align(lines) => {
            let body: Vec<String> = lines
                .iter()
                .map(|l| {
                    let mut s = format!("{} &{} {}", ser(&l.lhs), sym(&l.rel), ser(&l.rhs));
                    if let Some(n) = &l.note {
                        s += &format!(" \\quad \\text{{({n})}}");
                    }
                    s
                })
                .collect();
            format!("\\begin{{aligned}} {} \\end{{aligned}}", body.join(" \\\\ "))
        }
    }
}

fn base(e: &Expr) -> String {
    match e {
        Expr::Sym(_) | Expr::Num(_) | Expr::Func(_) | Expr::Accent(..) | Expr::Fence(..) | Expr::Matrix { .. } => ser(e),
        _ => format!("{{{}}}", ser(e)),
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
    fn greek_frac_sqrt() {
        let e = Frac(b(Expr::sym("α")), b(Sqrt { index: None, body: b(Expr::sym("x")) }));
        assert_eq!(ser(&e), "\\frac{\\alpha}{\\sqrt{x}}");
        let e = Sqrt { index: Some(b(Expr::num("3"))), body: b(Expr::sym("x")) };
        assert_eq!(ser(&e), "\\sqrt[3]{x}");
    }

    #[test]
    fn integral_and_scripts() {
        let e = Big {
            op: BigOp::Int,
            lower: Some(b(Expr::num("0"))),
            upper: Some(b(Expr::sym("∞"))),
            body: b(Row(vec![Sup(b(Expr::sym("e")), b(Row(vec![Expr::sym("−"), Expr::sym("x")]))), Expr::sym("d"), Expr::sym("x")])),
        };
        assert_eq!(ser(&e), "\\int_{0}^{\\infty} e^{- x} d x");
        // script on a script is grouped
        let e = Sup(b(Sub(b(Expr::sym("x")), b(Expr::sym("i")))), b(Expr::num("2")));
        assert_eq!(ser(&e), "{x_{i}}^{2}");
    }

    #[test]
    fn matrices() {
        let m = Matrix { kind: MatrixKind::Bracket, rows: vec![vec![Expr::num("1"), Expr::num("2")], vec![Expr::num("3"), Expr::num("4")]], augment: None };
        assert_eq!(ser(&m), "\\begin{bmatrix} 1 & 2 \\\\ 3 & 4 \\end{bmatrix}");
        let m = Matrix { kind: MatrixKind::Bracket, rows: vec![vec![Expr::num("1"), Expr::num("2"), Expr::num("3")]], augment: Some(2) };
        assert_eq!(ser(&m), "\\left[\\begin{array}{c c|c} 1 & 2 & 3 \\end{array}\\right]".replace("c c|c", "cc|c"));
    }
}
