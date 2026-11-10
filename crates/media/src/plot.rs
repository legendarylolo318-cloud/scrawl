//! Function plotter: parse f(x), sample, and render a 2D plot to SVG with labelled axes.
//! Supports multiple functions, parametric (x(t), y(t)) and polar (r(theta)) plots.

use std::f64::consts::{E, PI};

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let st = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(cs[st..i].iter().collect::<String>().parse().map_err(|_| "bad number")?));
        } else if c.is_alphabetic() || c == 'π' {
            let st = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == 'π') {
                i += 1;
            }
            out.push(Tok::Id(cs[st..i].iter().collect()));
        } else {
            out.push(match c {
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                ',' => Tok::Comma,
                '+' | '-' | '*' | '/' | '^' | '−' | '·' | '×' => Tok::Op(match c {
                    '−' => '-',
                    '·' | '×' => '*',
                    o => o,
                }),
                _ => return Err(format!("unexpected '{c}'")),
            });
            i += 1;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub enum Ast {
    Num(f64),
    Var(String),
    Neg(Box<Ast>),
    Bin(char, Box<Ast>, Box<Ast>),
    Call(String, Vec<Ast>),
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn expr(&mut self) -> Result<Ast, String> {
        let mut l = self.term()?;
        while let Some(Tok::Op(o @ ('+' | '-'))) = self.peek().cloned() {
            self.i += 1;
            l = Ast::Bin(o, Box::new(l), Box::new(self.term()?));
        }
        Ok(l)
    }
    fn term(&mut self) -> Result<Ast, String> {
        let mut l = self.unary()?;
        loop {
            match self.peek().cloned() {
                Some(Tok::Op(o @ ('*' | '/'))) => {
                    self.i += 1;
                    l = Ast::Bin(o, Box::new(l), Box::new(self.unary()?));
                }
                // implicit multiplication: 2x, 3(x+1), x sin(x)
                Some(Tok::Num(_) | Tok::Id(_) | Tok::LParen) => l = Ast::Bin('*', Box::new(l), Box::new(self.unary()?)),
                _ => break,
            }
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Ast, String> {
        if let Some(Tok::Op('-')) = self.peek() {
            self.i += 1;
            return Ok(Ast::Neg(Box::new(self.unary()?)));
        }
        if let Some(Tok::Op('+')) = self.peek() {
            self.i += 1;
        }
        self.power()
    }
    fn power(&mut self) -> Result<Ast, String> {
        let b = self.atom()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.i += 1;
            let e = self.unary()?; // right associative
            return Ok(Ast::Bin('^', Box::new(b), Box::new(e)));
        }
        Ok(b)
    }
    fn atom(&mut self) -> Result<Ast, String> {
        match self.t.get(self.i).cloned() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                Ok(Ast::Num(n))
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.expr()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("missing )".into());
                }
                self.i += 1;
                Ok(e)
            }
            Some(Tok::Id(name)) => {
                self.i += 1;
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let mut args = vec![self.expr()?];
                    while self.peek() == Some(&Tok::Comma) {
                        self.i += 1;
                        args.push(self.expr()?);
                    }
                    if self.peek() != Some(&Tok::RParen) {
                        return Err("missing )".into());
                    }
                    self.i += 1;
                    Ok(Ast::Call(name, args))
                } else {
                    Ok(Ast::Var(name))
                }
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

pub fn parse(src: &str) -> Result<Ast, String> {
    let mut p = P { t: lex(src)?, i: 0 };
    if p.t.is_empty() {
        return Err("empty expression".into());
    }
    let a = p.expr()?;
    if p.i != p.t.len() {
        return Err("trailing input".into());
    }
    Ok(a)
}

/// Evaluate with variable bindings (e.g. [("x", 1.0)]). Unknown identifiers are errors at eval time (NaN).
pub fn eval(a: &Ast, vars: &[(&str, f64)]) -> f64 {
    match a {
        Ast::Num(n) => *n,
        Ast::Var(v) => match v.as_str() {
            "pi" | "π" => PI,
            "e" => E,
            _ => vars.iter().find(|(n, _)| n == v).map_or(f64::NAN, |(_, x)| *x),
        },
        Ast::Neg(x) => -eval(x, vars),
        Ast::Bin(o, l, r) => {
            let (l, r) = (eval(l, vars), eval(r, vars));
            match o {
                '+' => l + r,
                '-' => l - r,
                '*' => l * r,
                '/' => l / r,
                _ => l.powf(r),
            }
        }
        Ast::Call(f, args) => {
            let x = eval(&args[0], vars);
            match (f.as_str(), args.len()) {
                ("sin", 1) => x.sin(),
                ("cos", 1) => x.cos(),
                ("tan", 1) => x.tan(),
                ("asin" | "arcsin", 1) => x.asin(),
                ("acos" | "arccos", 1) => x.acos(),
                ("atan" | "arctan", 1) => x.atan(),
                ("sinh", 1) => x.sinh(),
                ("cosh", 1) => x.cosh(),
                ("tanh", 1) => x.tanh(),
                ("exp", 1) => x.exp(),
                ("ln", 1) => x.ln(),
                ("log", 1) => x.log10(),
                ("sqrt", 1) => x.sqrt(),
                ("abs", 1) => x.abs(),
                ("floor", 1) => x.floor(),
                ("ceil", 1) => x.ceil(),
                ("min", 2) => x.min(eval(&args[1], vars)),
                ("max", 2) => x.max(eval(&args[1], vars)),
                ("pow", 2) => x.powf(eval(&args[1], vars)),
                _ => f64::NAN,
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Curve {
    /// y = f(x)
    Cartesian(String),
    /// (x(t), y(t)), t over `range`
    Parametric { x: String, y: String, t: (f64, f64) },
    /// r = f(theta), theta over `range`
    Polar { r: String, theta: (f64, f64) },
}

#[derive(Clone, Debug)]
pub struct Plot {
    pub curves: Vec<Curve>,
    pub x_range: (f64, f64),
    /// None = fit to the data.
    pub y_range: Option<(f64, f64)>,
    pub x_label: String,
    pub y_label: String,
}

const COLORS: [&str; 6] = ["#7aa2f7", "#f7768e", "#9ece6a", "#e0af68", "#bb9af7", "#73daca"];

/// Sample one curve into polylines (broken at NaN / infinities / big jumps).
pub fn sample(curve: &Curve, x_range: (f64, f64), n: usize) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let mut pts: Vec<Option<(f64, f64)>> = Vec::with_capacity(n);
    match curve {
        Curve::Cartesian(f) => {
            let a = parse(f)?;
            for i in 0..=n {
                let x = x_range.0 + (x_range.1 - x_range.0) * i as f64 / n as f64;
                let y = eval(&a, &[("x", x)]);
                pts.push(y.is_finite().then_some((x, y)));
            }
        }
        Curve::Parametric { x, y, t } => {
            let (ax, ay) = (parse(x)?, parse(y)?);
            for i in 0..=n {
                let tt = t.0 + (t.1 - t.0) * i as f64 / n as f64;
                let (px, py) = (eval(&ax, &[("t", tt)]), eval(&ay, &[("t", tt)]));
                pts.push((px.is_finite() && py.is_finite()).then_some((px, py)));
            }
        }
        Curve::Polar { r, theta } => {
            let a = parse(r)?;
            for i in 0..=n {
                let th = theta.0 + (theta.1 - theta.0) * i as f64 / n as f64;
                let rr = eval(&a, &[("theta", th), ("t", th), ("θ", th)]);
                pts.push(rr.is_finite().then(|| (rr * th.cos(), rr * th.sin())));
            }
        }
    }
    let mut lines = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    for p in pts {
        match p {
            Some(p) => {
                // break on near-vertical jumps (asymptotes such as tan x)
                if let Some(l) = cur.last() {
                    if (p.1 - l.1).abs() > 1e4 {
                        lines.push(std::mem::take(&mut cur));
                    }
                }
                cur.push(p);
            }
            None => lines.push(std::mem::take(&mut cur)),
        }
    }
    lines.push(cur);
    Ok(lines.into_iter().filter(|l| l.len() >= 2).collect())
}

fn nice_step(range: f64) -> f64 {
    let raw = range / 6.0;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    mag * if n < 1.5 { 1.0 } else if n < 3.5 { 2.0 } else if n < 7.5 { 5.0 } else { 10.0 }
}

fn fmt_tick(v: f64) -> String {
    if v.abs() < 1e-9 {
        "0".into()
    } else if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Render to a self-contained SVG with grid, axes, tick labels, axis labels and a legend.
pub fn render_svg(plot: &Plot, width: f64, height: f64, dark: bool) -> Result<String, String> {
    let (bg, fg, grid) = if dark { ("#1a1b26", "#c0caf5", "#3b4261") } else { ("#ffffff", "#1a1b26", "#d0d5e2") };
    let mut all = Vec::new();
    for c in &plot.curves {
        all.push(sample(c, plot.x_range, 600)?);
    }
    let (mut xmin, mut xmax) = plot.x_range;
    let polarish = plot.curves.iter().any(|c| !matches!(c, Curve::Cartesian(_)));
    let mut ys: Vec<f64> = all.iter().flatten().flatten().map(|p| p.1).collect();
    if polarish {
        let xs: Vec<f64> = all.iter().flatten().flatten().map(|p| p.0).collect();
        if let (Some(a), Some(b)) = (xs.iter().cloned().reduce(f64::min), xs.iter().cloned().reduce(f64::max)) {
            xmin = a - 0.1 * (b - a).max(1.0);
            xmax = b + 0.1 * (b - a).max(1.0);
        }
    }
    ys.sort_by(|a, b| a.total_cmp(b));
    let (ymin, ymax) = match plot.y_range {
        Some(r) => r,
        None if ys.is_empty() => (-1.0, 1.0),
        None => {
            // trim outliers (asymptotes) with 2%/98% percentiles
            let lo = ys[ys.len() * 2 / 100];
            let hi = ys[(ys.len() * 98 / 100).min(ys.len() - 1)];
            let pad = ((hi - lo) * 0.08).max(1e-6);
            (lo - pad, hi + pad)
        }
    };
    let (ymin, ymax) = if (ymax - ymin).abs() < 1e-9 { (ymin - 1.0, ymax + 1.0) } else { (ymin, ymax) };
    let (ml, mr, mt, mb) = (52.0, 16.0, 16.0, 40.0);
    let (pw, ph) = (width - ml - mr, height - mt - mb);
    let sx = |x: f64| ml + (x - xmin) / (xmax - xmin) * pw;
    let sy = |y: f64| mt + (1.0 - (y - ymin) / (ymax - ymin)) * ph;
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" font-family=\"sans-serif\" font-size=\"11\"><rect width=\"100%\" height=\"100%\" fill=\"{bg}\"/>");
    let (xs, ysx) = (nice_step(xmax - xmin), nice_step(ymax - ymin));
    let mut x = (xmin / xs).ceil() * xs;
    while x <= xmax + 1e-9 {
        s += &format!("<line x1=\"{0:.1}\" y1=\"{1}\" x2=\"{0:.1}\" y2=\"{2}\" stroke=\"{grid}\"/><text x=\"{0:.1}\" y=\"{3}\" fill=\"{fg}\" text-anchor=\"middle\">{4}</text>", sx(x), mt, mt + ph, mt + ph + 14.0, fmt_tick(x));
        x += xs;
    }
    let mut y = (ymin / ysx).ceil() * ysx;
    while y <= ymax + 1e-9 {
        s += &format!("<line x1=\"{0}\" y1=\"{1:.1}\" x2=\"{2}\" y2=\"{1:.1}\" stroke=\"{grid}\"/><text x=\"{3}\" y=\"{4:.1}\" fill=\"{fg}\" text-anchor=\"end\">{5}</text>", ml, sy(y), ml + pw, ml - 5.0, sy(y) + 4.0, fmt_tick(y));
        y += ysx;
    }
    if (xmin..=xmax).contains(&0.0) {
        s += &format!("<line x1=\"{0:.1}\" y1=\"{1}\" x2=\"{0:.1}\" y2=\"{2}\" stroke=\"{fg}\" stroke-width=\"1.5\"/>", sx(0.0), mt, mt + ph);
    }
    if (ymin..=ymax).contains(&0.0) {
        s += &format!("<line x1=\"{0}\" y1=\"{1:.1}\" x2=\"{2}\" y2=\"{1:.1}\" stroke=\"{fg}\" stroke-width=\"1.5\"/>", ml, sy(0.0), ml + pw);
    }
    s += &format!("<clipPath id=\"c\"><rect x=\"{ml}\" y=\"{mt}\" width=\"{pw}\" height=\"{ph}\"/></clipPath><g clip-path=\"url(#c)\" fill=\"none\" stroke-width=\"2\" stroke-linejoin=\"round\">");
    for (i, lines) in all.iter().enumerate() {
        for l in lines {
            let d: String = l.iter().enumerate().map(|(k, p)| format!("{}{:.1},{:.1}", if k == 0 { "M" } else { "L" }, sx(p.0), sy(p.1))).collect();
            s += &format!("<path d=\"{d}\" stroke=\"{}\"/>", COLORS[i % COLORS.len()]);
        }
    }
    s += "</g>";
    s += &format!("<text x=\"{}\" y=\"{}\" fill=\"{fg}\" text-anchor=\"middle\">{}</text>", ml + pw / 2.0, height - 6.0, esc(&plot.x_label));
    s += &format!("<text transform=\"translate(12,{}) rotate(-90)\" fill=\"{fg}\" text-anchor=\"middle\">{}</text>", mt + ph / 2.0, esc(&plot.y_label));
    for (i, c) in plot.curves.iter().enumerate() {
        let label = match c {
            Curve::Cartesian(f) => format!("y = {f}"),
            Curve::Parametric { x, y, .. } => format!("({x}, {y})"),
            Curve::Polar { r, .. } => format!("r = {r}"),
        };
        s += &format!("<text x=\"{}\" y=\"{}\" fill=\"{}\">{}</text>", ml + 8.0, mt + 14.0 + 14.0 * i as f64, COLORS[i % COLORS.len()], esc(&label));
    }
    s += "</svg>";
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str, x: f64) -> f64 {
        eval(&parse(s).unwrap(), &[("x", x)])
    }

    #[test]
    fn expression_parser_and_evaluator() {
        assert!((ev("2x + 1", 3.0) - 7.0).abs() < 1e-12);
        assert!((ev("x^2 - 4", 3.0) - 5.0).abs() < 1e-12);
        assert!((ev("-x^2", 3.0) + 9.0).abs() < 1e-12, "unary minus binds looser than ^");
        assert!((ev("2^3^2", 0.0) - 512.0).abs() < 1e-9, "right associative");
        assert!((ev("sin(pi/2) + cos(0)", 0.0) - 2.0).abs() < 1e-12);
        assert!((ev("3(x+1)", 1.0) - 6.0).abs() < 1e-12, "implicit multiplication");
        assert!((ev("e^0 + sqrt(16) + abs(-2) + max(1, 5)", 0.0) - 12.0).abs() < 1e-12);
        assert!(ev("unknown(x)", 1.0).is_nan());
        assert!(parse("2 +").is_err() && parse("(1").is_err() && parse("").is_err() && parse("1 $ 2").is_err());
        assert!((ev("2·x − 1", 2.0) - 3.0).abs() < 1e-12, "unicode operators from handwriting");
    }

    #[test]
    fn asymptotes_break_the_line() {
        let l = sample(&Curve::Cartesian("1/x".into()), (-2.0, 2.0), 400).unwrap();
        assert!(l.len() >= 2, "1/x is drawn as two branches, {} found", l.len());
        let t = sample(&Curve::Cartesian("log(x)".into()), (-1.0, 1.0), 100).unwrap();
        assert!(t.iter().flatten().all(|p| p.0 > 0.0), "no points where ln is undefined");
    }

    #[test]
    fn renders_multi_function_parametric_and_polar_svg() {
        let p = Plot {
            curves: vec![Curve::Cartesian("sin(x)".into()), Curve::Cartesian("x^2/10".into()), Curve::Parametric { x: "cos(t)".into(), y: "sin(t)".into(), t: (0.0, std::f64::consts::TAU) }, Curve::Polar { r: "1 + cos(theta)".into(), theta: (0.0, std::f64::consts::TAU) }],
            x_range: (-6.0, 6.0),
            y_range: None,
            x_label: "x".into(),
            y_label: "f(x)".into(),
        };
        let svg = render_svg(&p, 640.0, 400.0, true).unwrap();
        assert!(svg.starts_with("<svg") && svg.matches("<path").count() >= 4);
        assert!(svg.contains("y = sin(x)") && svg.contains("r = 1 + cos(theta)") && svg.contains(">f(x)<"), "legend and axis labels");
        assert!(render_svg(&Plot { curves: vec![Curve::Cartesian("(".into())], ..p.clone() }, 100.0, 100.0, false).is_err());
    }

    #[test]
    fn nice_ticks() {
        assert_eq!(nice_step(10.0), 2.0);
        assert_eq!(fmt_tick(2.5), "2.5");
        assert_eq!(fmt_tick(3.0), "3");
    }
}
