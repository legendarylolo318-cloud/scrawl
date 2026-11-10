//! Cursor-based structure editor over the expression tree. Typed keys, shorthand,
//! autocomplete picks and recognized drawn symbols all edit the same tree at the same cursor.

use crate::shorthand::{Out, Suggestion, Table, UserShorthand};
use layout::{AlignLine, BigOp, Expr, MatrixKind};

type Path = Vec<(usize, usize)>;

fn slots_mut(e: &mut Expr) -> Vec<&mut Expr> {
    match e {
        Expr::Frac(a, b) => vec![&mut **a, &mut **b],
        Expr::Sqrt { index, body } => {
            let mut v = Vec::new();
            if let Some(i) = index {
                v.push(&mut **i);
            }
            v.push(&mut **body);
            v
        }
        Expr::Sup(_, x) | Expr::Sub(_, x) => vec![&mut **x],
        Expr::SubSup(_, lo, hi) => vec![&mut **lo, &mut **hi],
        Expr::Big { lower, upper, body, .. } => {
            let mut v = Vec::new();
            if let Some(l) = lower {
                v.push(&mut **l);
            }
            if let Some(u) = upper {
                v.push(&mut **u);
            }
            v.push(&mut **body);
            v
        }
        Expr::Matrix { rows, .. } => rows.iter_mut().flatten().collect(),
        Expr::Accent(_, b) | Expr::Fence(_, b) => vec![&mut **b],
        Expr::Cases(rows) => rows.iter_mut().flat_map(|(a, b)| [a, b]).collect(),
        Expr::Deriv { var, body, .. } => vec![&mut **var, &mut **body],
        Expr::EvalBar { body, lower, upper } => {
            let mut v = vec![&mut **body];
            if let Some(l) = lower {
                v.push(&mut **l);
            }
            if let Some(u) = upper {
                v.push(&mut **u);
            }
            v
        }
        _ => vec![],
    }
}

fn slot_count(e: &Expr) -> usize {
    slots_mut(&mut e.clone()).len()
}

fn row_mut<'a>(row: &'a mut Vec<Expr>, path: &[(usize, usize)]) -> &'a mut Vec<Expr> {
    match path.split_first() {
        None => row,
        Some((&(i, s), rest)) => {
            let item = &mut row[i];
            let slot = slots_mut(item).into_iter().nth(s).expect("valid slot path");
            match slot {
                Expr::Row(v) => row_mut(v, rest),
                _ => unreachable!("slots are always rows"),
            }
        }
    }
}

/// Make every slot a `Row` so the cursor can enter it.
pub fn normalize(e: &mut Expr) {
    for s in slots_mut(e) {
        if !matches!(s, Expr::Row(_)) {
            let inner = std::mem::replace(s, Expr::empty());
            *s = Expr::Row(vec![inner]);
        }
        if let Expr::Row(v) = s {
            v.iter_mut().for_each(normalize);
        }
    }
    if let Expr::Sup(b, _) | Expr::Sub(b, _) | Expr::SubSup(b, ..) = e {
        normalize(b);
    }
    if let Expr::Row(v) = e {
        v.iter_mut().for_each(normalize);
    }
}

/// Drop empty optional parts and unwrap single-item rows, producing a tidy tree for output.
pub fn clean(e: &Expr) -> Expr {
    fn opt(o: &Option<Box<Expr>>) -> Option<Box<Expr>> {
        o.as_ref().map(|b| clean(b)).filter(|c| !c.is_empty()).map(Box::new)
    }
    let b = |x: &Expr| Box::new(clean(x));
    match e {
        Expr::Row(v) => Expr::row(v.iter().map(clean).collect()),
        Expr::Frac(a, c) => Expr::Frac(b(a), b(c)),
        Expr::Sqrt { index, body } => Expr::Sqrt { index: opt(index), body: b(body) },
        Expr::Sup(base, x) => Expr::Sup(b(base), b(x)),
        Expr::Sub(base, x) => Expr::Sub(b(base), b(x)),
        Expr::SubSup(base, l, h) => Expr::SubSup(b(base), b(l), b(h)),
        Expr::Big { op, lower, upper, body } => Expr::Big { op: *op, lower: opt(lower), upper: opt(upper), body: b(body) },
        Expr::Matrix { kind, rows, augment } => Expr::Matrix {
            kind: *kind,
            rows: rows.iter().map(|r| r.iter().map(clean).collect()).collect(),
            augment: *augment,
        },
        Expr::Accent(a, x) => Expr::Accent(*a, b(x)),
        Expr::Fence(f, x) => Expr::Fence(*f, b(x)),
        Expr::Cases(rows) => Expr::Cases(rows.iter().map(|(v, c)| (clean(v), clean(c))).collect()),
        Expr::Deriv { partial, order, var, body } => Expr::Deriv { partial: *partial, order: opt(order), var: b(var), body: b(body) },
        Expr::EvalBar { body, lower, upper } => Expr::EvalBar { body: b(body), lower: opt(lower), upper: opt(upper) },
        other => other.clone(),
    }
}

pub struct Editor {
    root: Vec<Expr>,
    path: Path,
    pos: usize,
    /// Alphanumeric run typed since the last delimiter (starts with a letter).
    word: String,
    /// Raw ASCII punctuation typed contiguously (e.g. `<=`), waiting to be combined.
    ops: String,
    table: Table,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(&UserShorthand::default())
    }
}

impl Editor {
    pub fn new(user: &UserShorthand) -> Self {
        Self { root: vec![], path: vec![], pos: 0, word: String::new(), ops: String::new(), table: Table::new(user) }
    }

    pub fn set_shorthand(&mut self, user: &UserShorthand) {
        self.table = Table::new(user);
    }

    /// The expression as currently typed (tidied).
    pub fn expr(&self) -> Expr {
        clean(&Expr::Row(self.root.clone()))
    }

    /// Load an existing expression; cursor goes to the end.
    pub fn set_expr(&mut self, e: Expr) {
        let mut e = e;
        normalize(&mut e);
        self.root = match e {
            Expr::Row(v) => v,
            other => vec![other],
        };
        self.path.clear();
        self.pos = self.root.len();
        self.word.clear();
    }

    pub fn pending_word(&self) -> &str {
        &self.word
    }

    pub fn depth(&self) -> usize {
        self.path.len()
    }

    fn row(&mut self) -> &mut Vec<Expr> {
        let path = self.path.clone();
        row_mut(&mut self.root, &path)
    }

    fn put(&mut self, e: Expr) {
        let pos = self.pos;
        self.row().insert(pos, e);
        self.pos += 1;
    }

    /// Insert a composite at the cursor and move into slot `slot`.
    fn put_composite(&mut self, mut e: Expr, slot: usize) {
        normalize(&mut e);
        let pos = self.pos;
        self.row().insert(pos, e);
        self.path.push((pos, slot));
        self.pos = 0;
    }

    fn remove_tail_chars(&mut self, n: usize) {
        let mut left = n;
        while left > 0 && self.pos > 0 {
            let pos = self.pos;
            let row = self.row();
            match &mut row[pos - 1] {
                Expr::Num(s) => {
                    let take = left.min(s.chars().count());
                    for _ in 0..take {
                        s.pop();
                    }
                    left -= take;
                    if s.is_empty() {
                        row.remove(pos - 1);
                        self.pos -= 1;
                    }
                }
                _ => {
                    row.remove(pos - 1);
                    self.pos -= 1;
                    left -= 1;
                }
            }
        }
    }

    // ---- shorthand -------------------------------------------------------------------

    fn expand(&mut self, out: Out, replaced_chars: usize) {
        self.remove_tail_chars(replaced_chars);
        self.word.clear();
        match out {
            Out::Sym(s) => self.put(Expr::Sym(s)),
            Out::Func(f) => self.put(Expr::Func(f)),
            // Big operators: cursor stays after; `_`/`^` then enter the bound slots.
            Out::Big(op) => {
                let mut e = Expr::Big { op, lower: Some(Box::new(Expr::empty())), upper: Some(Box::new(Expr::empty())), body: Box::new(Expr::empty()) };
                normalize(&mut e);
                self.put(e);
            }
            Out::Sqrt => self.put_composite(Expr::Sqrt { index: None, body: Box::new(Expr::empty()) }, 0),
            Out::Accent(a) => self.put_composite(Expr::Accent(a, Box::new(Expr::empty())), 0),
            Out::Fence(f) => self.put_composite(Expr::Fence(f, Box::new(Expr::empty())), 0),
            Out::Frac => self.put_composite(Expr::Frac(Box::new(Expr::empty()), Box::new(Expr::empty())), 0),
            Out::Matrix { kind, rows, cols } => {
                let m = Expr::Matrix { kind, rows: vec![vec![Expr::empty(); cols]; rows], augment: None };
                self.put_composite(m, 0)
            }
            Out::Cases => self.put_composite(Expr::Cases(vec![(Expr::empty(), Expr::empty()), (Expr::empty(), Expr::empty())]), 0),
            Out::Deriv { partial } => {
                self.put_composite(Expr::Deriv { partial, order: None, var: Box::new(Expr::empty()), body: Box::new(Expr::empty()) }, 0)
            }
            Out::EvalBar => {
                let e = Expr::EvalBar { body: Box::new(Expr::empty()), lower: Some(Box::new(Expr::empty())), upper: Some(Box::new(Expr::empty())) };
                self.put_composite(e, 0)
            }
        }
    }

    /// Convert the pending word if it is an exact shorthand. `force` = a delimiter was typed.
    fn try_word(&mut self, force: bool) -> bool {
        if self.word.is_empty() {
            return false;
        }
        let w = self.word.clone();
        if let Some(out) = self.table.get(&w) {
            if force || !self.table.is_strict_prefix(&w) {
                self.expand(out, w.chars().count());
                return true;
            }
        }
        false
    }

    /// Apply an autocomplete suggestion in place of the pending word.
    pub fn accept(&mut self, s: &Suggestion) {
        let n = self.word.chars().count();
        self.expand(s.out.clone(), n);
    }

    pub fn suggestions(&self, limit: usize) -> Vec<Suggestion> {
        self.table.suggest(&self.word, limit)
    }

    // ---- typing ----------------------------------------------------------------------

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert_char(c);
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let is_op = !c.is_alphanumeric() && !c.is_whitespace() && !matches!(c, '^' | '_' | '/' | '\'' | '.');
        if !is_op {
            self.flush_ops();
        }
        match c {
            c if c.is_alphabetic() => {
                self.put(Expr::Sym(c.to_string()));
                self.word.push(c);
                self.try_word(false);
            }
            c if c.is_ascii_digit() => {
                let pos = self.pos;
                let prev_is_num = pos > 0 && matches!(self.row().get(pos - 1), Some(Expr::Num(_)));
                if prev_is_num {
                    if let Expr::Num(n) = &mut self.row()[pos - 1] {
                        n.push(c);
                    }
                } else {
                    self.put(Expr::Num(c.to_string()));
                }
                if !self.word.is_empty() {
                    self.word.push(c); // e.g. mat3x3
                    self.try_word(false);
                }
            }
            '^' => {
                self.try_word(true);
                self.script(true);
            }
            '_' => {
                self.try_word(true);
                self.script(false);
            }
            '/' => {
                self.try_word(true);
                self.fraction();
            }
            ' ' => {
                self.try_word(true);
                self.word.clear();
            }
            '.' if self.word.is_empty() => {
                let pos = self.pos;
                if pos > 0 {
                    if let Some(Expr::Num(n)) = self.row().get_mut(pos - 1) {
                        if !n.contains('.') {
                            n.push('.');
                            return;
                        }
                    }
                }
                self.put(Expr::sym("."));
            }
            '\'' => {
                self.try_word(true);
                self.word.clear();
                let pos = self.pos;
                let prev_prime = pos > 0 && matches!(self.row().get(pos - 1), Some(Expr::Sym(s)) if s == "′");
                if prev_prime {
                    self.row()[pos - 1] = Expr::sym("″");
                } else {
                    self.put(Expr::sym("′"));
                }
            }
            other => {
                self.try_word(true);
                self.word.clear();
                self.put(Expr::Sym(other.to_string()));
                self.ops.push(other);
                self.combine_operator_tail(false);
            }
        }
    }

    fn flush_ops(&mut self) {
        if !self.ops.is_empty() {
            self.combine_operator_tail(true);
            self.ops.clear();
        }
    }

    /// Merge typed ASCII operator runs like `-` `>` into `→`. While the run could still grow
    /// into a longer shorthand (`<=` -> `<=>`), wait unless `force`.
    fn combine_operator_tail(&mut self, force: bool) {
        let run: Vec<char> = self.ops.chars().collect();
        for len in [3usize, 2] {
            if run.len() < len || self.pos < len {
                continue;
            }
            let tail: String = run[run.len() - len..].iter().collect();
            if let Some(Out::Sym(s)) = self.table.get(&tail) {
                if force || !self.table.is_strict_prefix(&tail) {
                    let pos = self.pos;
                    self.row().drain(pos - len..pos);
                    self.pos -= len;
                    self.put(Expr::Sym(s));
                    self.ops.clear();
                    return;
                }
            }
        }
    }

    /// Insert a complete expression (e.g. from recognized ink) at the cursor.
    pub fn insert_expr(&mut self, e: Expr) {
        self.flush_ops();
        self.try_word(true);
        self.word.clear();
        let mut e = e;
        normalize(&mut e);
        match e {
            Expr::Row(items) => {
                for i in items {
                    self.put(i);
                }
            }
            other => self.put(other),
        }
    }

    fn take_prev_atom(&mut self) -> Expr {
        if self.pos == 0 {
            return Expr::empty();
        }
        let pos = self.pos;
        let e = self.row().remove(pos - 1);
        self.pos -= 1;
        e
    }

    fn script(&mut self, sup: bool) {
        self.word.clear();
        // Inside a script/bound slot, `^`/`_` at the slot end addresses the *enclosing* template
        // (x_i^2 means a sub and a sup on x; sum_i=0^n moves from the lower to the upper bound).
        if let Some(&(i, s)) = self.path.last() {
            let parent: Path = self.path[..self.path.len() - 1].to_vec();
            let at_end = self.pos == self.row().len();
            let kind = {
                let item = &row_mut(&mut self.root, &parent)[i];
                match item {
                    Expr::Sub(..) if sup => 1,
                    Expr::Sup(..) if !sup => 1,
                    Expr::Big { lower: Some(_), upper: Some(_), .. } if sup && s == 0 => 2,
                    Expr::Big { lower: Some(_), upper: Some(_), .. } if !sup && s == 1 => 3,
                    _ => 0,
                }
            };
            if at_end && kind == 1 {
                self.path.pop();
                self.pos = i + 1;
            } else if at_end && (kind == 2 || kind == 3) {
                *self.path.last_mut().unwrap() = (i, if kind == 2 { 1 } else { 0 });
                self.pos = 0;
                return;
            }
        }
        // `_`/`^` right after a big operator edits its bounds.
        let pos = self.pos;
        if pos > 0 {
            if let Some(Expr::Big { lower, upper, .. }) = self.row().get(pos - 1) {
                let (has_lo, has_hi) = (lower.is_some(), upper.is_some());
                let slot = if sup { has_lo as usize } else { 0 };
                if (sup && has_hi) || (!sup && has_lo) {
                    self.path.push((pos - 1, slot));
                    self.pos = 0;
                    return;
                }
            }
        }
        let base = self.take_prev_atom();
        let pos = self.pos;
        let b = Box::new(base);
        let e = match (sup, *b) {
            (true, Expr::Sub(inner, lo)) => (Expr::SubSup(inner, lo, Box::new(Expr::empty())), 1),
            (false, Expr::Sup(inner, hi)) => (Expr::SubSup(inner, Box::new(Expr::empty()), hi), 0),
            (true, other) => (Expr::Sup(Box::new(other), Box::new(Expr::empty())), 0),
            (false, other) => (Expr::Sub(Box::new(other), Box::new(Expr::empty())), 0),
        };
        let (mut node, slot) = e;
        normalize(&mut node);
        self.row().insert(pos, node);
        self.path.push((pos, slot));
        self.pos = 0;
    }

    fn fraction(&mut self) {
        self.word.clear();
        let num = self.take_prev_atom();
        let pos = self.pos;
        let mut node = Expr::Frac(Box::new(num), Box::new(Expr::empty()));
        normalize(&mut node);
        self.row().insert(pos, node);
        self.path.push((pos, 1));
        self.pos = 0;
    }

    // ---- navigation ------------------------------------------------------------------

    /// Tab: next template slot; leaves the template after the last one. False at top level.
    pub fn tab(&mut self) -> bool {
        self.flush_ops();
        self.try_word(true);
        self.word.clear();
        let Some(&(i, s)) = self.path.last() else { return false };
        let parent_path: Path = self.path[..self.path.len() - 1].to_vec();
        let n = slot_count(&row_mut(&mut self.root, &parent_path)[i]);
        if s + 1 < n {
            *self.path.last_mut().unwrap() = (i, s + 1);
            self.pos = 0;
        } else {
            self.path.pop();
            self.pos = i + 1;
        }
        true
    }

    pub fn shift_tab(&mut self) -> bool {
        self.word.clear();
        let Some(&(i, s)) = self.path.last() else { return false };
        if s > 0 {
            *self.path.last_mut().unwrap() = (i, s - 1);
            self.pos = self.row().len();
        } else {
            self.path.pop();
            self.pos = i;
        }
        true
    }

    pub fn right(&mut self) {
        self.flush_ops();
        self.try_word(true);
        self.word.clear();
        let pos = self.pos;
        if pos < self.row().len() {
            if slot_count(&self.row()[pos]) > 0 {
                self.path.push((pos, 0));
                self.pos = 0;
            } else {
                self.pos += 1;
            }
        } else {
            self.tab();
        }
    }

    pub fn left(&mut self) {
        self.word.clear();
        if self.pos > 0 {
            let pos = self.pos;
            let n = slot_count(&self.row()[pos - 1]);
            if n > 0 {
                self.path.push((pos - 1, n - 1));
                self.pos = self.row().len();
            } else {
                self.pos -= 1;
            }
        } else {
            self.shift_tab();
        }
    }

    pub fn backspace(&mut self) {
        if self.pos == 0 {
            // Backspace in the first position of an otherwise empty template deletes the template.
            if let Some(&(i, _)) = self.path.last() {
                let parent: Path = self.path[..self.path.len() - 1].to_vec();
                let mut item = row_mut(&mut self.root, &parent)[i].clone();
                if slots_mut(&mut item).iter().all(|s| matches!(s, Expr::Row(v) if v.is_empty())) {
                    row_mut(&mut self.root, &parent).remove(i);
                    self.path.pop();
                    self.pos = i;
                    return;
                }
            }
            self.left();
            return;
        }
        let pos = self.pos;
        let n = slot_count(&self.row()[pos - 1]);
        if n > 0 {
            let mut e = self.row()[pos - 1].clone();
            let all_empty = slots_mut(&mut e).iter().all(|s| matches!(s, Expr::Row(v) if v.is_empty()));
            if all_empty {
                self.row().remove(pos - 1);
                self.pos -= 1;
            } else {
                self.left();
            }
            return;
        }
        let row = self.row();
        match &mut row[pos - 1] {
            Expr::Num(s) if s.chars().count() > 1 => {
                s.pop();
            }
            _ => {
                row.remove(pos - 1);
                self.pos -= 1;
            }
        }
        self.word.pop();
    }

    // ---- structured inserts from toolbar / palette -------------------------------------

    pub fn insert_template(&mut self, out: Out) {
        self.try_word(true);
        self.expand(out, 0);
    }

    pub fn insert_matrix(&mut self, kind: MatrixKind, rows: usize, cols: usize, augment: Option<usize>) {
        self.try_word(true);
        let mut m = Expr::Matrix { kind, rows: vec![vec![Expr::empty(); cols.clamp(1, 6)]; rows.clamp(1, 6)], augment };
        normalize(&mut m);
        self.put_composite(m, 0);
    }

    pub fn is_inside_template(&self) -> bool {
        !self.path.is_empty()
    }

    /// Big operator helper for tests / palette: insert and leave cursor after it.
    pub fn insert_big(&mut self, op: BigOp) {
        self.expand(Out::Big(op), 0);
    }
}

/// Multi-line derivation block: each line is an editor-produced expression.
pub fn derivation(lines: Vec<(Expr, String, Expr, Option<String>)>) -> Expr {
    Expr::Align(lines.into_iter().map(|(lhs, rel, rhs, note)| AlignLine { lhs, rel, rhs, note }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typst(s: &str) -> String {
        let mut e = Editor::default();
        e.insert_str(s);
        e.try_word(true);
        e.expr().to_typst()
    }

    #[test]
    fn greek_and_operators() {
        assert_eq!(typst("alpha + beta"), "α + β");
        assert_eq!(typst("x -> y"), "x → y");
        assert_eq!(typst("a != b"), "a ≠ b");
        assert_eq!(typst("a <=> b"), "a ⇔ b");
        assert_eq!(typst("a +- b"), "a ± b");
        assert_eq!(typst("a <= b >= c"), "a ≤ b ≥ c");
    }

    #[test]
    fn ambiguous_words_wait_for_delimiter() {
        assert_eq!(typst("x in RR"), "x ∈ RR", "Typst spells ℝ as RR");
        assert_eq!(typst("x in RR").replace("RR", "ℝ"), "x ∈ ℝ");
        assert_eq!(typst("inf"), "infinity", "∞ in Typst");
        // never touches ordinary multi-letter runs
        assert_eq!(typst("axin"), "a x i n");
        assert_eq!(typst("xi"), "ξ", "a complete shorthand converts as soon as it is unambiguous");
    }

    #[test]
    fn scripts_and_fractions_navigate_with_tab() {
        let mut e = Editor::default();
        e.insert_str("x^2");
        assert!(e.tab());
        e.insert_str("+1");
        assert_eq!(e.expr().to_typst(), "x^2 + 1");
        let mut e = Editor::default();
        e.insert_str("a/b");
        assert_eq!(e.expr().to_latex(), "\\frac{a}{b}");
        let mut e = Editor::default();
        e.insert_str("x_i^2");
        assert_eq!(e.expr().to_latex(), "x_{i}^{2}");
    }

    #[test]
    fn sqrt_vec_and_matrix_templates() {
        let mut e = Editor::default();
        e.insert_str("sqrt ");
        e.insert_str("x+1");
        assert!(e.tab());
        e.insert_str("= 0");
        assert_eq!(e.expr().to_typst(), "sqrt(x + 1) = 0");
        let mut e = Editor::default();
        e.insert_str("mat2x2");
        for (i, c) in ["1", "2", "3", "4"].iter().enumerate() {
            e.insert_str(c);
            if i < 3 {
                e.tab();
            }
        }
        assert_eq!(e.expr().to_typst(), "mat(1, 2; 3, 4)");
        let mut e = Editor::default();
        e.insert_str("vec ");
        e.insert_str("v");
        assert_eq!(e.expr().to_typst(), "arrow(v)");
    }

    #[test]
    fn big_operators_with_bounds() {
        let mut e = Editor::default();
        e.insert_str("sum_i=0^n");
        e.tab(); // upper -> body
        e.tab(); // body -> out
        e.insert_str("a_i");
        assert_eq!(e.expr().to_typst(), "sum_(i = 0)^n a_i");
        let mut e = Editor::default();
        e.insert_str("int_0^1");
        e.tab();
        e.tab();
        e.insert_str("x dx");
        assert_eq!(e.expr().to_latex(), "\\int_{0}^{1} x d x");
    }

    #[test]
    fn drawn_and_typed_input_mix_at_one_cursor() {
        let mut e = Editor::default();
        e.insert_str("y=");
        // pretend ink recognition produced "x^2"
        e.insert_expr(Expr::Sup(Box::new(Expr::sym("x")), Box::new(Expr::num("2"))));
        e.insert_str("+1");
        assert_eq!(e.expr().to_typst(), "y = x^2 + 1");
    }

    #[test]
    fn backspace_and_primes_and_numbers() {
        let mut e = Editor::default();
        e.insert_str("f'");
        e.insert_char('\'');
        assert_eq!(e.expr().to_typst(), "f ''");
        let mut e = Editor::default();
        e.insert_str("12.5");
        assert_eq!(e.expr().to_typst(), "12.5");
        e.backspace();
        assert_eq!(e.expr().to_typst(), "12.");
        let mut e = Editor::default();
        e.insert_str("a/");
        e.backspace(); // empty denominator: step back into the numerator
        e.backspace(); // delete `a`
        e.backspace(); // template is now empty: delete it
        assert!(e.expr().is_empty());
        assert_eq!(e.depth(), 0);
    }

    #[test]
    fn autocomplete_accepts_into_tree() {
        let mut e = Editor::default();
        e.insert_str("sub");
        let s = e.suggestions(5);
        assert!(s.iter().any(|x| x.name == "subset"));
        let pick = s.into_iter().find(|x| x.name == "subset").unwrap();
        e.accept(&pick);
        assert_eq!(e.expr().to_typst(), "⊂");
    }
}
