//! Shorthand table (`alpha` -> α, `->` -> →, `sqrt` -> template ...), user overrides,
//! and fuzzy autocomplete.

use layout::{Accent, BigOp, Fence, MatrixKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a shorthand expands to.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Sym(String),
    Func(String),
    Big(BigOp),
    Sqrt,
    Accent(Accent),
    Fence(Fence),
    Frac,
    Matrix { kind: MatrixKind, rows: usize, cols: usize },
    Cases,
    Deriv { partial: bool },
    EvalBar,
}

const SYMS: &[(&str, &str)] = &[
    // Greek lower
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ϵ"), ("varepsilon", "ε"),
    ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("vartheta", "ϑ"), ("iota", "ι"), ("kappa", "κ"),
    ("lambda", "λ"), ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"), ("pi", "π"), ("rho", "ρ"), ("sigma", "σ"),
    ("varsigma", "ς"), ("tau", "τ"), ("upsilon", "υ"), ("phi", "ϕ"), ("varphi", "φ"), ("chi", "χ"),
    ("psi", "ψ"), ("omega", "ω"),
    // Greek upper
    ("Gamma", "Γ"), ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"),
    ("Sigma", "Σ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"),
    ("hbar", "ħ"), ("ell", "ℓ"), ("angstrom", "Å"), ("deg", "°"),
    // calculus
    ("inf", "∞"), ("infty", "∞"), ("infinity", "∞"), ("nabla", "∇"), ("del", "∇"), ("partial", "∂"),
    // operators
    ("times", "×"), ("cdot", "·"), ("div", "÷"), ("pm", "±"), ("mp", "∓"), ("circ", "∘"), ("otimes", "⊗"),
    ("oplus", "⊕"), ("ast", "∗"), ("+-", "±"), ("-+", "∓"),
    // relations / arrows
    ("->", "→"), ("<-", "←"), ("<->", "↔"), ("=>", "⇒"), ("<=>", "⇔"), ("|->", "↦"), ("!=", "≠"),
    ("<=", "≤"), (">=", "≥"), ("~=", "≈"), ("<<", "≪"), (">>", "≫"), ("neq", "≠"), ("leq", "≤"),
    ("geq", "≥"), ("approx", "≈"), ("equiv", "≡"), ("propto", "∝"), ("cong", "≅"), ("sim", "∼"),
    ("to", "→"), ("mapsto", "↦"), ("implies", "⇒"), ("iff", "⇔"), ("ll", "≪"), ("gg", "≫"),
    // sets & logic
    ("in", "∈"), ("notin", "∉"), ("subset", "⊂"), ("subseteq", "⊆"), ("supset", "⊃"), ("supseteq", "⊇"),
    ("union", "∪"), ("inter", "∩"), ("emptyset", "∅"), ("setminus", "∖"), ("forall", "∀"), ("exists", "∃"),
    ("nexists", "∄"), ("not", "¬"), ("and", "∧"), ("or", "∨"), ("therefore", "∴"), ("because", "∵"),
    ("RR", "ℝ"), ("CC", "ℂ"), ("NN", "ℕ"), ("ZZ", "ℤ"), ("QQ", "ℚ"),
    // linear algebra
    ("perp", "⊥"), ("parallel", "∥"), ("angle", "∠"), ("langle", "⟨"), ("rangle", "⟩"), ("transpose", "ᵀ"),
];

const FUNCS: &[&str] = &[
    "sin", "cos", "tan", "sec", "csc", "cot", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "ln", "log",
    "exp", "det", "tr", "rank", "span", "dim", "ker", "im", "max", "min",
];

pub fn defaults() -> BTreeMap<String, Out> {
    let mut m = BTreeMap::new();
    for (k, v) in SYMS {
        m.insert(k.to_string(), Out::Sym(v.to_string()));
    }
    for f in FUNCS {
        m.insert(f.to_string(), Out::Func(f.to_string()));
    }
    for (k, op) in [
        ("sum", BigOp::Sum), ("prod", BigOp::Prod), ("int", BigOp::Int), ("iint", BigOp::IInt),
        ("iiint", BigOp::IIInt), ("oint", BigOp::OInt), ("oiint", BigOp::OIInt), ("lim", BigOp::Lim),
    ] {
        m.insert(k.into(), Out::Big(op));
    }
    m.insert("sqrt".into(), Out::Sqrt);
    m.insert("frac".into(), Out::Frac);
    for (k, a) in [
        ("vec", Accent::Arrow), ("hat", Accent::Hat), ("bar", Accent::Bar), ("tilde", Accent::Tilde),
        ("dot", Accent::Dot), ("ddot", Accent::DDot), ("bf", Accent::Bold),
    ] {
        m.insert(k.into(), Out::Accent(a));
    }
    for (k, f) in [("abs", Fence::Abs), ("norm", Fence::Norm), ("floor", Fence::Floor), ("ceil", Fence::Ceil), ("ang", Fence::Angle)] {
        m.insert(k.into(), Out::Fence(f));
    }
    m.insert("mat".into(), Out::Matrix { kind: MatrixKind::Paren, rows: 2, cols: 2 });
    m.insert("bmat".into(), Out::Matrix { kind: MatrixKind::Bracket, rows: 2, cols: 2 });
    m.insert("vmat".into(), Out::Matrix { kind: MatrixKind::Det, rows: 2, cols: 2 });
    m.insert("cases".into(), Out::Cases);
    m.insert("ddx".into(), Out::Deriv { partial: false });
    m.insert("pdx".into(), Out::Deriv { partial: true });
    m.insert("eval".into(), Out::EvalBar);
    m
}

/// User-editable overrides stored in settings: name -> unicode text (symbols only).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UserShorthand {
    pub map: BTreeMap<String, String>,
}

pub struct Table {
    entries: BTreeMap<String, Out>,
}

impl Table {
    pub fn new(user: &UserShorthand) -> Self {
        let mut entries = defaults();
        for (k, v) in &user.map {
            if k.is_empty() {
                continue;
            }
            if v.is_empty() {
                entries.remove(k);
            } else {
                entries.insert(k.clone(), Out::Sym(v.clone()));
            }
        }
        Self { entries }
    }

    pub fn get(&self, name: &str) -> Option<Out> {
        if let Some(o) = self.entries.get(name) {
            return Some(o.clone());
        }
        parse_matrix_name(name)
    }

    /// Is `word` a strict prefix of some longer shorthand (so we should wait for more input)?
    pub fn is_strict_prefix(&self, word: &str) -> bool {
        self.entries.keys().any(|k| k.len() > word.len() && k.starts_with(word)) || (parse_matrix_name(word).is_none() && matrix_prefix_ok(word))
    }

    /// Fuzzy autocomplete: prefix matches first, then subsequence matches.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        if query.is_empty() {
            return vec![];
        }
        let mut v: Vec<(i32, Suggestion)> = self
            .entries
            .iter()
            .filter_map(|(name, out)| {
                fuzzy_score(query, name).map(|s| (s, Suggestion { name: name.clone(), preview: preview(out), out: out.clone() }))
            })
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.len().cmp(&b.1.name.len())).then(a.1.name.cmp(&b.1.name)));
        v.into_iter().take(limit).map(|(_, s)| s).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    /// What to show as the rendered symbol in the popup.
    pub preview: String,
    pub out: Out,
}

fn preview(o: &Out) -> String {
    match o {
        Out::Sym(s) => s.clone(),
        Out::Func(f) => f.clone(),
        Out::Big(b) => match b {
            BigOp::Sum => "∑",
            BigOp::Prod => "∏",
            BigOp::Int => "∫",
            BigOp::IInt => "∬",
            BigOp::IIInt => "∭",
            BigOp::OInt => "∮",
            BigOp::OIInt => "∯",
            BigOp::Lim => "lim",
        }
        .into(),
        Out::Sqrt => "√□".into(),
        Out::Accent(a) => match a {
            Accent::Arrow => "v⃗",
            Accent::Hat => "v̂",
            Accent::Bold => "𝐯",
            Accent::Bar => "v̄",
            Accent::Tilde => "ṽ",
            Accent::Dot => "v̇",
            Accent::DDot => "v̈",
        }
        .into(),
        Out::Fence(f) => match f {
            Fence::Abs => "|□|",
            Fence::Norm => "‖□‖",
            Fence::Floor => "⌊□⌋",
            Fence::Ceil => "⌈□⌉",
            Fence::Angle => "⟨□⟩",
            _ => "(□)",
        }
        .into(),
        Out::Frac => "□⁄□".into(),
        Out::Matrix { rows, cols, .. } => format!("[{rows}×{cols}]"),
        Out::Cases => "{ cases".into(),
        Out::Deriv { partial: false } => "d/dx".into(),
        Out::Deriv { partial: true } => "∂/∂x".into(),
        Out::EvalBar => "□|ₐᵇ".into(),
    }
}

/// Subsequence match score; higher is better; None = no match.
pub fn fuzzy_score(query: &str, name: &str) -> Option<i32> {
    let (q, n): (Vec<char>, Vec<char>) = (query.to_lowercase().chars().collect(), name.to_lowercase().chars().collect());
    if name.starts_with(query) {
        return Some(1000 - (n.len() - q.len()) as i32);
    }
    let mut qi = 0;
    let mut score = 0;
    let mut last: Option<usize> = None;
    for (i, c) in n.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            score += 10;
            if last == Some(i.wrapping_sub(1)) {
                score += 15; // consecutive
            }
            if i == 0 {
                score += 20;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score - n.len() as i32)
}

/// `mat3x4` / `bmat2x2` / `vmat3x3` etc.
pub fn parse_matrix_name(name: &str) -> Option<Out> {
    let (kind, rest) = if let Some(r) = name.strip_prefix("bmat") {
        (MatrixKind::Bracket, r)
    } else if let Some(r) = name.strip_prefix("vmat") {
        (MatrixKind::Det, r)
    } else {
        (MatrixKind::Paren, name.strip_prefix("mat")?)
    };
    let (r, c) = rest.split_once('x')?;
    let (rows, cols): (usize, usize) = (r.parse().ok()?, c.parse().ok()?);
    ((1..=6).contains(&rows) && (1..=6).contains(&cols)).then_some(Out::Matrix { kind, rows, cols })
}

fn matrix_prefix_ok(word: &str) -> bool {
    let Some(rest) = word.strip_prefix("bmat").or_else(|| word.strip_prefix("vmat")).or_else(|| word.strip_prefix("mat")) else {
        return false;
    };
    rest.chars().enumerate().all(|(i, c)| c.is_ascii_digit() || (c == 'x' && i > 0)) && rest.matches('x').count() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_matrix_names() {
        let t = Table::new(&UserShorthand::default());
        assert_eq!(t.get("alpha"), Some(Out::Sym("α".into())));
        assert_eq!(t.get("mat3x4"), Some(Out::Matrix { kind: MatrixKind::Paren, rows: 3, cols: 4 }));
        assert_eq!(t.get("bmat6x6"), Some(Out::Matrix { kind: MatrixKind::Bracket, rows: 6, cols: 6 }));
        assert_eq!(t.get("mat7x7"), None);
    }

    #[test]
    fn prefix_ambiguity() {
        let t = Table::new(&UserShorthand::default());
        assert!(t.is_strict_prefix("in"), "in -> int, inf, inter");
        assert!(!t.is_strict_prefix("alpha"));
        assert!(t.is_strict_prefix("mat3"));
    }

    #[test]
    fn user_overrides_and_removal() {
        let mut u = UserShorthand::default();
        u.map.insert("ee".into(), "ε".into());
        u.map.insert("in".into(), String::new()); // disable
        let t = Table::new(&u);
        assert_eq!(t.get("ee"), Some(Out::Sym("ε".into())));
        assert_eq!(t.get("in"), None);
    }

    #[test]
    fn fuzzy_suggestions_rank_prefix_first() {
        let t = Table::new(&UserShorthand::default());
        let s = t.suggest("alp", 5);
        assert_eq!(s[0].name, "alpha");
        assert_eq!(s[0].preview, "α");
        let s = t.suggest("sbst", 5);
        assert!(s.iter().any(|x| x.name == "subset"), "{s:?}");
        assert!(t.suggest("", 5).is_empty());
    }
}
