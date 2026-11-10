//! Spatial parse: recognized glyphs (label + bounding box) -> expression tree.
//!
//! Heuristics, applied outermost-first: fraction bars, radicals, big operators with
//! bounds above/below; then a left-to-right pass handling superscripts, subscripts and
//! primes relative to each base glyph's baseline.

use crate::expr::*;
use ink::Rect;

#[derive(Clone, Debug)]
pub struct Glyph {
    pub label: String,
    pub rect: Rect,
}

impl Glyph {
    pub fn new(label: &str, x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { label: label.into(), rect: Rect { x0: x, y0: y, x1: x + w, y1: y + h } }
    }
    fn cx(&self) -> f32 {
        (self.rect.x0 + self.rect.x1) / 2.0
    }
    fn cy(&self) -> f32 {
        (self.rect.y0 + self.rect.y1) / 2.0
    }
    fn w(&self) -> f32 {
        self.rect.width()
    }
    fn h(&self) -> f32 {
        self.rect.height()
    }
    fn is_bar(&self) -> bool {
        matches!(self.label.as_str(), "−" | "-" | "—") && self.w() > 2.5 * self.h().max(1.0)
    }
}

const FUNCS: &[&str] = &[
    "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "sin", "cos", "tan", "sec", "csc", "cot", "ln", "log",
    "exp", "det", "tr", "rank", "span", "dim", "ker", "im", "max", "min", "lim",
];

pub fn parse(glyphs: &[Glyph]) -> Expr {
    let mut gs: Vec<Glyph> = glyphs.to_vec();
    gs.sort_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0));
    parse_level(gs)
}

fn split_by<F: Fn(&Glyph) -> bool>(gs: Vec<Glyph>, f: F) -> (Vec<Glyph>, Vec<Glyph>) {
    gs.into_iter().partition(|g| f(g))
}

fn parse_level(gs: Vec<Glyph>) -> Expr {
    if gs.is_empty() {
        return Expr::empty();
    }
    if let Some(e) = try_fraction(&gs) {
        return e;
    }
    if let Some(e) = try_sqrt(&gs) {
        return e;
    }
    if let Some(e) = try_big_op(&gs) {
        return e;
    }
    parse_row(gs)
}

/// Rows of the form `left <structure> right`, where left/right are glyphs outside the structure.
fn assemble(left_right_pivot: f32, rest: Vec<Glyph>, mid: Expr) -> Expr {
    let (left, right) = split_by(rest, |g| g.cx() < left_right_pivot);
    let mut v = Vec::new();
    for part in [parse_level(left), mid, parse_level(right)] {
        match part {
            Expr::Row(items) => v.extend(items),
            other => v.push(other),
        }
    }
    Expr::row(v)
}

fn try_fraction(gs: &[Glyph]) -> Option<Expr> {
    let mut bars: Vec<usize> = (0..gs.len()).filter(|&i| gs[i].is_bar()).collect();
    bars.sort_by(|&a, &b| gs[b].w().total_cmp(&gs[a].w()));
    for bi in bars {
        let bar = &gs[bi];
        let slack = 0.1 * bar.w();
        let in_range = |g: &Glyph| g.cx() >= bar.rect.x0 - slack && g.cx() <= bar.rect.x1 + slack;
        let above: Vec<Glyph> = gs.iter().enumerate().filter(|(i, g)| *i != bi && in_range(g) && g.rect.y1 <= bar.cy() + 0.2 * g.h()).map(|(_, g)| g.clone()).collect();
        let below: Vec<Glyph> = gs.iter().enumerate().filter(|(i, g)| *i != bi && in_range(g) && g.rect.y0 >= bar.cy() - 0.2 * g.h()).map(|(_, g)| g.clone()).collect();
        if above.is_empty() || below.is_empty() {
            continue;
        }
        let pivot = bar.cx();
        let used = |g: &Glyph| above.iter().chain(below.iter()).any(|u| std::ptr::eq(u, g) || (u.rect.x0 == g.rect.x0 && u.rect.y0 == g.rect.y0 && u.label == g.label));
        let rest: Vec<Glyph> = gs.iter().enumerate().filter(|(i, g)| *i != bi && !used(g)).map(|(_, g)| g.clone()).collect();
        let frac = Expr::Frac(Box::new(parse_level(above.clone())), Box::new(parse_level(below.clone())));
        return Some(assemble(pivot, rest, frac));
    }
    None
}

fn try_sqrt(gs: &[Glyph]) -> Option<Expr> {
    let si = gs.iter().position(|g| g.label == "√")?;
    let s = &gs[si];
    let inside = |g: &Glyph| g.cx() > s.rect.x0 + 0.25 * s.w() && g.cx() < s.rect.x1 + 4.0 * s.w() && g.cy() >= s.rect.y0 - 0.2 * s.h() && g.cy() <= s.rect.y1;
    let index = |g: &Glyph| g.cx() < s.rect.x0 + 0.4 * s.w() && g.cy() < s.cy() && g.rect.x1 <= s.rect.x1;
    let mut body = Vec::new();
    let mut idx = Vec::new();
    let mut rest = Vec::new();
    for (i, g) in gs.iter().enumerate() {
        if i == si {
            continue;
        }
        if index(g) && g.h() < 0.7 * s.h() {
            idx.push(g.clone());
        } else if inside(g) {
            body.push(g.clone());
        } else {
            rest.push(g.clone());
        }
    }
    if body.is_empty() {
        return None;
    }
    let node = Expr::Sqrt {
        index: (!idx.is_empty()).then(|| Box::new(parse_level(idx))),
        body: Box::new(parse_level(body)),
    };
    Some(assemble(s.cx(), rest, node))
}

fn big_op(label: &str) -> Option<BigOp> {
    Some(match label {
        "∑" => BigOp::Sum,
        "∏" => BigOp::Prod,
        "∫" => BigOp::Int,
        "∬" => BigOp::IInt,
        "∭" => BigOp::IIInt,
        "∮" => BigOp::OInt,
        "∯" => BigOp::OIInt,
        _ => return None,
    })
}

fn try_big_op(gs: &[Glyph]) -> Option<Expr> {
    let oi = gs.iter().position(|g| big_op(&g.label).is_some())?;
    let o = &gs[oi];
    let op = big_op(&o.label)?;
    let is_int = matches!(op, BigOp::Int | BigOp::IInt | BigOp::IIInt | BigOp::OInt | BigOp::OIInt);
    let (mut upper, mut lower, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    for (i, g) in gs.iter().enumerate() {
        if i == oi {
            continue;
        }
        // Limits: ∑ ∏ have bounds centred above/below; ∫ has them to the right at the top/bottom.
        let (x_ok, up, down) = if is_int {
            (g.rect.x0 >= o.rect.x1 - 0.8 * o.w() && g.rect.x0 <= o.rect.x1 + 1.2 * o.w() + 0.5 * g.w(), g.cy() < o.rect.y0 + 0.3 * o.h(), g.cy() > o.rect.y1 - 0.3 * o.h())
        } else {
            (g.cx() >= o.rect.x0 - 0.3 * o.w() && g.cx() <= o.rect.x1 + 0.3 * o.w(), g.cy() < o.rect.y0, g.cy() > o.rect.y1)
        };
        if x_ok && up {
            upper.push(g.clone());
        } else if x_ok && down {
            lower.push(g.clone());
        } else {
            rest.push(g.clone());
        }
    }
    // Body: everything to the right of the operator; stuff to the left stays in the outer row.
    let (left, body) = split_by(rest, |g| g.cx() < o.cx());
    let big = Expr::Big {
        op,
        lower: (!lower.is_empty()).then(|| Box::new(parse_level(lower))),
        upper: (!upper.is_empty()).then(|| Box::new(parse_level(upper))),
        body: Box::new(parse_level(body)),
    };
    let mut v = Vec::new();
    for part in [parse_level(left), big] {
        match part {
            Expr::Row(items) => v.extend(items),
            other => v.push(other),
        }
    }
    Some(Expr::row(v))
}

#[derive(PartialEq)]
enum Pos {
    Sup,
    Sub,
    Main,
}

fn parse_row(gs: Vec<Glyph>) -> Expr {
    let mut items: Vec<Expr> = Vec::new();
    let mut plain: Vec<bool> = Vec::new(); // item is a bare symbol with no scripts
    let n = gs.len();
    let mut i = 0;
    while i < n {
        let base = &gs[i];
        let bh = base.h().max(1.0);
        let mut sups: Vec<Glyph> = Vec::new();
        let mut subs: Vec<Glyph> = Vec::new();
        let mut j = i + 1;
        let mut last = Pos::Main;
        while j < n {
            let g = &gs[j];
            // Must start near/after the base, not far away.
            if g.rect.x0 > base.rect.x1 + 1.5 * bh {
                break;
            }
            let (above, below) = (g.cy() < base.cy() - 0.25 * bh, g.cy() > base.cy() + 0.25 * bh);
            let smaller = g.h() < 0.85 * bh || g.is_bar();
            let pos = if above && (smaller || g.h() < bh) {
                Pos::Sup
            } else if below && smaller {
                Pos::Sub
            } else if last == Pos::Sup && g.cy() < base.cy() - 0.1 * bh && g.h() < bh {
                Pos::Sup // continuation of a superscript (x^{2n})
            } else if last == Pos::Sub && g.cy() > base.cy() + 0.1 * bh && g.h() < bh {
                Pos::Sub
            } else {
                Pos::Main
            };
            match pos {
                Pos::Sup => sups.push(g.clone()),
                Pos::Sub => subs.push(g.clone()),
                Pos::Main => break,
            }
            last = pos;
            j += 1;
        }
        let atom = atom(&base.label);
        let scripted = !sups.is_empty() || !subs.is_empty();
        let node = match (sups.is_empty(), subs.is_empty()) {
            (true, true) => atom,
            (false, true) => Expr::Sup(Box::new(atom), Box::new(parse_level(sups))),
            (true, false) => Expr::Sub(Box::new(atom), Box::new(parse_level(subs))),
            (false, false) => Expr::SubSup(Box::new(atom), Box::new(parse_level(subs)), Box::new(parse_level(sups))),
        };
        items.push(node);
        plain.push(!scripted);
        i = j;
    }
    Expr::row(merge_runs(items, plain))
}

fn atom(label: &str) -> Expr {
    if label.chars().all(|c| c.is_ascii_digit()) {
        Expr::num(label)
    } else {
        Expr::sym(label)
    }
}

/// Merge adjacent digits into numbers and letter runs that spell a function name into `Func`.
fn merge_runs(items: Vec<Expr>, plain: Vec<bool>) -> Vec<Expr> {
    let mut out: Vec<Expr> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        if plain[i] {
            // function names (longest match first)
            let mut matched = false;
            for f in FUNCS {
                let len = f.len();
                if i + len <= items.len()
                    && (i..i + len).all(|k| plain[k])
                    && (0..len).all(|k| matches!(&items[i + k], Expr::Sym(s) if s.len() == 1 && s.as_bytes()[0] as char == f.as_bytes()[k] as char))
                {
                    out.push(Expr::Func(f.to_string()));
                    i += len;
                    matched = true;
                    break;
                }
            }
            if matched {
                continue;
            }
            // numbers
            if let Expr::Num(d) = &items[i] {
                let mut s = d.clone();
                let mut k = i + 1;
                while k < items.len() && plain[k] {
                    if let Expr::Num(d2) = &items[k] {
                        s += d2;
                        k += 1;
                    } else {
                        break;
                    }
                }
                out.push(Expr::Num(s));
                i = k;
                continue;
            }
        }
        out.push(items[i].clone());
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(l: &str, x: f32, y: f32, w: f32, h: f32) -> Glyph {
        Glyph::new(l, x, y, w, h)
    }

    #[test]
    fn superscript_and_subscript() {
        // x^2
        let e = parse(&[g("x", 0.0, 10.0, 20.0, 20.0), g("2", 22.0, 0.0, 10.0, 14.0)]);
        assert_eq!(e.to_typst(), "x^2");
        // a_1
        let e = parse(&[g("a", 0.0, 10.0, 20.0, 20.0), g("1", 22.0, 24.0, 8.0, 14.0)]);
        assert_eq!(e.to_typst(), "a_1");
    }

    #[test]
    fn exponent_with_multiple_glyphs_and_following_term() {
        // x^{2n} + 1
        let e = parse(&[
            g("x", 0.0, 10.0, 20.0, 20.0),
            g("2", 22.0, 0.0, 9.0, 13.0),
            g("n", 33.0, 2.0, 10.0, 11.0),
            g("+", 55.0, 12.0, 18.0, 18.0),
            g("1", 80.0, 8.0, 10.0, 22.0),
        ]);
        assert_eq!(e.to_typst(), "x^(2 n) + 1");
    }

    #[test]
    fn minus_is_not_a_script() {
        let e = parse(&[g("x", 0.0, 10.0, 20.0, 20.0), g("−", 26.0, 18.0, 16.0, 3.0), g("1", 50.0, 8.0, 10.0, 22.0)]);
        assert_eq!(e.to_typst(), "x − 1");
    }

    #[test]
    fn fraction() {
        let e = parse(&[g("a", 10.0, 0.0, 14.0, 20.0), g("−", 0.0, 28.0, 50.0, 3.0), g("b", 10.0, 36.0, 14.0, 20.0), g("c", 10.0, 36.0, 14.0, 20.0)]);
        assert_eq!(e.to_latex(), "\\frac{a}{b c}");
    }

    #[test]
    fn fraction_with_neighbours() {
        // y = a/b
        let e = parse(&[
            g("y", 0.0, 22.0, 18.0, 22.0),
            g("=", 24.0, 28.0, 18.0, 12.0),
            g("a", 60.0, 0.0, 14.0, 20.0),
            g("−", 50.0, 28.0, 40.0, 3.0),
            g("b", 62.0, 36.0, 14.0, 20.0),
        ]);
        assert_eq!(e.to_typst(), "y = frac(a, b)");
    }

    #[test]
    fn sqrt_with_index() {
        let e = parse(&[g("√", 10.0, 0.0, 20.0, 30.0), g("x", 32.0, 8.0, 16.0, 20.0), g("3", 0.0, 0.0, 8.0, 12.0)]);
        assert_eq!(e.to_latex(), "\\sqrt[3]{x}");
    }

    #[test]
    fn summation_with_bounds() {
        // sum_{i=0}^{n} a_i  -> bounds centred above/below
        let e = parse(&[
            g("∑", 0.0, 20.0, 30.0, 40.0),
            g("n", 8.0, 0.0, 14.0, 14.0),
            g("i", 4.0, 66.0, 6.0, 14.0),
            g("=", 11.0, 68.0, 10.0, 8.0),
            g("0", 22.0, 66.0, 8.0, 14.0),
            g("a", 40.0, 30.0, 18.0, 20.0),
        ]);
        assert_eq!(e.to_latex(), "\\sum_{i = 0}^{n} a");
    }

    #[test]
    fn integral_with_bounds_and_func_names() {
        let e = parse(&[
            g("∫", 0.0, 0.0, 14.0, 50.0),
            g("1", 16.0, 0.0, 8.0, 12.0),
            g("0", 16.0, 38.0, 8.0, 12.0),
            g("x", 40.0, 15.0, 16.0, 20.0),
        ]);
        assert_eq!(e.to_typst(), "integral_0^1 x");
        let e = parse(&[g("s", 0.0, 10.0, 12.0, 14.0), g("i", 14.0, 6.0, 6.0, 18.0), g("n", 22.0, 10.0, 14.0, 14.0), g("x", 40.0, 10.0, 14.0, 14.0)]);
        assert_eq!(e.to_typst(), "sin x");
        let e = parse(&[g("1", 0.0, 0.0, 8.0, 20.0), g("2", 10.0, 0.0, 12.0, 20.0)]);
        assert_eq!(e, Expr::num("12"));
    }
}
