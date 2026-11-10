//! Best-effort LaTeX math -> Typst math, for pasting formulas from elsewhere. Covers fractions,
//! roots, scripts, matrices, fonts, accents, Greek, operators, relations and arrows. Anything
//! unknown is passed through by name (Typst shares most symbol names with LaTeX).

fn sym(name: &str) -> Option<&'static str> {
    Some(match name {
        "cdot" => "dot",
        "times" => "times",
        "div" => "div",
        "pm" => "plus.minus",
        "mp" => "minus.plus",
        "infty" => "infinity",
        "int" => "integral",
        "iint" => "integral.double",
        "iiint" => "integral.triple",
        "oint" => "integral.cont",
        "sum" => "sum",
        "prod" => "product",
        "partial" => "partial",
        "nabla" => "nabla",
        "le" | "leq" => "<=",
        "ge" | "geq" => ">=",
        "ne" | "neq" => "!=",
        "ll" => "<<",
        "gg" => ">>",
        "approx" => "approx",
        "equiv" => "equiv",
        "sim" => "tilde.op",
        "cong" => "tilde.equiv",
        "propto" => "prop",
        "to" | "rightarrow" => "->",
        "leftarrow" => "<-",
        "leftrightarrow" => "<->",
        "Rightarrow" | "implies" => "=>",
        "Leftrightarrow" | "iff" => "<=>",
        "mapsto" => "|->",
        "in" => "in",
        "notin" => "in.not",
        "subset" => "subset",
        "subseteq" => "subset.eq",
        "supset" => "supset",
        "supseteq" => "supset.eq",
        "cup" => "union",
        "cap" => "sect",
        "setminus" => "without",
        "emptyset" | "varnothing" => "emptyset",
        "forall" => "forall",
        "exists" => "exists",
        "nexists" => "exists.not",
        "neg" | "lnot" => "not",
        "land" | "wedge" => "and",
        "lor" | "vee" => "or",
        "therefore" => "therefore",
        "because" => "because",
        "ldots" | "dots" => "...",
        "cdots" => "dots.c",
        "vdots" => "dots.v",
        "ddots" => "dots.down",
        "langle" => "angle.l",
        "rangle" => "angle.r",
        "lfloor" => "floor.l",
        "rfloor" => "floor.r",
        "lceil" => "ceil.l",
        "rceil" => "ceil.r",
        "perp" => "perp",
        "parallel" => "parallel",
        "angle" => "angle",
        "circ" => "compose",
        "degree" => "degree",
        "hbar" => "planck.reduce",
        "ell" => "ell",
        "prime" => "prime",
        "otimes" => "times.circle",
        "oplus" => "plus.circle",
        "ast" => "ast",
        "star" => "star",
        "varepsilon" => "epsilon",
        "epsilon" => "epsilon.alt",
        "vartheta" => "theta.alt",
        "varphi" => "phi",
        "phi" => "phi.alt",
        "varrho" => "rho.alt",
        "varpi" => "pi.alt",
        "varsigma" => "sigma.alt",
        "varkappa" => "kappa.alt",
        "quad" | "qquad" | "," | ";" | ":" | "!" | " " => " ",
        "{" => "{",
        "}" => "}",
        "|" => "||",
        "%" => "%",
        "$" => "\\$",
        "#" => "\\#",
        "&" => "&",
        "_" => "\\_",
        _ => return None,
    })
}

const FUNCS: &[&str] = &["sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "log", "ln", "exp", "det", "dim", "ker", "max", "min", "sup", "inf", "lim", "gcd", "arg", "deg", "Pr", "mod"];

struct P<'a> {
    s: &'a [char],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }
    fn command(&mut self) -> String {
        // after a backslash: letters, or a single non-letter
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i == start {
            if let Some(c) = self.peek() {
                self.i += 1;
                return c.to_string();
            }
        }
        self.s[start..self.i].iter().collect()
    }
    /// One argument: `{...}` group or a single token.
    fn arg(&mut self) -> String {
        self.skip_ws();
        match self.peek() {
            Some('{') => {
                self.i += 1;
                let inner = self.seq(Some('}'));
                self.i += 1;
                inner
            }
            Some('\\') => {
                self.i += 1;
                let c = self.command();
                self.cmd(&c)
            }
            Some(c) => {
                self.i += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }
    fn raw_group(&mut self) -> String {
        self.skip_ws();
        if self.peek() != Some('{') {
            return String::new();
        }
        self.i += 1;
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            self.i += 1;
        }
        let out: String = self.s[start..self.i].iter().collect();
        self.i += 1;
        out
    }
    fn cmd(&mut self, c: &str) -> String {
        match c {
            "frac" | "dfrac" | "tfrac" => {
                let a = self.arg();
                let b = self.arg();
                format!("frac({a}, {b})")
            }
            "sqrt" => {
                self.skip_ws();
                if self.peek() == Some('[') {
                    self.i += 1;
                    let n = self.seq(Some(']'));
                    self.i += 1;
                    let b = self.arg();
                    format!("root({n}, {b})")
                } else {
                    format!("sqrt({})", self.arg())
                }
            }
            "mathbb" => format!("bb({})", self.arg()),
            "mathbf" | "boldsymbol" => format!("bold({})", self.arg()),
            "mathrm" | "operatorname" => format!("upright({})", self.arg()),
            "mathcal" => format!("cal({})", self.arg()),
            "mathit" => format!("italic({})", self.arg()),
            "text" | "mbox" | "textrm" => format!("\"{}\"", self.raw_group().replace('"', "\\\"")),
            "vec" => format!("arrow({})", self.arg()),
            "hat" | "widehat" => format!("hat({})", self.arg()),
            "bar" | "overline" => format!("overline({})", self.arg()),
            "underline" => format!("underline({})", self.arg()),
            "tilde" | "widetilde" => format!("tilde({})", self.arg()),
            "dot" => format!("dot({})", self.arg()),
            "ddot" => format!("dot.double({})", self.arg()),
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle" | "limits" | "nolimits" => String::new(),
            "begin" => {
                let env = self.raw_group();
                let body = self.seq_until_end();
                let rows: Vec<String> = body.split("\\\\").map(|r| r.split('&').map(|c| c.trim().to_string()).collect::<Vec<_>>().join(", ")).filter(|r| !r.trim().is_empty()).collect();
                match env.as_str() {
                    "pmatrix" => format!("mat({})", rows.join("; ")),
                    "bmatrix" => format!("mat(delim: \"[\", {})", rows.join("; ")),
                    "vmatrix" => format!("mat(delim: \"|\", {})", rows.join("; ")),
                    "Vmatrix" => format!("mat(delim: \"||\", {})", rows.join("; ")),
                    "matrix" => format!("mat(delim: #none, {})", rows.join("; ")),
                    "cases" => format!("cases({})", body.split("\\\\").map(|r| r.replace('&', "&").trim().to_string()).filter(|r| !r.is_empty()).collect::<Vec<_>>().join(", ")),
                    _ => body,
                }
            }
            "end" => {
                self.raw_group();
                String::new()
            }
            "\\" => "\\\\".into(),
            f if FUNCS.contains(&f) => format!(" {f} "),
            other => match sym(other) {
                Some(s) => format!(" {s} "),
                None => format!(" {other} "),
            },
        }
    }
    /// Everything up to the matching `\end{...}`, converted, with `\\` row breaks kept as "\\".
    fn seq_until_end(&mut self) -> String {
        let mut out = String::new();
        while self.i < self.s.len() {
            if self.s[self.i..].starts_with(&['\\', 'e', 'n', 'd']) {
                self.i += 4;
                self.raw_group();
                break;
            }
            if self.s[self.i..].starts_with(&['\\', '\\']) {
                self.i += 2;
                out += "\\\\";
                continue;
            }
            if self.peek() == Some('&') {
                self.i += 1;
                out.push('&');
                continue;
            }
            out += &self.token();
        }
        out
    }
    fn token(&mut self) -> String {
        let Some(c) = self.peek() else { return String::new() };
        self.i += 1;
        match c {
            '\\' => {
                let name = self.command();
                self.cmd(&name)
            }
            '^' | '_' => {
                let a = self.arg();
                if a.chars().count() == 1 {
                    format!("{c}{a}")
                } else {
                    format!("{c}({a})")
                }
            }
            '{' => {
                let inner = self.seq(Some('}'));
                self.i += 1;
                format!("({inner})")
            }
            '~' => " ".into(),
            '"' => "\\\"".into(),
            c if c.is_ascii_alphabetic() => {
                // separate single-letter variables: LaTeX `xy` is x times y, Typst `xy` is a name
                format!("{c} ")
            }
            c => c.to_string(),
        }
    }
    fn seq(&mut self, end: Option<char>) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if Some(c) == end {
                break;
            }
            out += &self.token();
        }
        out
    }
}

pub fn latex_to_typst(src: &str) -> String {
    let s = src.trim().trim_start_matches("$$").trim_end_matches("$$").trim_matches('$').trim_start_matches("\\[").trim_end_matches("\\]");
    let chars: Vec<char> = s.chars().collect();
    let mut p = P { s: &chars, i: 0 };
    let out = p.seq(None);
    out.split_whitespace().collect::<Vec<_>>().join(" ").replace(" ^", "^").replace(" _", "_").replace("( ", "(").replace(" )", ")").replace(" ,", ",")
}

/// Does this pasted text look like LaTeX (rather than our shorthand)?
pub fn looks_like_latex(s: &str) -> bool {
    s.contains('\\') || (s.contains('{') && (s.contains('^') || s.contains('_')))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiles(t: &str) {
        // the output must be valid Typst math (checked with the real compiler in render's tests;
        // here we check the shapes)
        assert!(!t.contains('\\') || t.contains("\\\\") || t.contains("\\$") || t.contains("\\#") || t.contains("\\\""), "{t}");
    }

    fn n(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn common_formulas() {
        assert_eq!(n(&latex_to_typst(r"\frac{a}{b}")), n("frac(a, b)"));
        assert_eq!(n(&latex_to_typst(r"x^{2}+y^2")), n("x^2 + y^2"));
        assert_eq!(n(&latex_to_typst(r"\sqrt[3]{x}")), n("root(3, x)"));
        assert_eq!(n(&latex_to_typst(r"\int_0^\infty e^{-x}\,dx")), n("integral_0^(infinity) e^(-x) d x"));
        assert_eq!(n(&latex_to_typst(r"\alpha \leq \beta")), n("alpha <= beta"));
        assert_eq!(n(&latex_to_typst(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}")), n("mat(a, b; c, d)"));
        assert_eq!(n(&latex_to_typst(r"\mathbb{R}^n")), n("bb(R)^n"));
        assert_eq!(n(&latex_to_typst(r"\vec{v}\cdot\hat{n}")), n("arrow(v) dot hat(n)"));
        assert_eq!(n(&latex_to_typst(r"\sin(x)")), n("sin(x)"));
        assert_eq!(n(&latex_to_typst(r"$\lim_{x \to 0} \frac{\sin x}{x}$")), n("lim_(x -> 0) frac(sin x, x)"));
        for t in [r"\frac{\partial f}{\partial x}", r"\left( \sum_{i=1}^n i \right)", r"\text{if } x > 0"] {
            compiles(&latex_to_typst(t));
        }
        assert!(looks_like_latex(r"\alpha") && !looks_like_latex("alpha + 1"));
    }
}
