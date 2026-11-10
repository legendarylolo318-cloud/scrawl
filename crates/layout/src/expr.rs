//! The expression tree. Drawn input (via spatial layout) and typed input both produce this.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BigOp {
    Sum,
    Prod,
    Int,
    IInt,
    IIInt,
    OInt,
    OIInt,
    Lim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    Arrow,
    Hat,
    Bold,
    Bar,
    Tilde,
    Dot,
    DDot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatrixKind {
    Paren,
    Bracket,
    Det,
    Norm,
    Brace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fence {
    Paren,
    Bracket,
    Brace,
    Abs,
    Norm,
    Angle,
    Floor,
    Ceil,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// Horizontal sequence.
    Row(Vec<Expr>),
    /// Single symbol: a letter, Greek letter, operator or relation, as unicode.
    Sym(String),
    /// Digits, possibly with a decimal point.
    Num(String),
    /// Upright function name: sin, det, rank, ...
    Func(String),
    /// Upright unit: kg, eV, ...
    Unit(String),
    /// Plain text (annotations etc.).
    Text(String),
    Frac(Box<Expr>, Box<Expr>),
    Sqrt { index: Option<Box<Expr>>, body: Box<Expr> },
    Sup(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    SubSup(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Sum, product, integral, limit with optional bounds and body.
    Big { op: BigOp, lower: Option<Box<Expr>>, upper: Option<Box<Expr>>, body: Box<Expr> },
    Matrix { kind: MatrixKind, rows: Vec<Vec<Expr>>, augment: Option<usize> },
    Accent(Accent, Box<Expr>),
    Fence(Fence, Box<Expr>),
    /// Piecewise: (value, condition) per line.
    Cases(Vec<(Expr, Expr)>),
    /// d/dx or ∂/∂x applied to `body`.
    Deriv { partial: bool, order: Option<Box<Expr>>, var: Box<Expr>, body: Box<Expr> },
    /// Evaluation bar with optional bounds: f(x)|_a^b.
    EvalBar { body: Box<Expr>, lower: Option<Box<Expr>>, upper: Option<Box<Expr>> },
    /// Source we could not parse; kept verbatim and still rendered.
    Raw(String),
    /// Multi-line derivation aligned at relation signs.
    Align(Vec<AlignLine>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlignLine {
    /// Content before the aligned relation (may be empty on continuation lines).
    pub lhs: Expr,
    /// The relation, e.g. "=".
    pub rel: String,
    pub rhs: Expr,
    pub note: Option<String>,
}

impl Expr {
    pub fn sym(s: &str) -> Expr {
        Expr::Sym(s.to_string())
    }
    pub fn num(s: &str) -> Expr {
        Expr::Num(s.to_string())
    }
    pub fn row(v: Vec<Expr>) -> Expr {
        if v.len() == 1 {
            v.into_iter().next().unwrap()
        } else {
            Expr::Row(v)
        }
    }
    pub fn empty() -> Expr {
        Expr::Row(vec![])
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Expr::Row(v) if v.is_empty())
    }
}
