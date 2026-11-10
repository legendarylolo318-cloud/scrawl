//! Desmos-style graphing: type expressions, get curves. Supports y = f(x), x = c, implicit
//! relations F(x,y) = G(x,y), inequalities (shaded), parametric curves, points, and named
//! parameters (`a = 2` or any undefined letter) that become sliders. Plus numeric analysis:
//! roots, extrema, intersections, derivative and definite integral.

use crate::plot::{eval, parse, Ast};
use std::collections::BTreeMap;

const BUILTIN: &[&str] = &[
    "x", "y", "t", "theta", "θ", "pi", "π", "e", "sin", "cos", "tan", "asin", "acos", "atan", "arcsin", "arccos",
    "arctan", "sinh", "cosh", "tanh", "exp", "ln", "log", "sqrt", "abs", "floor", "ceil", "min", "max", "pow",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// y = f(x)
    Explicit(Ast),
    /// x = c
    Vertical(Ast),
    /// F(x, y) = 0 (from `lhs = rhs`)
    Implicit(Ast),
    /// F(x, y) <cmp> 0, shaded
    Inequality(Ast, Cmp),
    /// (x(t), y(t)), t from 0 to 2π
    Parametric(Ast, Ast),
    Point(Ast, Ast),
    /// `a = 3`: defines a parameter
    Param(String, Ast),
    /// Imported / computed data series (from MATLAB scripts or .mat files).
    Data(Vec<(f64, f64)>),
    Invalid(String),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub text: String,
    pub kind: Kind,
    pub visible: bool,
}

fn split_top(s: &str, ch: char) -> Option<(String, String)> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == ch && depth == 0 => return Some((s[..i].to_string(), s[i + c.len_utf8()..].to_string())),
            _ => {}
        }
    }
    None
}

fn sub(l: Ast, r: Ast) -> Ast {
    Ast::Bin('-', Box::new(l), Box::new(r))
}

fn vars_in(a: &Ast, out: &mut Vec<String>) {
    match a {
        Ast::Var(v) => out.push(v.clone()),
        Ast::Neg(x) => vars_in(x, out),
        Ast::Bin(_, l, r) => {
            vars_in(l, out);
            vars_in(r, out);
        }
        Ast::Call(_, args) => args.iter().for_each(|x| vars_in(x, out)),
        Ast::Num(_) => {}
    }
}

/// Classify one line of input.
pub fn classify(text: &str) -> Kind {
    let t = text.trim().replace('−', "-").replace('≤', "<=").replace('≥', ">=");
    if t.is_empty() {
        return Kind::Invalid("empty".into());
    }
    // point or parametric: "(a, b)"
    if t.starts_with('(') && t.ends_with(')') {
        if let Some((a, b)) = split_top(&t[1..t.len() - 1], ',') {
            return match (parse(&a), parse(&b)) {
                (Ok(x), Ok(y)) => {
                    let mut v = Vec::new();
                    vars_in(&x, &mut v);
                    vars_in(&y, &mut v);
                    if v.iter().any(|n| n == "t") { Kind::Parametric(x, y) } else { Kind::Point(x, y) }
                }
                (Err(e), _) | (_, Err(e)) => Kind::Invalid(e),
            };
        }
    }
    for (op, cmp) in [("<=", Cmp::Le), (">=", Cmp::Ge), ("<", Cmp::Lt), (">", Cmp::Gt)] {
        if let Some(i) = t.find(op) {
            return match (parse(&t[..i]), parse(&t[i + op.len()..])) {
                (Ok(l), Ok(r)) => Kind::Inequality(sub(l, r), cmp),
                (Err(e), _) | (_, Err(e)) => Kind::Invalid(e),
            };
        }
    }
    if let Some((l, r)) = split_top(&t, '=') {
        let (l, r) = (l.trim().to_string(), r.trim().to_string());
        let (pl, pr) = match (parse(&l), parse(&r)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return Kind::Invalid(e),
        };
        if l == "y" {
            return Kind::Explicit(pr);
        }
        if r == "y" {
            return Kind::Explicit(pl);
        }
        if l == "x" {
            let mut v = Vec::new();
            vars_in(&pr, &mut v);
            if !v.iter().any(|n| n == "y" || n == "x") {
                return Kind::Vertical(pr);
            }
        }
        // `a = 3`: a single non-reserved letter on the left with a constant right side
        if l.chars().count() == 1 && !BUILTIN.contains(&l.as_str()) {
            let mut v = Vec::new();
            vars_in(&pr, &mut v);
            if v.iter().all(|n| BUILTIN.contains(&n.as_str()) && n != "x" && n != "y") {
                return Kind::Param(l, pr);
            }
        }
        return Kind::Implicit(sub(pl, pr));
    }
    // bare expression: f(x)
    match parse(&t) {
        Ok(a) => Kind::Explicit(a),
        Err(e) => Kind::Invalid(e),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub items: Vec<Item>,
    pub params: BTreeMap<String, Param>,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub cx: f64,
    pub cy: f64,
    /// pixels per unit
    pub scale: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self { cx: 0.0, cy: 0.0, scale: 40.0 }
    }
}

impl Viewport {
    pub fn to_px(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
        (w / 2.0 + (x - self.cx) * self.scale, h / 2.0 - (y - self.cy) * self.scale)
    }
    pub fn to_world(&self, px: f64, py: f64, w: f64, h: f64) -> (f64, f64) {
        (self.cx + (px - w / 2.0) / self.scale, self.cy - (py - h / 2.0) / self.scale)
    }
    pub fn zoom_at(&mut self, factor: f64, px: f64, py: f64, w: f64, h: f64) {
        let before = self.to_world(px, py, w, h);
        self.scale = (self.scale * factor).clamp(0.5, 100_000.0);
        let after = self.to_world(px, py, w, h);
        self.cx += before.0 - after.0;
        self.cy += before.1 - after.1;
    }
    pub fn pan_px(&mut self, dx: f64, dy: f64) {
        self.cx -= dx / self.scale;
        self.cy += dy / self.scale;
    }
}

impl Graph {
    pub fn set_lines(&mut self, lines: &[&str]) {
        self.items = lines.iter().map(|l| Item { text: l.to_string(), kind: classify(l), visible: true }).collect();
        self.sync_params();
    }

    /// Create slider parameters for explicit `a = 3` lines and for any undefined identifier.
    pub fn sync_params(&mut self) {
        let mut wanted: BTreeMap<String, f64> = BTreeMap::new();
        let mut used = Vec::new();
        for it in &self.items {
            match &it.kind {
                Kind::Param(n, v) => {
                    wanted.insert(n.clone(), eval(v, &[]));
                }
                Kind::Explicit(a) | Kind::Vertical(a) | Kind::Implicit(a) | Kind::Inequality(a, _) => vars_in(a, &mut used),
                Kind::Parametric(a, b) | Kind::Point(a, b) => {
                    vars_in(a, &mut used);
                    vars_in(b, &mut used);
                }
                Kind::Invalid(_) | Kind::Data(_) => {}
            }
        }
        for u in used {
            if !BUILTIN.contains(&u.as_str()) {
                wanted.entry(u).or_insert(1.0);
            }
        }
        let old = std::mem::take(&mut self.params);
        for (n, v) in wanted {
            let p = old.get(&n).copied().unwrap_or(Param { value: v, min: (v - 10.0).min(-10.0), max: (v + 10.0).max(10.0) });
            let p = if explicit_value(&self.items, &n).is_some() { Param { value: v, ..p } } else { p };
            self.params.insert(n, p);
        }
    }

    pub fn set_param(&mut self, name: &str, value: f64) {
        if let Some(p) = self.params.get_mut(name) {
            p.value = value.clamp(p.min, p.max);
        }
    }

    fn bind(&self) -> Vec<(String, f64)> {
        self.params.iter().map(|(k, p)| (k.clone(), p.value)).collect()
    }

    /// Evaluate an AST at (x, y) with the current parameters.
    pub fn at(&self, a: &Ast, x: f64, y: f64) -> f64 {
        let mut vars: Vec<(&str, f64)> = vec![("x", x), ("y", y), ("t", x)];
        let bound = self.bind();
        vars.extend(bound.iter().map(|(k, v)| (k.as_str(), *v)));
        eval(a, &vars)
    }

    fn at_t(&self, a: &Ast, t: f64) -> f64 {
        let mut vars: Vec<(&str, f64)> = vec![("t", t), ("theta", t)];
        let bound = self.bind();
        vars.extend(bound.iter().map(|(k, v)| (k.as_str(), *v)));
        eval(a, &vars)
    }
}

fn explicit_value(items: &[Item], name: &str) -> Option<f64> {
    items.iter().find_map(|i| match &i.kind {
        Kind::Param(n, v) if n == name => Some(eval(v, &[])),
        _ => None,
    })
}

// ---- analysis ------------------------------------------------------------------------------

fn bisect(f: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64) -> f64 {
    let (mut fa, _) = (f(a), f(b));
    for _ in 0..80 {
        let m = (a + b) / 2.0;
        let fm = f(m);
        if fa * fm <= 0.0 {
            b = m;
        } else {
            a = m;
            fa = fm;
        }
    }
    (a + b) / 2.0
}

/// Roots of f on [a, b] (sign changes; skips poles).
pub fn roots(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> Vec<f64> {
    let n = 2000;
    let mut out: Vec<f64> = Vec::new();
    let mut prev = (a, f(a));
    for i in 1..=n {
        let x = a + (b - a) * i as f64 / n as f64;
        let y = f(x);
        if prev.1.is_finite() && y.is_finite() && (prev.1 == 0.0 || prev.1 * y < 0.0) {
            let r = bisect(f, prev.0, x);
            if f(r).abs() < 1e-6 * (1.0 + prev.1.abs().max(y.abs())) && out.last().is_none_or(|l| (l - r).abs() > 1e-7) {
                out.push(r);
            }
        }
        prev = (x, y);
    }
    out
}

pub fn derivative(f: &dyn Fn(f64) -> f64, x: f64) -> f64 {
    let h = 1e-5 * (1.0 + x.abs());
    (f(x + h) - f(x - h)) / (2.0 * h)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExtremumKind {
    Min,
    Max,
}

/// Local extrema on [a, b].
pub fn extrema(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> Vec<(f64, f64, ExtremumKind)> {
    let d = |x: f64| derivative(f, x);
    roots(&d, a, b)
        .into_iter()
        .filter_map(|x| {
            let h = 1e-3 * (1.0 + x.abs());
            let (l, m, r) = (f(x - h), f(x), f(x + h));
            if m < l && m < r {
                Some((x, m, ExtremumKind::Min))
            } else if m > l && m > r {
                Some((x, m, ExtremumKind::Max))
            } else {
                None
            }
        })
        .collect()
}

/// Intersections of two explicit curves.
pub fn intersections(f: &dyn Fn(f64) -> f64, g: &dyn Fn(f64) -> f64, a: f64, b: f64) -> Vec<(f64, f64)> {
    roots(&|x| f(x) - g(x), a, b).into_iter().map(|x| (x, f(x))).collect()
}

/// Definite integral by Simpson's rule.
pub fn integral(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    let n = 4000;
    let h = (b - a) / n as f64;
    let mut s = f(a) + f(b);
    for i in 1..n {
        s += f(a + i as f64 * h) * if i % 2 == 0 { 2.0 } else { 4.0 };
    }
    s * h / 3.0
}

// ---- rendering -----------------------------------------------------------------------------

pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA.
    pub rgba: Vec<u8>,
    /// Tick labels to draw on top: (x px, y px, text, is_x_axis)
    pub labels: Vec<(f32, f32, String, bool)>,
}

pub const COLORS: [[u8; 3]; 6] = [[0x7a, 0xa2, 0xf7], [0xf7, 0x76, 0x8e], [0x9e, 0xce, 0x6a], [0xe0, 0xaf, 0x68], [0xbb, 0x9a, 0xf7], [0x73, 0xda, 0xca]];

fn nice(range: f64) -> f64 {
    let raw = range / 8.0;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    mag * if n < 1.5 { 1.0 } else if n < 3.5 { 2.0 } else if n < 7.5 { 5.0 } else { 10.0 }
}

fn fmt(v: f64) -> String {
    if v.abs() < 1e-9 {
        "0".into()
    } else if (v - v.round()).abs() < 1e-9 && v.abs() < 1e9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.3}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn render(g: &Graph, vp: &Viewport, w: u32, h: u32, dark: bool) -> Frame {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let (wf, hf) = (w as f64, h as f64);
    let mut pm = Pixmap::new(w.max(1), h.max(1)).unwrap();
    let (bg, grid, axis) = if dark { ((0x1a, 0x1b, 0x26), (0x2f, 0x35, 0x4d), (0x8a, 0x93, 0xb8)) } else { ((255, 255, 255), (0xe3, 0xe6, 0xef), (0x55, 0x5a, 0x70)) };
    pm.fill(Color::from_rgba8(bg.0, bg.1, bg.2, 255));
    let paint = |c: (u8, u8, u8), a: u8| {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba8(c.0, c.1, c.2, a));
        p.anti_alias = true;
        p
    };
    let line = |pm: &mut Pixmap, p: &Paint, wd: f32, pts: &[(f32, f32)]| {
        let mut pb = PathBuilder::new();
        for (i, q) in pts.iter().enumerate() {
            if i == 0 { pb.move_to(q.0, q.1) } else { pb.line_to(q.0, q.1) }
        }
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, p, &Stroke { width: wd, line_join: tiny_skia::LineJoin::Round, ..Stroke::default() }, Transform::identity(), None);
        }
    };
    let (x0, y1) = vp.to_world(0.0, 0.0, wf, hf);
    let (x1, y0) = vp.to_world(wf, hf, wf, hf);
    let step = nice(x1 - x0);
    let mut labels = Vec::new();
    let mut gx = (x0 / step).ceil() * step;
    while gx <= x1 {
        let (px, _) = vp.to_px(gx, 0.0, wf, hf);
        line(&mut pm, &paint(grid, 255), 1.0, &[(px as f32, 0.0), (px as f32, h as f32)]);
        if gx.abs() > step / 2.0 {
            labels.push((px as f32, (vp.to_px(0.0, 0.0, wf, hf).1.clamp(4.0, hf - 18.0) + 4.0) as f32, fmt(gx), true));
        }
        gx += step;
    }
    let ystep = nice(y1 - y0);
    let mut gy = (y0 / ystep).ceil() * ystep;
    while gy <= y1 {
        let (_, py) = vp.to_px(0.0, gy, wf, hf);
        line(&mut pm, &paint(grid, 255), 1.0, &[(0.0, py as f32), (w as f32, py as f32)]);
        if gy.abs() > ystep / 2.0 {
            labels.push(((vp.to_px(0.0, 0.0, wf, hf).0.clamp(24.0, wf - 6.0) - 22.0) as f32, py as f32 - 7.0, fmt(gy), false));
        }
        gy += ystep;
    }
    let (ox, oy) = vp.to_px(0.0, 0.0, wf, hf);
    line(&mut pm, &paint(axis, 255), 1.6, &[(ox as f32, 0.0), (ox as f32, h as f32)]);
    line(&mut pm, &paint(axis, 255), 1.6, &[(0.0, oy as f32), (w as f32, oy as f32)]);

    for (idx, it) in g.items.iter().enumerate().filter(|(_, i)| i.visible) {
        let col = COLORS[idx % COLORS.len()];
        let p = paint((col[0], col[1], col[2]), 255);
        match &it.kind {
            Kind::Explicit(a) => {
                let mut seg: Vec<(f32, f32)> = Vec::new();
                let mut last_py = f64::NAN;
                for px in 0..=w {
                    let (x, _) = vp.to_world(px as f64, 0.0, wf, hf);
                    let y = g.at(a, x, 0.0);
                    let (_, py) = vp.to_px(0.0, y, wf, hf);
                    if !y.is_finite() || py.abs() > 1e6 || (last_py.is_finite() && (py - last_py).abs() > hf * 3.0) {
                        if seg.len() > 1 { line(&mut pm, &p, 2.4, &seg); }
                        seg.clear();
                    }
                    if y.is_finite() && py.abs() <= 1e6 {
                        seg.push((px as f32, py as f32));
                        last_py = py;
                    } else {
                        last_py = f64::NAN;
                    }
                }
                if seg.len() > 1 { line(&mut pm, &p, 2.4, &seg); }
            }
            Kind::Vertical(a) => {
                let x = g.at(a, 0.0, 0.0);
                if x.is_finite() {
                    let (px, _) = vp.to_px(x, 0.0, wf, hf);
                    line(&mut pm, &p, 2.4, &[(px as f32, 0.0), (px as f32, h as f32)]);
                }
            }
            Kind::Parametric(a, b) => {
                let pts: Vec<(f32, f32)> = (0..=800)
                    .filter_map(|i| {
                        let t = std::f64::consts::TAU * i as f64 / 800.0;
                        let (x, y) = (g.at_t(a, t), g.at_t(b, t));
                        (x.is_finite() && y.is_finite()).then(|| { let q = vp.to_px(x, y, wf, hf); (q.0 as f32, q.1 as f32) })
                    })
                    .collect();
                if pts.len() > 1 { line(&mut pm, &p, 2.4, &pts); }
            }
            Kind::Point(a, b) => {
                let (x, y) = (g.at(a, 0.0, 0.0), g.at(b, 0.0, 0.0));
                if x.is_finite() && y.is_finite() {
                    let (px, py) = vp.to_px(x, y, wf, hf);
                    let mut pb = PathBuilder::new();
                    pb.push_circle(px as f32, py as f32, 5.0);
                    if let Some(path) = pb.finish() {
                        pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
                    }
                }
            }
            Kind::Implicit(a) => marching(g, a, vp, w, h, |pts| line(&mut pm, &p, 2.2, pts)),
            Kind::Inequality(a, cmp) => {
                let shade = paint((col[0], col[1], col[2]), 70);
                let cell = 4u32;
                for py in (0..h).step_by(cell as usize) {
                    for px in (0..w).step_by(cell as usize) {
                        let (x, y) = vp.to_world(px as f64 + 2.0, py as f64 + 2.0, wf, hf);
                        let v = g.at(a, x, y);
                        let on = match cmp { Cmp::Lt => v < 0.0, Cmp::Le => v <= 0.0, Cmp::Gt => v > 0.0, Cmp::Ge => v >= 0.0 };
                        if on && v.is_finite() {
                            if let Some(r) = tiny_skia::Rect::from_xywh(px as f32, py as f32, cell as f32, cell as f32) {
                                pm.fill_rect(r, &shade, Transform::identity(), None);
                            }
                        }
                    }
                }
                marching(g, a, vp, w, h, |pts| line(&mut pm, &p, 1.6, pts));
            }
            Kind::Data(pts) => {
                let px: Vec<(f32, f32)> = pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).map(|p| { let q = vp.to_px(p.0, p.1, wf, hf); (q.0 as f32, q.1 as f32) }).collect();
                if px.len() > 1 { line(&mut pm, &p, 2.2, &px); }
            }
            Kind::Param(..) | Kind::Invalid(_) => {}
        }
    }
    Frame { width: w, height: h, rgba: pm.data().to_vec(), labels }
}

/// Marching squares for F(x, y) = 0 on a coarse grid; calls `draw` for each short segment.
fn marching(g: &Graph, a: &Ast, vp: &Viewport, w: u32, h: u32, mut draw: impl FnMut(&[(f32, f32)])) {
    let cell = 6.0;
    let (nx, ny) = ((w as f64 / cell).ceil() as usize + 1, (h as f64 / cell).ceil() as usize + 1);
    let (wf, hf) = (w as f64, h as f64);
    let val = |i: usize, j: usize| {
        let (x, y) = vp.to_world(i as f64 * cell, j as f64 * cell, wf, hf);
        g.at(a, x, y)
    };
    let field: Vec<f64> = (0..ny).flat_map(|j| (0..nx).map(move |i| (i, j))).map(|(i, j)| val(i, j)).collect();
    let f = |i: usize, j: usize| field[j * nx + i];
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let c = [f(i, j), f(i + 1, j), f(i + 1, j + 1), f(i, j + 1)];
            if c.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let (px, py) = (i as f64 * cell, j as f64 * cell);
            let corners = [(px, py), (px + cell, py), (px + cell, py + cell), (px, py + cell)];
            let mut pts = Vec::new();
            for e in 0..4 {
                let (a0, b0) = (c[e], c[(e + 1) % 4]);
                if (a0 < 0.0) != (b0 < 0.0) {
                    let t = a0 / (a0 - b0);
                    let (p, q) = (corners[e], corners[(e + 1) % 4]);
                    pts.push(((p.0 + (q.0 - p.0) * t) as f32, (p.1 + (q.1 - p.1) * t) as f32));
                }
            }
            if pts.len() == 2 {
                draw(&pts);
            } else if pts.len() == 4 {
                draw(&pts[0..2]);
                draw(&pts[2..4]);
            }
        }
    }
}

/// The points of interest for a graph: roots, extrema, y-intercepts, intersections.
pub fn interesting_points(g: &Graph, x0: f64, x1: f64) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    let funcs: Vec<(usize, &Ast)> = g.items.iter().enumerate().filter_map(|(i, it)| match &it.kind { Kind::Explicit(a) if it.visible => Some((i, a)), _ => None }).collect();
    for &(_, a) in &funcs {
        let f = |x: f64| g.at(a, x, 0.0);
        for r in roots(&f, x0, x1) {
            out.push((r, 0.0, "zero".to_string()));
        }
        for (x, y, k) in extrema(&f, x0, x1) {
            out.push((x, y, if k == ExtremumKind::Min { "min" } else { "max" }.to_string()));
        }
        let y0 = f(0.0);
        if y0.is_finite() && (x0..=x1).contains(&0.0) {
            out.push((0.0, y0, "y-intercept".into()));
        }
    }
    for (n, &(_, a)) in funcs.iter().enumerate() {
        for &(_, b) in &funcs[n + 1..] {
            for (x, y) in intersections(&|x| g.at(a, x, 0.0), &|x| g.at(b, x, 0.0), x0, x1) {
                out.push((x, y, "intersection".into()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(lines: &[&str]) -> Graph {
        let mut g = Graph::default();
        g.set_lines(lines);
        g
    }

    #[test]
    fn classifies_inputs_like_desmos() {
        assert!(matches!(classify("y = x^2"), Kind::Explicit(_)));
        assert!(matches!(classify("sin(x)"), Kind::Explicit(_)));
        assert!(matches!(classify("x = 3"), Kind::Vertical(_)));
        assert!(matches!(classify("x^2 + y^2 = 1"), Kind::Implicit(_)));
        assert!(matches!(classify("y > x^2"), Kind::Inequality(_, Cmp::Gt)));
        assert!(matches!(classify("y ≤ 2x"), Kind::Inequality(_, Cmp::Le)));
        assert!(matches!(classify("(cos(t), sin(t))"), Kind::Parametric(..)));
        assert!(matches!(classify("(2, 3)"), Kind::Point(..)));
        assert!(matches!(classify("a = 2.5"), Kind::Param(..)));
        assert!(matches!(classify("y = (1 +"), Kind::Invalid(_)));
    }

    #[test]
    fn sliders_appear_for_parameters_and_drive_the_curve() {
        let mut g = graph(&["a = 2", "y = a x^2"]);
        assert_eq!(g.params["a"].value, 2.0);
        let ast = match &g.items[1].kind { Kind::Explicit(a) => a.clone(), _ => panic!() };
        assert_eq!(g.at(&ast, 3.0, 0.0), 18.0);
        g.set_param("a", 5.0);
        assert_eq!(g.at(&ast, 3.0, 0.0), 45.0);
        // an undefined letter gets a slider automatically
        let g = graph(&["y = m x + b"]);
        assert!(g.params.contains_key("m") && g.params.contains_key("b"));
        assert!(!g.params.contains_key("x"));
    }

    #[test]
    fn calculus_tools() {
        let f = |x: f64| x * x - 4.0;
        let r = roots(&f, -5.0, 5.0);
        assert_eq!(r.len(), 2);
        assert!((r[0] + 2.0).abs() < 1e-6 && (r[1] - 2.0).abs() < 1e-6);
        assert!((derivative(&f, 3.0) - 6.0).abs() < 1e-4);
        assert!((integral(&|x| x * x, 0.0, 3.0) - 9.0).abs() < 1e-6);
        let e = extrema(&|x| x * x * x - 3.0 * x, -3.0, 3.0);
        assert_eq!(e.len(), 2);
        assert!(e.iter().any(|(x, y, k)| (x + 1.0).abs() < 1e-3 && (y - 2.0).abs() < 1e-3 && *k == ExtremumKind::Max));
        let i = intersections(&|x| x * x, &|x| x + 2.0, -5.0, 5.0);
        assert_eq!(i.len(), 2);
        assert!(roots(&|x| 1.0 / x, -1.0, 1.0).is_empty(), "poles are not roots");
    }

    #[test]
    fn interesting_points_of_a_parabola_and_line() {
        let g = graph(&["y = x^2 - 1", "y = x + 1"]);
        let pts = interesting_points(&g, -5.0, 5.0);
        let has = |label: &str| pts.iter().filter(|p| p.2 == label).count();
        assert_eq!(has("zero"), 3, "x=±1 for the parabola, x=-1 for the line (the second zero counts once per curve)");
        assert_eq!(has("min"), 1);
        assert_eq!(has("intersection"), 2);
        assert!(has("y-intercept") >= 2);
    }

    #[test]
    fn viewport_zoom_keeps_the_cursor_point_fixed() {
        let mut vp = Viewport::default();
        let before = vp.to_world(300.0, 100.0, 800.0, 600.0);
        vp.zoom_at(2.0, 300.0, 100.0, 800.0, 600.0);
        let after = vp.to_world(300.0, 100.0, 800.0, 600.0);
        assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
        let c = vp.to_world(400.0, 300.0, 800.0, 600.0);
        vp.pan_px(40.0, 0.0);
        assert!(vp.to_world(400.0, 300.0, 800.0, 600.0).0 < c.0, "dragging right moves the view left");
    }

    #[test]
    fn renders_every_kind_without_panicking_and_draws_ink() {
        let g = graph(&["y = sin(x)", "x = 2", "x^2 + y^2 = 4", "y > x - 3", "(cos(t) * 3, sin(t) * 3)", "(1, 1)", "y = tan(x)", "y = ln(x)", "bad ((", "a = 1"]);
        let f = render(&g, &Viewport::default(), 400, 300, true);
        assert_eq!(f.rgba.len(), 400 * 300 * 4);
        let bg = [0x1a, 0x1b, 0x26];
        let colored = f.rgba.chunks(4).filter(|p| p[0] != bg[0] || p[1] != bg[1] || p[2] != bg[2]).count();
        assert!(colored > 3000, "{colored}");
        assert!(f.labels.iter().any(|l| l.2 == "5" && l.3), "x tick labels");
        // invisible items draw nothing extra
        let mut g2 = graph(&["y = x"]);
        let a = render(&g2, &Viewport::default(), 100, 100, false).rgba;
        g2.items[0].visible = false;
        assert_ne!(a, render(&g2, &Viewport::default(), 100, 100, false).rgba);
    }
}
