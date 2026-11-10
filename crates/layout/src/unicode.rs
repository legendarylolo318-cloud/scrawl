//! Plain Unicode rendering of an expression, for "copy as text".

use crate::expr::*;

fn sup_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹',
        '+' => '⁺', '-' | '−' => '⁻', 'n' => 'ⁿ', 'i' => 'ⁱ', 'x' => 'ˣ', 'T' => 'ᵀ', '=' => '⁼', '(' => '⁽', ')' => '⁾',
        _ => return None,
    })
}

fn sub_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉',
        '+' => '₊', '-' | '−' => '₋', '=' => '₌', '(' => '₍', ')' => '₎', 'i' => 'ᵢ', 'n' => 'ₙ', 'x' => 'ₓ',
        _ => return None,
    })
}

fn script(s: &str, up: bool, marker: char) -> String {
    let conv: Option<String> = s.chars().map(|c| if up { sup_char(c) } else { sub_char(c) }).collect();
    conv.unwrap_or_else(|| format!("{marker}({s})"))
}

fn wrap(s: String) -> String {
    if s.chars().count() <= 1 { s } else { format!("({s})") }
}

pub fn ser(e: &Expr) -> String {
    match e {
        Expr::Row(v) => v.iter().map(ser).collect::<Vec<_>>().join(" "),
        Expr::Sym(s) => s.clone(),
        Expr::Num(s) | Expr::Func(s) | Expr::Unit(s) | Expr::Text(s) | Expr::Raw(s) => s.clone(),
        Expr::Frac(a, b) => format!("{}/{}", wrap(ser(a)), wrap(ser(b))),
        Expr::Sqrt { index: None, body } => format!("√({})", ser(body)),
        Expr::Sqrt { index: Some(i), body } => format!("{}√({})", script(&ser(i), true, '^'), ser(body)),
        Expr::Sup(b, x) => format!("{}{}", ser(b), script(&ser(x), true, '^')),
        Expr::Sub(b, x) => format!("{}{}", ser(b), script(&ser(x), false, '_')),
        Expr::SubSup(b, lo, hi) => format!("{}{}{}", ser(b), script(&ser(lo), false, '_'), script(&ser(hi), true, '^')),
        Expr::Big { op, lower, upper, body } => {
            let sym = match op {
                BigOp::Sum => "∑", BigOp::Prod => "∏", BigOp::Int => "∫", BigOp::IInt => "∬", BigOp::IIInt => "∭",
                BigOp::OInt => "∮", BigOp::OIInt => "∯", BigOp::Lim => "lim",
            };
            let mut s = sym.to_string();
            if let Some(l) = lower { s += &script(&ser(l), false, '_'); }
            if let Some(u) = upper { s += &script(&ser(u), true, '^'); }
            if !body.is_empty() { s += &format!(" {}", ser(body)); }
            s
        }
        Expr::Matrix { kind, rows, .. } => {
            let (l, r) = match kind { MatrixKind::Paren => ("(", ")"), MatrixKind::Bracket => ("[", "]"), MatrixKind::Det => ("|", "|"), MatrixKind::Norm => ("‖", "‖"), MatrixKind::Brace => ("{", "}") };
            let body: Vec<String> = rows.iter().map(|r| r.iter().map(ser).collect::<Vec<_>>().join(" ")).collect();
            format!("{l}{}{r}", body.join("; "))
        }
        Expr::Accent(a, b) => {
            let mark = match a { Accent::Arrow => "\u{20d7}", Accent::Hat => "\u{302}", Accent::Bold => "", Accent::Bar => "\u{304}", Accent::Tilde => "\u{303}", Accent::Dot => "\u{307}", Accent::DDot => "\u{308}" };
            let s = ser(b);
            let mut out = String::new();
            for (i, c) in s.chars().enumerate() { out.push(c); if i == 0 { out += mark; } }
            out
        }
        Expr::Fence(f, b) => {
            let (l, r) = match f { Fence::Paren => ("(", ")"), Fence::Bracket => ("[", "]"), Fence::Brace => ("{", "}"), Fence::Abs => ("|", "|"), Fence::Norm => ("‖", "‖"), Fence::Angle => ("⟨", "⟩"), Fence::Floor => ("⌊", "⌋"), Fence::Ceil => ("⌈", "⌉") };
            format!("{l}{}{r}", ser(b))
        }
        Expr::Cases(rows) => format!("{{ {} }}", rows.iter().map(|(v, c)| format!("{} if {}", ser(v), ser(c))).collect::<Vec<_>>().join("; ")),
        Expr::Deriv { partial, var, body, .. } => {
            let d = if *partial { "∂" } else { "d" };
            format!("{d}/{d}{} {}", ser(var), ser(body))
        }
        Expr::EvalBar { body, lower, upper } => {
            let mut s = format!("{}|", ser(body));
            if let Some(l) = lower { s += &script(&ser(l), false, '_'); }
            if let Some(u) = upper { s += &script(&ser(u), true, '^'); }
            s
        }
        Expr::Align(lines) => lines.iter().map(|l| format!("{} {} {}", ser(&l.lhs), l.rel, ser(&l.rhs))).collect::<Vec<_>>().join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_output() {
        let e = Expr::Sup(Box::new(Expr::sym("x")), Box::new(Expr::num("2")));
        assert_eq!(ser(&e), "x²");
        let e = Expr::Frac(Box::new(Expr::sym("a")), Box::new(Expr::Row(vec![Expr::sym("b"), Expr::sym("+"), Expr::num("1")])));
        assert_eq!(ser(&e), "a/(b + 1)");
        let e = Expr::Sqrt { index: None, body: Box::new(Expr::sym("x")) };
        assert_eq!(ser(&e), "√(x)");
        let e = Expr::Sup(Box::new(Expr::sym("x")), Box::new(Expr::sym("y")));
        assert_eq!(ser(&e), "x^(y)");
    }
}
