//! MATLAB compatibility, in pure Rust:
//! * `.mat` (v5/v7, non-HDF5) data files via the `matfile` crate;
//! * a small `.m` interpreter covering numeric scripts: scalars, vectors, matrices, ranges,
//!   element-wise and matrix operators, common functions, indexing, `plot`/`hold`/labels, `disp`.
//!
//! Not supported (clearly reported as errors): `function` definitions, control flow (`for`, `if`,
//! `while`), cell arrays, structs, strings beyond labels, toolboxes, and HDF5-based v7.3 files.

use std::collections::HashMap;

/// Column-major matrix like MATLAB's.
#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Mat {
    pub fn scalar(v: f64) -> Self {
        Self { rows: 1, cols: 1, data: vec![v] }
    }
    pub fn row(v: Vec<f64>) -> Self {
        Self { rows: 1, cols: v.len(), data: v }
    }
    fn is_scalar(&self) -> bool {
        self.data.len() == 1
    }
    fn at(&self, r: usize, c: usize) -> f64 {
        self.data[c * self.rows + r]
    }
    fn map(&self, mut f: impl FnMut(f64) -> f64) -> Mat {
        Mat { rows: self.rows, cols: self.cols, data: self.data.iter().map(|v| f(*v)).collect() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    pub name: String,
    pub value: Mat,
}

/// Load numeric variables from a `.mat` file (v5/v7).
pub fn load_mat(bytes: &[u8]) -> Result<Vec<Var>, String> {
    let f = matfile::MatFile::parse(std::io::Cursor::new(bytes)).map_err(|e| format!("cannot read .mat file (HDF5-based v7.3 files are not supported): {e}"))?;
    let mut out = Vec::new();
    for a in f.arrays() {
        let sz = a.size().to_vec();
        if sz.len() != 2 {
            continue;
        }
        let data: Vec<f64> = match a.data() {
            matfile::NumericData::Double { real, .. } => real.clone(),
            matfile::NumericData::Single { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::Int8 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::UInt8 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::Int16 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::UInt16 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::Int32 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::UInt32 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::Int64 { real, .. } => real.iter().map(|v| *v as f64).collect(),
            matfile::NumericData::UInt64 { real, .. } => real.iter().map(|v| *v as f64).collect(),
        };
        out.push(Var { name: a.name().to_string(), value: Mat { rows: sz[0], cols: sz[1], data } });
    }
    Ok(out)
}

#[derive(Clone, Debug, Default)]
pub struct Series {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Debug, Default)]
pub struct Output {
    pub series: Vec<Series>,
    pub title: String,
    pub xlabel: String,
    pub ylabel: String,
    pub text: String,
}

// ---- lexer ---------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum T {
    Num(f64),
    Id(String),
    Str(String),
    Op(&'static str),
    Semi,
    Comma,
    Newline,
}

#[derive(Clone, Debug)]
struct Tok {
    t: T,
    space_before: bool,
    space_after: bool,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out: Vec<Tok> = Vec::new();
    let mut bracket_depth = 0i32;
    while i < cs.len() {
        let c = cs[i];
        let space_before = i > 0 && (cs[i - 1] == ' ' || cs[i - 1] == '\t');
        let push = |out: &mut Vec<Tok>, t: T, end: usize| {
            let space_after = cs.get(end).is_some_and(|c| *c == ' ' || *c == '\t');
            out.push(Tok { t, space_before, space_after });
        };
        match c {
            ' ' | '\t' | '\r' => i += 1,
            '%' | '#' => while i < cs.len() && cs[i] != '\n' { i += 1 },
            '.' if cs.get(i + 1) == Some(&'.') && cs.get(i + 2) == Some(&'.') => {
                while i < cs.len() && cs[i] != '\n' { i += 1 }
                i += 1; // line continuation
            }
            '\n' => { push(&mut out, if bracket_depth > 0 { T::Semi } else { T::Newline }, i + 1); i += 1 }
            ';' => { push(&mut out, T::Semi, i + 1); i += 1 }
            ',' => { push(&mut out, T::Comma, i + 1); i += 1 }
            '0'..='9' | '.' if c.is_ascii_digit() || cs.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let st = i;
                while i < cs.len() && (cs[i].is_ascii_digit() || (cs[i] == '.' && !matches!(cs.get(i + 1), Some('*' | '/' | '^' | '\'')))) { i += 1 }
                if i < cs.len() && (cs[i] == 'e' || cs[i] == 'E') && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '-' || *d == '+') {
                    i += 2;
                    while i < cs.len() && cs[i].is_ascii_digit() { i += 1 }
                }
                let s: String = cs[st..i].iter().collect();
                push(&mut out, T::Num(s.parse().map_err(|_| format!("bad number {s}"))?), i);
            }
            c if c.is_alphabetic() || c == '_' => {
                let st = i;
                while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') { i += 1 }
                push(&mut out, T::Id(cs[st..i].iter().collect()), i);
            }
            '\'' => {
                // transpose after a value, otherwise a string literal
                let after_value = out.last().is_some_and(|t| matches!(&t.t, T::Num(_) | T::Id(_) | T::Op(")") | T::Op("]") | T::Op("'")) ) && !space_before_in_matrix(&out, space_before, bracket_depth);
                if after_value {
                    push(&mut out, T::Op("'"), i + 1);
                    i += 1;
                } else {
                    let st = i + 1;
                    i += 1;
                    while i < cs.len() && cs[i] != '\'' && cs[i] != '\n' { i += 1 }
                    if i >= cs.len() || cs[i] != '\'' { return Err("unterminated string".into()); }
                    push(&mut out, T::Str(cs[st..i].iter().collect()), i + 1);
                    i += 1;
                }
            }
            '"' => {
                let st = i + 1;
                i += 1;
                while i < cs.len() && cs[i] != '"' { i += 1 }
                push(&mut out, T::Str(cs[st..i.min(cs.len())].iter().collect()), i + 1);
                i += 1;
            }
            _ => {
                let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
                let op: &'static str = match two.as_str() {
                    ".*" => ".*", "./" => "./", ".^" => ".^", ".'" => "'", "==" => "==", "~=" => "~=", "<=" => "<=", ">=" => ">=",
                    _ => match c {
                        '+' => "+", '-' => "-", '*' => "*", '/' => "/", '^' => "^", '(' => "(", ')' => ")", '[' => "[", ']' => "]",
                        '=' => "=", ':' => ":", '<' => "<", '>' => ">", '~' => "~", '{' | '}' | '@' => return Err(format!("unsupported syntax '{c}' (cells and function handles are not supported)")),
                        _ => return Err(format!("unexpected character '{c}'")),
                    },
                };
                match op {
                    "(" | "[" => bracket_depth += 1,
                    ")" | "]" => bracket_depth -= 1,
                    _ => {}
                }
                push(&mut out, T::Op(op), i + op.len().max(1));
                i += if two.starts_with('.') || ["==", "~=", "<=", ">="].contains(&two.as_str()) { 2 } else { 1 };
            }
        }
    }
    Ok(out)
}

fn space_before_in_matrix(out: &[Tok], space_before: bool, depth: i32) -> bool {
    depth > 0 && space_before && out.last().is_some()
}

// ---- parser / evaluator (direct interpretation over tokens) --------------------------------

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Str(String),
    Var(String),
    Colon,
    End,
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Transpose(Box<Expr>),
    Call(String, Vec<Expr>),
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Matrix(Vec<Vec<Expr>>),
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
    in_matrix: bool,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok { t: T::Op(o), .. }) if *o == op)
    }
    fn eat(&mut self, op: &str) -> bool {
        if self.is(op) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<Expr, String> {
        self.range()
    }
    fn range(&mut self) -> Result<Expr, String> {
        let a = self.compare()?;
        if self.is(":") {
            self.i += 1;
            let b = self.compare()?;
            if self.is(":") {
                self.i += 1;
                let c = self.compare()?;
                return Ok(Expr::Range(Box::new(a), Some(Box::new(b)), Box::new(c)));
            }
            return Ok(Expr::Range(Box::new(a), None, Box::new(b)));
        }
        Ok(a)
    }
    fn compare(&mut self) -> Result<Expr, String> {
        let mut l = self.add()?;
        for op in ["==", "~=", "<=", ">=", "<", ">"] {
            if self.is(op) {
                self.i += 1;
                let r = self.add()?;
                l = Expr::Bin(op, Box::new(l), Box::new(r));
                break;
            }
        }
        Ok(l)
    }
    /// In a matrix literal, `[1 -2]` is two elements but `[1 - 2]` and `[1 -2]`-without-space are not.
    fn starts_new_element(&self) -> bool {
        self.in_matrix && matches!(self.peek(), Some(t) if matches!(t.t, T::Op("+") | T::Op("-")) && t.space_before && !t.space_after)
    }
    fn add(&mut self) -> Result<Expr, String> {
        let mut l = self.mul()?;
        loop {
            if self.starts_new_element() {
                break;
            }
            let op = if self.is("+") { "+" } else if self.is("-") { "-" } else { break };
            self.i += 1;
            let r = self.mul()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn mul(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        loop {
            let op = ["*", "/", ".*", "./"].into_iter().find(|o| self.is(o));
            let Some(op) = op else { break };
            self.i += 1;
            let r = self.unary()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("~") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.power()
    }
    fn power(&mut self) -> Result<Expr, String> {
        let mut b = self.postfix()?;
        loop {
            let op = if self.is("^") { "^" } else if self.is(".^") { ".^" } else { break };
            self.i += 1;
            let neg = self.eat("-");
            let mut e = self.postfix()?;
            if neg {
                e = Expr::Neg(Box::new(e));
            }
            b = Expr::Bin(op, Box::new(b), Box::new(e));
        }
        Ok(b)
    }
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            if self.is("'") && !self.starts_new_element() {
                self.i += 1;
                e = Expr::Transpose(Box::new(e));
            } else {
                break;
            }
        }
        Ok(e)
    }
    fn args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        let saved = self.in_matrix;
        self.in_matrix = false;
        if !self.is(")") {
            loop {
                args.push(if self.is(":") && matches!(self.t.get(self.i + 1), Some(Tok { t: T::Op(")"), .. } | Tok { t: T::Comma, .. })) { self.i += 1; Expr::Colon } else { self.expr()? });
                if matches!(self.peek(), Some(Tok { t: T::Comma, .. })) { self.i += 1 } else { break }
            }
        }
        self.in_matrix = saved;
        if !self.eat(")") {
            return Err("missing )".into());
        }
        Ok(args)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        let Some(tok) = self.peek().cloned() else { return Err("unexpected end of input".into()) };
        self.i += 1;
        match tok.t {
            T::Num(n) => Ok(Expr::Num(n)),
            T::Str(s) => Ok(Expr::Str(s)),
            T::Id(name) => {
                if name == "end" { return Ok(Expr::End); }
                if self.is("(") && !(self.in_matrix && self.peek().is_some_and(|t| t.space_before)) {
                    self.i += 1;
                    Ok(Expr::Call(name, self.args()?))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            T::Op("(") => {
                let saved = self.in_matrix;
                self.in_matrix = false;
                let e = self.expr()?;
                self.in_matrix = saved;
                if !self.eat(")") { return Err("missing )".into()); }
                Ok(e)
            }
            T::Op("[") => {
                let saved = self.in_matrix;
                self.in_matrix = true;
                let mut rows: Vec<Vec<Expr>> = vec![vec![]];
                loop {
                    match self.peek().map(|t| t.t.clone()) {
                        Some(T::Op("]")) => { self.i += 1; break; }
                        Some(T::Semi) => { self.i += 1; if !rows.last().unwrap().is_empty() { rows.push(vec![]); } }
                        Some(T::Comma) => self.i += 1,
                        None => return Err("missing ]".into()),
                        _ => { let e = self.expr()?; rows.last_mut().unwrap().push(e); }
                    }
                }
                self.in_matrix = saved;
                if rows.last().is_some_and(Vec::is_empty) { rows.pop(); }
                Ok(Expr::Matrix(rows))
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

pub struct Interp {
    pub vars: HashMap<String, Mat>,
    out: Output,
    hold: bool,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

fn broadcast(a: &Mat, b: &Mat, f: impl Fn(f64, f64) -> f64) -> Result<Mat, String> {
    if a.is_scalar() {
        return Ok(b.map(|v| f(a.data[0], v)));
    }
    if b.is_scalar() {
        return Ok(a.map(|v| f(v, b.data[0])));
    }
    if (a.rows, a.cols) != (b.rows, b.cols) {
        return Err(format!("dimensions do not agree: {}x{} vs {}x{}", a.rows, a.cols, b.rows, b.cols));
    }
    Ok(Mat { rows: a.rows, cols: a.cols, data: a.data.iter().zip(&b.data).map(|(x, y)| f(*x, *y)).collect() })
}

fn matmul(a: &Mat, b: &Mat) -> Result<Mat, String> {
    if a.cols != b.rows {
        return Err(format!("inner dimensions must agree: {}x{} * {}x{}", a.rows, a.cols, b.rows, b.cols));
    }
    let mut d = vec![0.0; a.rows * b.cols];
    for r in 0..a.rows {
        for c in 0..b.cols {
            d[c * a.rows + r] = (0..a.cols).map(|k| a.at(r, k) * b.at(k, c)).sum();
        }
    }
    Ok(Mat { rows: a.rows, cols: b.cols, data: d })
}

fn transpose(m: &Mat) -> Mat {
    let mut d = Vec::with_capacity(m.data.len());
    for r in 0..m.rows {
        for c in 0..m.cols {
            d.push(m.at(r, c));
        }
    }
    Mat { rows: m.cols, cols: m.rows, data: d }
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-12 && v.abs() < 1e15 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.4}")
    }
}

fn fmt_mat(m: &Mat) -> String {
    if m.is_scalar() {
        return fmt_num(m.data[0]);
    }
    (0..m.rows).map(|r| (0..m.cols).map(|c| format!("{:>9}", fmt_num(m.at(r, c)))).collect::<String>()).collect::<Vec<_>>().join("\n")
}

impl Interp {
    pub fn new() -> Self {
        let mut vars = HashMap::new();
        vars.insert("pi".into(), Mat::scalar(std::f64::consts::PI));
        vars.insert("e".into(), Mat::scalar(std::f64::consts::E));
        vars.insert("Inf".into(), Mat::scalar(f64::INFINITY));
        vars.insert("inf".into(), Mat::scalar(f64::INFINITY));
        vars.insert("NaN".into(), Mat::scalar(f64::NAN));
        Self { vars, out: Output::default(), hold: false }
    }

    pub fn set_var(&mut self, name: &str, m: Mat) {
        self.vars.insert(name.into(), m);
    }

    /// Run a script; returns plots and printed text.
    pub fn run(&mut self, src: &str) -> Result<Output, String> {
        let toks = lex(src)?;
        // split into statements at newlines / semicolons / commas outside brackets
        let mut stmts: Vec<(Vec<Tok>, bool)> = Vec::new();
        let (mut cur, mut depth) = (Vec::new(), 0i32);
        for t in toks {
            match &t.t {
                T::Op("(") | T::Op("[") => { depth += 1; cur.push(t) }
                T::Op(")") | T::Op("]") => { depth -= 1; cur.push(t) }
                T::Newline | T::Semi | T::Comma if depth <= 0 => {
                    let silent = t.t == T::Semi;
                    if !cur.is_empty() { stmts.push((std::mem::take(&mut cur), silent)); }
                }
                _ => cur.push(t),
            }
        }
        if !cur.is_empty() { stmts.push((cur, false)); }
        for (n, (s, silent)) in stmts.into_iter().enumerate() {
            self.stmt(s, silent).map_err(|e| format!("statement {}: {e}", n + 1))?;
        }
        Ok(std::mem::take(&mut self.out))
    }

    fn stmt(&mut self, toks: Vec<Tok>, silent: bool) -> Result<(), String> {
        if let Some(Tok { t: T::Id(kw), .. }) = toks.first() {
            if ["function", "for", "while", "if", "switch", "end", "else", "elseif", "end", "try", "catch"].contains(&kw.as_str()) && !self.vars.contains_key(kw) {
                return Err(format!("`{kw}`: control flow and function definitions are not supported by this interpreter"));
            }
        }
        // assignment: id = expr   (but not ==)
        if toks.len() > 2 {
            if let (T::Id(name), T::Op("=")) = (&toks[0].t, &toks[1].t) {
                let name = name.clone();
                let mut p = Parser { t: toks[2..].to_vec(), i: 0, in_matrix: false };
                let e = p.expr()?;
                if p.i != p.t.len() { return Err("unexpected trailing tokens".into()); }
                let v = self.eval(&e, None)?;
                if !silent { self.out.text += &format!("{name} =\n{}\n", fmt_mat(&v)); }
                self.vars.insert(name, v);
                return Ok(());
            }
        }
        // `hold on` command syntax
        if let (Some(Tok { t: T::Id(c), .. }), Some(Tok { t: T::Id(a), .. })) = (toks.first(), toks.get(1)) {
            if c == "hold" { self.hold = a == "on"; return Ok(()); }
            if ["close", "clear", "clc", "figure", "format"].contains(&c.as_str()) { return Ok(()); }
        }
        if let Some(Tok { t: T::Id(c), .. }) = toks.first() {
            if ["close", "clear", "clc", "figure"].contains(&c.as_str()) && toks.len() == 1 {
                return Ok(());
            }
        }
        let mut p = Parser { t: toks, i: 0, in_matrix: false };
        let e = p.expr()?;
        if p.i != p.t.len() { return Err("unexpected trailing tokens".into()); }
        if let Expr::Call(name, args) = &e {
            if self.command(name, args)? { return Ok(()); }
        }
        let v = self.eval(&e, None)?;
        if !silent { self.out.text += &format!("ans =\n{}\n", fmt_mat(&v)); }
        self.vars.insert("ans".into(), v);
        Ok(())
    }

    fn command(&mut self, name: &str, args: &[Expr]) -> Result<bool, String> {
        let text = |s: &mut Self, i: usize| -> Option<String> { match args.get(i) { Some(Expr::Str(t)) => Some(t.clone()), Some(e) => s.eval(e, None).ok().map(|m| fmt_mat(&m)), None => None } };
        match name {
            "plot" => {
                let nums: Vec<&Expr> = args.iter().filter(|a| !matches!(a, Expr::Str(_))).collect();
                if !self.hold { self.out.series.clear(); }
                match nums.as_slice() {
                    [y] => {
                        let y = self.eval(y, None)?;
                        self.out.series.push(Series { x: (1..=y.data.len()).map(|i| i as f64).collect(), y: y.data });
                    }
                    [x, y, ..] => {
                        let (x, y) = (self.eval(x, None)?, self.eval(y, None)?);
                        if x.data.len() != y.data.len() { return Err(format!("plot: x has {} elements but y has {}", x.data.len(), y.data.len())); }
                        self.out.series.push(Series { x: x.data, y: y.data });
                    }
                    [] => return Err("plot needs data".into()),
                }
                Ok(true)
            }
            "title" => { self.out.title = text(self, 0).unwrap_or_default(); Ok(true) }
            "xlabel" => { self.out.xlabel = text(self, 0).unwrap_or_default(); Ok(true) }
            "ylabel" => { self.out.ylabel = text(self, 0).unwrap_or_default(); Ok(true) }
            "legend" | "grid" | "axis" | "xlim" | "ylim" | "figure" | "close" | "clc" | "format" | "subplot" => Ok(true),
            "disp" | "display" => {
                let v = match &args[0] { Expr::Str(s) => s.clone(), e => fmt_mat(&self.eval(e, None)?) };
                self.out.text += &format!("{v}\n");
                Ok(true)
            }
            "fprintf" => {
                let s = text(self, 0).unwrap_or_default();
                let vals: Vec<f64> = args.iter().skip(1).filter_map(|a| self.eval(a, None).ok()).flat_map(|m| m.data).collect();
                let mut it = vals.into_iter();
                let mut res = String::new();
                let cs: Vec<char> = s.chars().collect();
                let mut i = 0;
                while i < cs.len() {
                    if cs[i] == '%' && i + 1 < cs.len() {
                        let mut j = i + 1;
                        while j < cs.len() && !cs[j].is_alphabetic() { j += 1 }
                        let spec: String = cs[i + 1..j.min(cs.len())].iter().collect();
                        let v = it.next().unwrap_or(f64::NAN);
                        let prec = spec.split('.').nth(1).and_then(|p| p.parse::<usize>().ok());
                        res += &match cs.get(j) { Some('d') | Some('i') => format!("{}", v as i64), Some('f') => format!("{:.*}", prec.unwrap_or(6), v), _ => fmt_num(v) };
                        i = j + 1;
                    } else if cs[i] == '\\' && cs.get(i + 1) == Some(&'n') { res.push('\n'); i += 2 } else { res.push(cs[i]); i += 1 }
                }
                self.out.text += &res;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn eval(&self, e: &Expr, end: Option<usize>) -> Result<Mat, String> {
        Ok(match e {
            Expr::Num(n) => Mat::scalar(*n),
            Expr::Str(_) => return Err("strings are only supported as labels".into()),
            Expr::Colon => return Err("unexpected ':'".into()),
            Expr::End => Mat::scalar(end.ok_or("`end` outside an index")? as f64),
            Expr::Var(v) => self.vars.get(v).cloned().ok_or_else(|| format!("undefined variable '{v}'"))?,
            Expr::Neg(x) => self.eval(x, end)?.map(|v| -v),
            Expr::Not(x) => self.eval(x, end)?.map(|v| if v == 0.0 { 1.0 } else { 0.0 }),
            Expr::Transpose(x) => transpose(&self.eval(x, end)?),
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.eval(a, end)?, self.eval(b, end)?);
                match *op {
                    "+" => broadcast(&a, &b, |x, y| x + y)?,
                    "-" => broadcast(&a, &b, |x, y| x - y)?,
                    ".*" => broadcast(&a, &b, |x, y| x * y)?,
                    "./" => broadcast(&a, &b, |x, y| x / y)?,
                    ".^" => broadcast(&a, &b, f64::powf)?,
                    "*" => if a.is_scalar() || b.is_scalar() { broadcast(&a, &b, |x, y| x * y)? } else { matmul(&a, &b)? },
                    "/" => if b.is_scalar() { broadcast(&a, &b, |x, y| x / y)? } else { return Err("matrix right-division is not supported; use ./ for element-wise".into()) },
                    "^" => {
                        if a.is_scalar() && b.is_scalar() { Mat::scalar(a.data[0].powf(b.data[0])) }
                        else if b.is_scalar() && a.rows == a.cols && b.data[0].fract() == 0.0 && b.data[0] >= 0.0 {
                            let mut r = Mat { rows: a.rows, cols: a.cols, data: (0..a.rows * a.cols).map(|i| if i % (a.rows + 1) == 0 { 1.0 } else { 0.0 }).collect() };
                            for _ in 0..b.data[0] as usize { r = matmul(&r, &a)?; }
                            r
                        } else { return Err("^ needs a scalar, or a square matrix and a non-negative integer".into()) }
                    }
                    "==" => broadcast(&a, &b, |x, y| (x == y) as u8 as f64)?,
                    "~=" => broadcast(&a, &b, |x, y| (x != y) as u8 as f64)?,
                    "<" => broadcast(&a, &b, |x, y| (x < y) as u8 as f64)?,
                    ">" => broadcast(&a, &b, |x, y| (x > y) as u8 as f64)?,
                    "<=" => broadcast(&a, &b, |x, y| (x <= y) as u8 as f64)?,
                    ">=" => broadcast(&a, &b, |x, y| (x >= y) as u8 as f64)?,
                    o => return Err(format!("unsupported operator {o}")),
                }
            }
            Expr::Range(a, step, b) => {
                let (a, b) = (self.eval(a, end)?.data[0], self.eval(b, end)?.data[0]);
                let s = match step { Some(s) => self.eval(s, end)?.data[0], None => 1.0 };
                // MATLAB writes a:step:b; the parser stores (a, Some(middle), b) with the middle as the step.
                if s == 0.0 || !(a.is_finite() && b.is_finite()) { return Err("invalid range".into()); }
                let n = ((b - a) / s + 1e-10).floor();
                if n < 0.0 { Mat { rows: 1, cols: 0, data: vec![] } } else if n > 5e6 { return Err("range too large".into()) } else { Mat::row((0..=n as usize).map(|i| a + i as f64 * s).collect()) }
            }
            Expr::Matrix(rows) => {
                let mut out_rows: Vec<Mat> = Vec::new();
                for r in rows {
                    let parts: Vec<Mat> = r.iter().map(|e| self.eval(e, end)).collect::<Result<_, _>>()?;
                    let h = parts.iter().map(|p| p.rows).max().unwrap_or(0);
                    let mut data = Vec::new();
                    let mut cols = 0;
                    for p in parts {
                        if p.rows != h && !p.data.is_empty() { return Err("horizontal concatenation: row counts differ".into()); }
                        cols += p.cols;
                        data.extend(p.data);
                    }
                    out_rows.push(Mat { rows: h, cols, data });
                }
                let cols = out_rows.first().map_or(0, |r| r.cols);
                if out_rows.iter().any(|r| r.cols != cols) { return Err("vertical dimensions mismatch".into()); }
                let rows: usize = out_rows.iter().map(|r| r.rows).sum();
                let mut data = vec![0.0; rows * cols];
                let mut r0 = 0;
                for m in &out_rows {
                    for r in 0..m.rows { for c in 0..cols { data[c * rows + r0 + r] = m.at(r, c); } }
                    r0 += m.rows;
                }
                Mat { rows, cols, data }
            }
            Expr::Call(name, args) => {
                if let Some(v) = self.vars.get(name) {
                    return self.index(v, args);
                }
                self.call(name, args, end)?
            }
        })
    }

    fn index(&self, v: &Mat, args: &[Expr]) -> Result<Mat, String> {
        let pick = |e: &Expr, n: usize| -> Result<Vec<usize>, String> {
            if matches!(e, Expr::Colon) { return Ok((0..n).collect()); }
            let m = self.eval(e, Some(n))?;
            m.data.iter().map(|x| { let i = *x as usize; if *x < 1.0 || i > n { Err(format!("index {x} out of bounds (1..{n})")) } else { Ok(i - 1) } }).collect()
        };
        match args {
            [i] => {
                let idx = pick(i, v.data.len())?;
                let data: Vec<f64> = idx.iter().map(|k| v.data[*k]).collect();
                Ok(if v.cols == 1 && v.rows > 1 { Mat { rows: data.len(), cols: 1, data } } else { Mat::row(data) })
            }
            [r, c] => {
                let (ri, ci) = (pick(r, v.rows)?, pick(c, v.cols)?);
                let mut data = Vec::new();
                for c in &ci { for r in &ri { data.push(v.at(*r, *c)); } }
                Ok(Mat { rows: ri.len(), cols: ci.len(), data })
            }
            _ => Err("only 1-D and 2-D indexing is supported".into()),
        }
    }

    fn call(&self, name: &str, args: &[Expr], end: Option<usize>) -> Result<Mat, String> {
        let a: Vec<Mat> = args.iter().map(|e| self.eval(e, end)).collect::<Result<_, _>>()?;
        let one = |f: fn(f64) -> f64| -> Result<Mat, String> { a.first().map(|m| m.map(f)).ok_or_else(|| format!("{name}: missing argument")) };
        let reduce = |f: &dyn Fn(&[f64]) -> f64| -> Result<Mat, String> {
            let m = a.first().ok_or("missing argument")?;
            if m.rows == 1 || m.cols == 1 { Ok(Mat::scalar(f(&m.data))) } else { Ok(Mat::row((0..m.cols).map(|c| f(&m.data[c * m.rows..(c + 1) * m.rows])).collect())) }
        };
        match name {
            "sin" => one(f64::sin), "cos" => one(f64::cos), "tan" => one(f64::tan), "asin" => one(f64::asin), "acos" => one(f64::acos), "atan" => one(f64::atan),
            "sinh" => one(f64::sinh), "cosh" => one(f64::cosh), "tanh" => one(f64::tanh), "exp" => one(f64::exp), "log" => one(f64::ln), "log10" => one(f64::log10), "log2" => one(f64::log2),
            "sqrt" => one(f64::sqrt), "abs" => one(f64::abs), "floor" => one(f64::floor), "ceil" => one(f64::ceil), "round" => one(f64::round), "sign" => one(f64::signum),
            "atan2" => broadcast(&a[0], &a[1], f64::atan2),
            "mod" => broadcast(&a[0], &a[1], |x, y| x - (x / y).floor() * y),
            "power" => broadcast(&a[0], &a[1], f64::powf),
            "linspace" => {
                let n = a.get(2).map_or(100.0, |m| m.data[0]) as usize;
                let (lo, hi) = (a[0].data[0], a[1].data[0]);
                Ok(Mat::row((0..n).map(|i| if n == 1 { hi } else { lo + (hi - lo) * i as f64 / (n - 1) as f64 }).collect()))
            }
            "zeros" | "ones" => {
                let (r, c) = (a.first().map_or(1.0, |m| m.data[0]) as usize, a.get(1).map_or(a.first().map_or(1.0, |m| m.data[0]), |m| m.data[0]) as usize);
                Ok(Mat { rows: r, cols: c, data: vec![if name == "ones" { 1.0 } else { 0.0 }; r * c] })
            }
            "eye" => { let n = a[0].data[0] as usize; Ok(Mat { rows: n, cols: n, data: (0..n * n).map(|i| if i % (n + 1) == 0 { 1.0 } else { 0.0 }).collect() }) }
            "sum" => reduce(&|v| v.iter().sum()),
            "prod" => reduce(&|v| v.iter().product()),
            "mean" => reduce(&|v| v.iter().sum::<f64>() / v.len() as f64),
            "max" if a.len() == 2 => broadcast(&a[0], &a[1], f64::max),
            "min" if a.len() == 2 => broadcast(&a[0], &a[1], f64::min),
            "max" => reduce(&|v| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max)),
            "min" => reduce(&|v| v.iter().cloned().fold(f64::INFINITY, f64::min)),
            "numel" | "length" => Ok(Mat::scalar(if name == "numel" { a[0].data.len() } else { a[0].rows.max(a[0].cols) } as f64)),
            "size" => Ok(Mat::row(vec![a[0].rows as f64, a[0].cols as f64])),
            "transpose" => Ok(transpose(&a[0])),
            "det" => {
                let m = &a[0];
                if m.rows != m.cols { return Err("det needs a square matrix".into()); }
                let n = m.rows;
                let mut w: Vec<Vec<f64>> = (0..n).map(|r| (0..n).map(|c| m.at(r, c)).collect()).collect();
                let mut det = 1.0;
                for i in 0..n {
                    let p = (i..n).max_by(|x, y| w[*x][i].abs().total_cmp(&w[*y][i].abs())).unwrap();
                    if w[p][i].abs() < 1e-14 { return Ok(Mat::scalar(0.0)); }
                    if p != i { w.swap(p, i); det = -det; }
                    det *= w[i][i];
                    for r in i + 1..n { let f = w[r][i] / w[i][i]; let pivot_row = w[i].clone(); for (c, pv) in pivot_row.iter().enumerate().skip(i) { w[r][c] -= f * pv; } }
                }
                Ok(Mat::scalar(det))
            }
            "cumsum" => { let mut s = 0.0; Ok(a[0].map(|v| { s += v; s })) }
            "diff" => Ok(Mat::row(a[0].data.windows(2).map(|w| w[1] - w[0]).collect())),
            "trapz" => {
                let (x, y) = if a.len() == 2 { (a[0].data.clone(), &a[1].data) } else { ((0..a[0].data.len()).map(|i| i as f64).collect(), &a[0].data) };
                Ok(Mat::scalar(x.windows(2).zip(y.windows(2)).map(|(xw, yw)| (xw[1] - xw[0]) * (yw[0] + yw[1]) / 2.0).sum()))
            }
            _ => Err(format!("undefined function or variable '{name}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> (Interp, Output) {
        let mut i = Interp::new();
        let o = i.run(src).unwrap_or_else(|e| panic!("{e}"));
        (i, o)
    }

    #[test]
    fn typical_calc_script_plots_a_damped_sine() {
        let (i, o) = run("x = linspace(0, 2*pi, 100);\ny = sin(x) .* exp(-x/4);\nplot(x, y); hold on; plot(x, cos(x));\ntitle('Damped'); xlabel('x'); ylabel('y')\n");
        assert_eq!(o.series.len(), 2);
        assert_eq!(o.series[0].x.len(), 100);
        assert!((o.series[0].y[0]).abs() < 1e-12 && (o.series[1].y[0] - 1.0).abs() < 1e-12);
        assert_eq!((o.title.as_str(), o.xlabel.as_str(), o.ylabel.as_str()), ("Damped", "x", "y"));
        assert!((i.vars["y"].data[50] - (std::f64::consts::PI * 2.0 * 50.0 / 99.0).sin() * (-(std::f64::consts::PI * 2.0 * 50.0 / 99.0) / 4.0).exp()).abs() < 1e-12);
    }

    #[test]
    fn plot_replaces_without_hold() {
        let (_, o) = run("plot([1 2 3], [1 4 9]); plot([1 2 3], [2 3 4]);");
        assert_eq!(o.series.len(), 1);
    }

    #[test]
    fn ranges_indexing_matrices_and_transpose() {
        let (i, _) = run("a = 0:0.5:2; b = a(2:3); c = a(end); A = [1 2; 3 4]; B = A*A'; D = A.^2; v = [1 -2 3]; w = [1 - 2]; r = 1:3;");
        assert_eq!(i.vars["a"].data, vec![0.0, 0.5, 1.0, 1.5, 2.0]);
        assert_eq!(i.vars["b"].data, vec![0.5, 1.0]);
        assert_eq!(i.vars["c"].data, vec![2.0]);
        assert_eq!(i.vars["B"], Mat { rows: 2, cols: 2, data: vec![5.0, 11.0, 11.0, 25.0] });
        assert_eq!(i.vars["D"].data, vec![1.0, 9.0, 4.0, 16.0]);
        assert_eq!(i.vars["v"].data, vec![1.0, -2.0, 3.0], "[1 -2 3] is three elements");
        assert_eq!(i.vars["w"].data, vec![-1.0], "[1 - 2] is one element");
        assert_eq!(i.vars["r"].data, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn linear_algebra_and_reductions_and_output() {
        let (i, o) = run("A = [2 1; 1 3]; d = det(A); s = sum([1 2 3 4]); m = mean([2 4 6]); disp(d)\nfprintf('s=%d m=%.1f\\n', s, m);\nx = 5\n");
        assert!((i.vars["d"].data[0] - 5.0).abs() < 1e-12);
        assert!(o.text.contains("5\n") && o.text.contains("s=10 m=4.0") && o.text.contains("x =\n5"), "{}", o.text);
        assert_eq!(run("t = trapz([0 1 2], [0 1 4]);").0.vars["t"].data[0], 3.0);
    }

    #[test]
    fn errors_are_helpful_not_panics() {
        for (src, needle) in [
            ("y = foo(3);", "undefined"),
            ("for i=1:3\nend", "control flow"),
            ("function f()\nend", "control flow"),
            ("a = [1 2 3]; a(7)", "out of bounds"),
            ("x = [1 2] + [1 2 3];", "dimensions"),
            ("c = {1,2};", "unsupported"),
            ("plot([1 2 3], [1 2])", "plot: x has"),
            ("y = (1 +", "unexpected"),
        ] {
            let e = Interp::new().run(src).unwrap_err();
            assert!(e.to_lowercase().contains(needle), "{src}: {e}");
        }
    }

    #[test]
    fn mat_file_loading_reports_garbage_without_panicking() {
        assert!(load_mat(b"definitely not a mat file").is_err());
        assert!(load_mat(&[]).is_err());
    }

    /// Build a minimal little-endian MAT v5 file with one double matrix (column-major data).
    fn mat5(name: &str, rows: u32, cols: u32, data: &[f64]) -> Vec<u8> {
        fn el(ty: u32, payload: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend(ty.to_le_bytes());
            v.extend((payload.len() as u32).to_le_bytes());
            v.extend(payload);
            while v.len() % 8 != 0 {
                v.push(0);
            }
            v
        }
        let mut body = Vec::new();
        body.extend(el(6, &[6, 0, 0, 0, 0, 0, 0, 0])); // array flags: class mxDOUBLE
        body.extend(el(5, &[rows.to_le_bytes(), cols.to_le_bytes()].concat()));
        body.extend(el(1, name.as_bytes()));
        body.extend(el(9, &data.iter().flat_map(|d| d.to_le_bytes()).collect::<Vec<u8>>()));
        let mut f = vec![b' '; 116];
        f[..20].copy_from_slice(b"MATLAB 5.0 MAT-file,");
        f.extend([0u8; 8]);
        f.extend(0x0100u16.to_le_bytes());
        f.extend(b"IM");
        f.extend(el(14, &body));
        f
    }

    #[test]
    fn loads_a_real_mat_v5_file_and_feeds_the_interpreter() {
        let bytes = mat5("signal", 2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let vars = load_mat(&bytes).unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "signal");
        assert_eq!((vars[0].value.rows, vars[0].value.cols), (2, 3));
        assert_eq!(vars[0].value.at(1, 2), 6.0, "column-major layout preserved");
        let mut i = Interp::new();
        i.set_var("signal", vars[0].value.clone());
        i.run("s = sum(signal(1, :)); plot(signal(2, :));").unwrap();
        assert_eq!(i.vars["s"].data[0], 9.0);
    }
}
