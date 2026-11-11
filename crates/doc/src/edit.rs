//! Editing operations on a `Doc` (word-processor rules). Pure functions: no layout, no UI.
//! Positions are `Pos { para, off }`; offsets count characters, an inline math object counts as 1.

use crate::model::*;

fn run_slice(r: &Run, from: usize, to: usize) -> Option<Run> {
    match r {
        Run::Text { text, style, link } => {
            let s: String = text.chars().skip(from).take(to.saturating_sub(from)).collect();
            (!s.is_empty()).then(|| Run::Text { text: s, style: *style, link: link.clone() })
        }
        Run::Math { .. } => (from == 0 && to >= 1).then(|| r.clone()),
    }
}

/// Runs covering offsets `a..b` of `runs`.
pub fn slice_runs(runs: &[Run], a: usize, b: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let mut at = 0;
    for r in runs {
        let len = run_len(r);
        let (lo, hi) = (a.max(at), b.min(at + len));
        if lo < hi {
            out.extend(run_slice(r, lo - at, hi - at));
        }
        at += len;
    }
    out
}

fn total(runs: &[Run]) -> usize {
    runs.iter().map(run_len).sum()
}

fn chars_of(p: &Para) -> Vec<char> {
    let mut v = Vec::new();
    for r in &p.runs {
        match r {
            Run::Text { text, .. } => v.extend(text.chars()),
            Run::Math { .. } => v.push('\u{FFFC}'),
        }
    }
    v
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\u{FFFC}'
}

fn len_of(doc: &Doc, para: usize) -> usize {
    doc.paras.get(para).map_or(0, |p| if p.kind.is_text() { total(&p.runs) } else { 1 })
}

pub fn clamp(doc: &Doc, p: Pos) -> Pos {
    if doc.paras.is_empty() {
        return Pos::default();
    }
    let para = p.para.min(doc.paras.len() - 1);
    Pos { para, off: p.off.min(len_of(doc, para)) }
}

pub fn ensure_not_empty(doc: &mut Doc) {
    if doc.paras.is_empty() {
        doc.push(ParaKind::Body, vec![]);
    }
}

fn new_para(doc: &mut Doc, kind: ParaKind, runs: Vec<Run>) -> Para {
    let id = doc.fresh_id();
    Para { id, kind, runs: normalize(runs) }
}

/// Style of the character before the caret (or after it at the start of a paragraph).
pub fn style_at(doc: &Doc, pos: Pos) -> Style {
    let Some(p) = doc.paras.get(pos.para) else { return Style::default() };
    let find = |at: usize| {
        let mut i = 0;
        for r in &p.runs {
            let len = run_len(r);
            if at >= i && at < i + len {
                return Some(match r {
                    Run::Text { style, .. } => *style,
                    Run::Math { .. } => Style::default(),
                });
            }
            i += len;
        }
        None
    };
    (if pos.off > 0 { find(pos.off - 1) } else { find(0) }).unwrap_or_default()
}

/// Remove everything between `a` and `b`; returns the caret position.
pub fn delete_range(doc: &mut Doc, a: Pos, b: Pos) -> Pos {
    let (s, e) = if a <= b { (clamp(doc, a), clamp(doc, b)) } else { (clamp(doc, b), clamp(doc, a)) };
    if s == e || doc.paras.is_empty() {
        return s;
    }
    if s.para == e.para {
        let p = &mut doc.paras[s.para];
        if p.kind.is_text() {
            let mut runs = slice_runs(&p.runs, 0, s.off);
            runs.extend(slice_runs(&p.runs, e.off, usize::MAX));
            p.runs = normalize(runs);
            return s;
        }
        doc.paras.remove(s.para);
        ensure_not_empty(doc);
        return clamp(doc, Pos { para: s.para, off: 0 });
    }
    let first_text = doc.paras[s.para].kind.is_text();
    let last_text = doc.paras[e.para].kind.is_text();
    let head: Vec<Run> = if first_text { slice_runs(&doc.paras[s.para].runs, 0, s.off) } else { vec![] };
    let tail: Vec<Run> = if last_text { slice_runs(&doc.paras[e.para].runs, e.off, usize::MAX) } else { vec![] };
    let keep_first = first_text || s.off >= 1;
    let keep_last = last_text || e.off == 0;
    let mut caret = Pos { para: s.para, off: s.off };
    if first_text && last_text {
        let at = total(&head);
        let mut runs = head;
        runs.extend(tail);
        doc.paras[s.para].runs = normalize(runs);
        doc.paras.drain(s.para + 1..=e.para);
        caret.off = at;
    } else {
        // at least one end is an object: keep what survives as separate paragraphs
        if keep_last && last_text {
            doc.paras[e.para].runs = normalize(tail);
        }
        let from = if keep_first { s.para + 1 } else { s.para };
        let to = if keep_last { e.para } else { e.para + 1 };
        if from < to {
            doc.paras.drain(from..to);
        }
        if first_text && keep_first {
            doc.paras[s.para].runs = normalize(head);
        }
        caret = if keep_first { Pos { para: s.para, off: if first_text { s.off } else { 1 } } } else { Pos { para: s.para, off: 0 } };
    }
    ensure_not_empty(doc);
    clamp(doc, caret)
}

/// Insert `text` (may contain newlines = new paragraphs) replacing the selection.
pub fn insert_text(doc: &mut Doc, sel: Sel, text: &str, style: Style) -> Pos {
    let (s, e) = sel.ordered();
    // typing over a selected object puts the text where the object was
    if s.para == e.para && s.off == 0 && e.off == 1 && doc.paras.get(s.para).is_some_and(|p| !p.kind.is_text()) {
        let np = new_para(doc, ParaKind::Body, vec![]);
        doc.paras[s.para] = np;
        return insert_text(doc, Sel::caret(Pos { para: s.para, off: 0 }), text, style);
    }
    let mut pos = delete_range(doc, s, e);
    ensure_not_empty(doc);
    let mut first = true;
    for seg in text.split('\n') {
        if !first {
            pos = enter(doc, pos);
        }
        first = false;
        let seg = seg.trim_end_matches('\r');
        if !seg.is_empty() {
            pos = insert_inline(doc, pos, Run::Text { text: seg.into(), style, link: None });
        }
    }
    pos
}

/// Insert one inline run (text or math) at `pos`.
pub fn insert_inline(doc: &mut Doc, pos: Pos, run: Run) -> Pos {
    let pos = clamp(doc, pos);
    if !doc.paras[pos.para].kind.is_text() {
        // typing next to an object: use a fresh paragraph after it (or before, at its start)
        let np = new_para(doc, ParaKind::Body, vec![]);
        let at = if pos.off == 0 { pos.para } else { pos.para + 1 };
        doc.paras.insert(at, np);
        return insert_inline(doc, Pos { para: at, off: 0 }, run);
    }
    let n = run_len(&run);
    let p = &mut doc.paras[pos.para];
    let mut runs = slice_runs(&p.runs, 0, pos.off);
    runs.push(run);
    runs.extend(slice_runs(&p.runs, pos.off, usize::MAX));
    p.runs = normalize(runs);
    Pos { para: pos.para, off: pos.off + n }
}

pub fn insert_math(doc: &mut Doc, sel: Sel, typst: &str) -> Pos {
    let (s, e) = sel.ordered();
    let pos = delete_range(doc, s, e);
    insert_inline(doc, pos, Run::Math { typst: typst.into() })
}

/// Enter: split the paragraph, with list/heading rules like a word processor.
pub fn enter(doc: &mut Doc, pos: Pos) -> Pos {
    let pos = clamp(doc, pos);
    if doc.paras.is_empty() {
        ensure_not_empty(doc);
        return Pos::default();
    }
    let kind = doc.paras[pos.para].kind.clone();
    if !kind.is_text() {
        let np = new_para(doc, ParaKind::Body, vec![]);
        let at = if pos.off == 0 { pos.para } else { pos.para + 1 };
        doc.paras.insert(at, np);
        return Pos { para: at + usize::from(pos.off == 0), off: 0 }.min_para(doc);
    }
    let empty = doc.paras[pos.para].runs.is_empty();
    let continuing = matches!(kind, ParaKind::Bullet { .. } | ParaKind::Numbered { .. } | ParaKind::Check { .. } | ParaKind::Quote | ParaKind::Callout(_));
    if empty && continuing {
        doc.paras[pos.para].kind = match kind {
            ParaKind::Bullet { indent } | ParaKind::Numbered { indent } if indent > 0 => match kind {
                ParaKind::Bullet { .. } => ParaKind::Bullet { indent: indent - 1 },
                _ => ParaKind::Numbered { indent: indent - 1 },
            },
            _ => ParaKind::Body,
        };
        return pos;
    }
    if pos.off == 0 && matches!(kind, ParaKind::Heading(_)) && !empty {
        let np = new_para(doc, ParaKind::Body, vec![]);
        doc.paras.insert(pos.para, np);
        return Pos { para: pos.para + 1, off: 0 };
    }
    let runs = &doc.paras[pos.para].runs;
    let left = slice_runs(runs, 0, pos.off);
    let right = slice_runs(runs, pos.off, usize::MAX);
    let next_kind = match kind {
        ParaKind::Heading(_) => ParaKind::Body,
        ParaKind::Check { .. } => ParaKind::Check { done: false },
        k => k,
    };
    doc.paras[pos.para].runs = normalize(left);
    let np = new_para(doc, next_kind, right);
    doc.paras.insert(pos.para + 1, np);
    Pos { para: pos.para + 1, off: 0 }
}

impl Pos {
    fn min_para(self, doc: &Doc) -> Pos {
        clamp(doc, self)
    }
}

pub fn backspace(doc: &mut Doc, sel: Sel) -> Pos {
    if !sel.is_caret() {
        let (s, e) = sel.ordered();
        return delete_range(doc, s, e);
    }
    let pos = clamp(doc, sel.head);
    if doc.paras.is_empty() {
        return pos;
    }
    if pos.off > 0 {
        return delete_range(doc, Pos { para: pos.para, off: pos.off - 1 }, pos);
    }
    let kind = doc.paras[pos.para].kind.clone();
    if kind.is_text() && kind != ParaKind::Body {
        // first Backspace at the start of a list item / heading / quote turns it into plain text
        doc.paras[pos.para].kind = ParaKind::Body;
        return pos;
    }
    if pos.para == 0 {
        return pos;
    }
    let prev = pos.para - 1;
    if !doc.paras[prev].kind.is_text() {
        doc.paras.remove(prev);
        return Pos { para: prev, off: 0 };
    }
    let at = total(&doc.paras[prev].runs);
    if !kind.is_text() {
        return Pos { para: prev, off: at };
    }
    let cur = doc.paras.remove(pos.para);
    let mut runs = std::mem::take(&mut doc.paras[prev].runs);
    runs.extend(cur.runs);
    doc.paras[prev].runs = normalize(runs);
    Pos { para: prev, off: at }
}

pub fn delete_forward(doc: &mut Doc, sel: Sel) -> Pos {
    if !sel.is_caret() {
        let (s, e) = sel.ordered();
        return delete_range(doc, s, e);
    }
    let pos = clamp(doc, sel.head);
    if doc.paras.is_empty() {
        return pos;
    }
    let len = len_of(doc, pos.para);
    if pos.off < len {
        return delete_range(doc, pos, Pos { para: pos.para, off: pos.off + 1 });
    }
    if pos.para + 1 >= doc.paras.len() {
        return pos;
    }
    if !doc.paras[pos.para + 1].kind.is_text() {
        doc.paras.remove(pos.para + 1);
        return pos;
    }
    if !doc.paras[pos.para].kind.is_text() {
        return pos;
    }
    let next = doc.paras.remove(pos.para + 1);
    let mut runs = std::mem::take(&mut doc.paras[pos.para].runs);
    runs.extend(next.runs);
    doc.paras[pos.para].runs = normalize(runs);
    pos
}

// ------------------------------------------------------------------------------------ motion

pub fn move_left(doc: &Doc, p: Pos) -> Pos {
    let p = clamp(doc, p);
    if p.off > 0 {
        Pos { para: p.para, off: p.off - 1 }
    } else if p.para > 0 {
        Pos { para: p.para - 1, off: len_of(doc, p.para - 1) }
    } else {
        p
    }
}

pub fn move_right(doc: &Doc, p: Pos) -> Pos {
    let p = clamp(doc, p);
    if p.off < len_of(doc, p.para) {
        Pos { para: p.para, off: p.off + 1 }
    } else if p.para + 1 < doc.paras.len() {
        Pos { para: p.para + 1, off: 0 }
    } else {
        p
    }
}

pub fn word_left(doc: &Doc, p: Pos) -> Pos {
    let p = clamp(doc, p);
    if p.off == 0 {
        return move_left(doc, p);
    }
    let c = chars_of(&doc.paras[p.para]);
    let mut i = p.off.min(c.len());
    while i > 0 && !is_word(c[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(c[i - 1]) {
        i -= 1;
    }
    Pos { para: p.para, off: i }
}

pub fn word_right(doc: &Doc, p: Pos) -> Pos {
    let p = clamp(doc, p);
    let c = chars_of(&doc.paras[p.para]);
    if p.off >= c.len() {
        return move_right(doc, p);
    }
    let mut i = p.off;
    while i < c.len() && !is_word(c[i]) {
        i += 1;
    }
    while i < c.len() && is_word(c[i]) {
        i += 1;
    }
    Pos { para: p.para, off: i }
}

pub fn doc_end(doc: &Doc) -> Pos {
    let last = doc.paras.len().saturating_sub(1);
    Pos { para: last, off: len_of(doc, last) }
}

/// Double-click: the word (or the run of spaces / symbol) around `p`.
pub fn word_range(doc: &Doc, p: Pos) -> (Pos, Pos) {
    let p = clamp(doc, p);
    let Some(para) = doc.paras.get(p.para) else { return (p, p) };
    if !para.kind.is_text() {
        return (Pos { para: p.para, off: 0 }, Pos { para: p.para, off: 1 });
    }
    let c = chars_of(para);
    if c.is_empty() {
        return (p, p);
    }
    let i = p.off.min(c.len() - 1);
    let class = |ch: char| if is_word(ch) { 0 } else if ch.is_whitespace() { 1 } else { 2 };
    // at the very end of the paragraph pick the last character
    let k = class(c[i]);
    let (mut a, mut b) = (i, i + 1);
    while a > 0 && class(c[a - 1]) == k {
        a -= 1;
    }
    while b < c.len() && class(c[b]) == k {
        b += 1;
    }
    (Pos { para: p.para, off: a }, Pos { para: p.para, off: b })
}

/// Triple-click: the whole paragraph.
pub fn para_range(doc: &Doc, p: Pos) -> (Pos, Pos) {
    let p = clamp(doc, p);
    (Pos { para: p.para, off: 0 }, Pos { para: p.para, off: len_of(doc, p.para) })
}

pub fn select_all(doc: &Doc) -> Sel {
    Sel { anchor: Pos::default(), head: doc_end(doc) }
}

// ------------------------------------------------------------------------------- formatting

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Bold,
    Italic,
    Underline,
    Code,
}

fn has(s: &Style, f: Fmt) -> bool {
    match f {
        Fmt::Bold => s.bold,
        Fmt::Italic => s.italic,
        Fmt::Underline => s.underline,
        Fmt::Code => s.code,
    }
}

fn set(s: &mut Style, f: Fmt, on: bool) {
    match f {
        Fmt::Bold => s.bold = on,
        Fmt::Italic => s.italic = on,
        Fmt::Underline => s.underline = on,
        Fmt::Code => s.code = on,
    }
}

fn each_range(doc: &Doc, a: Pos, b: Pos) -> Vec<(usize, usize, usize)> {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    (s.para..=e.para.min(doc.paras.len().saturating_sub(1)))
        .filter(|&i| doc.paras.get(i).is_some_and(|p| p.kind.is_text()))
        .map(|i| (i, if i == s.para { s.off } else { 0 }, if i == e.para { e.off } else { usize::MAX }))
        .collect()
}

/// True if every character in the range already has the format.
pub fn all_have(doc: &Doc, a: Pos, b: Pos, f: Fmt) -> bool {
    let mut any = false;
    for (i, from, to) in each_range(doc, a, b) {
        for r in slice_runs(&doc.paras[i].runs, from, to) {
            if let Run::Text { style, .. } = r {
                any = true;
                if !has(&style, f) {
                    return false;
                }
            }
        }
    }
    any
}

pub fn set_format(doc: &mut Doc, a: Pos, b: Pos, f: Fmt, on: bool) {
    for (i, from, to) in each_range(doc, a, b) {
        let runs = &doc.paras[i].runs;
        let mut out = slice_runs(runs, 0, from);
        for mut r in slice_runs(runs, from, to) {
            if let Run::Text { style, .. } = &mut r {
                set(style, f, on);
            }
            out.push(r);
        }
        if to != usize::MAX {
            out.extend(slice_runs(runs, to, usize::MAX));
        }
        doc.paras[i].runs = normalize(out);
    }
}

/// Toggle a format on the selection like Ctrl+B: removes it if everything has it, else adds it.
pub fn toggle_format(doc: &mut Doc, a: Pos, b: Pos, f: Fmt) -> bool {
    let on = !all_have(doc, a, b, f);
    set_format(doc, a, b, f, on);
    on
}

pub fn set_link(doc: &mut Doc, a: Pos, b: Pos, link: Option<Link>) {
    for (i, from, to) in each_range(doc, a, b) {
        let runs = &doc.paras[i].runs;
        let mut out = slice_runs(runs, 0, from);
        for mut r in slice_runs(runs, from, to) {
            if let Run::Text { link: l, .. } = &mut r {
                *l = link.clone();
            }
            out.push(r);
        }
        if to != usize::MAX {
            out.extend(slice_runs(runs, to, usize::MAX));
        }
        doc.paras[i].runs = normalize(out);
    }
}

/// Paragraph style for every paragraph touched by the selection; applying the same kind again resets to body.
pub fn toggle_kind(doc: &mut Doc, a: Pos, b: Pos, kind: ParaKind) {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    let idx: Vec<usize> = (s.para..=e.para.min(doc.paras.len().saturating_sub(1))).filter(|&i| doc.paras[i].kind.is_text()).collect();
    let same = !idx.is_empty() && idx.iter().all(|&i| doc.paras[i].kind == kind);
    for i in idx {
        doc.paras[i].kind = if same { ParaKind::Body } else { kind.clone() };
    }
}

pub fn indent(doc: &mut Doc, a: Pos, b: Pos, delta: i32) -> bool {
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    let mut changed = false;
    for i in s.para..=e.para.min(doc.paras.len().saturating_sub(1)) {
        let k = &mut doc.paras[i].kind;
        if let ParaKind::Bullet { indent } | ParaKind::Numbered { indent } = k {
            let n = (*indent as i32 + delta).clamp(0, 5) as u8;
            if n != *indent {
                *indent = n;
                changed = true;
            }
        }
    }
    changed
}

pub fn toggle_check(doc: &mut Doc, para: usize) {
    if let Some(ParaKind::Check { done }) = doc.paras.get_mut(para).map(|p| &mut p.kind) {
        *done = !*done;
    }
}

// ----------------------------------------------------------------------------- text <-> clipboard

pub fn slice_plain(doc: &Doc, a: Pos, b: Pos) -> String {
    let (s, e) = if a <= b { (clamp(doc, a), clamp(doc, b)) } else { (clamp(doc, b), clamp(doc, a)) };
    let mut out = Vec::new();
    for i in s.para..=e.para.min(doc.paras.len().saturating_sub(1)) {
        let p = &doc.paras[i];
        if !p.kind.is_text() {
            continue;
        }
        let from = if i == s.para { s.off } else { 0 };
        let to = if i == e.para { e.off } else { usize::MAX };
        let q = Para { id: p.id, kind: p.kind.clone(), runs: slice_runs(&p.runs, from, to) };
        out.push(q.plain());
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------------- autoformat

/// Call right after a space was typed (caret after it): `- `, `* `, `1. `, `[] `, `# `, `> ` at the
/// start of a plain paragraph become a list, checklist, heading or quote. Returns the new caret.
pub fn autoformat_space(doc: &mut Doc, pos: Pos) -> Option<Pos> {
    let p = doc.paras.get(pos.para)?;
    if p.kind != ParaKind::Body {
        return None;
    }
    let c = chars_of(p);
    let before: String = c.iter().take(pos.off).collect();
    let kind = match before.as_str() {
        "- " | "* " | "• " => ParaKind::Bullet { indent: 0 },
        "[] " | "[ ] " => ParaKind::Check { done: false },
        "# " => ParaKind::Heading(1),
        "## " => ParaKind::Heading(2),
        "### " => ParaKind::Heading(3),
        "> " => ParaKind::Quote,
        s if s.len() >= 3 && s.ends_with(". ") && s[..s.len() - 2].chars().all(|ch| ch.is_ascii_digit()) => ParaKind::Numbered { indent: 0 },
        _ => return None,
    };
    let p = &mut doc.paras[pos.para];
    p.runs = normalize(slice_runs(&p.runs, pos.off, usize::MAX));
    p.kind = kind;
    Some(Pos { para: pos.para, off: 0 })
}

/// Call right after `$` was typed (caret after it): `$x^2$` becomes an inline math object.
pub fn autoformat_dollar(doc: &mut Doc, pos: Pos) -> Option<Pos> {
    let p = doc.paras.get(pos.para)?;
    if !p.kind.is_text() || pos.off < 3 {
        return None;
    }
    let c = chars_of(p);
    if c.get(pos.off - 1) != Some(&'$') {
        return None;
    }
    let open = c[..pos.off - 1].iter().rposition(|&ch| ch == '$')?;
    let inner: String = c[open + 1..pos.off - 1].iter().collect();
    if inner.trim().is_empty() || inner.contains('\u{FFFC}') {
        return None;
    }
    let p = &mut doc.paras[pos.para];
    let mut runs = slice_runs(&p.runs, 0, open);
    runs.push(Run::Math { typst: inner.trim().to_string() });
    runs.extend(slice_runs(&p.runs, pos.off, usize::MAX));
    p.runs = normalize(runs);
    Some(Pos { para: pos.para, off: open + 1 })
}

/// Insert an object paragraph (image, plot, display math, rule) after the paragraph at `pos`;
/// an empty body paragraph is replaced. Returns the position after the object.
pub fn insert_object(doc: &mut Doc, pos: Pos, kind: ParaKind) -> Pos {
    ensure_not_empty(doc);
    let pos = clamp(doc, pos);
    let replace = doc.paras[pos.para].kind == ParaKind::Body && doc.paras[pos.para].runs.is_empty();
    let np = new_para(doc, kind, vec![]);
    let at_start = doc.paras[pos.para].kind.is_text() && pos.off == 0 && total(&doc.paras[pos.para].runs) > 0;
    let at = if replace || at_start { pos.para } else { pos.para + 1 };
    if replace {
        doc.paras[at] = np;
    } else {
        doc.paras.insert(at, np);
    }
    if at + 1 >= doc.paras.len() {
        let b = new_para(doc, ParaKind::Body, vec![]);
        doc.paras.push(b);
    }
    Pos { para: at + 1, off: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(markup: &str) -> Doc {
        Doc::from_markup(markup)
    }
    fn texts(doc: &Doc) -> Vec<String> {
        doc.paras.iter().map(|p| p.plain()).collect()
    }
    fn p(para: usize, off: usize) -> Pos {
        Pos { para, off }
    }

    #[test]
    fn typing_and_enter_split_and_join_paragraphs() {
        let mut doc = d("hello world");
        let mut at = insert_text(&mut doc, Sel::caret(p(0, 5)), " there", Style::default());
        assert_eq!(texts(&doc), ["hello there world"]);
        at = enter(&mut doc, at);
        assert_eq!(texts(&doc), ["hello there", " world"]);
        assert_eq!(at, p(1, 0));
        at = backspace(&mut doc, Sel::caret(at));
        assert_eq!(texts(&doc), ["hello there world"]);
        assert_eq!(at, p(0, 11));
        let end = delete_forward(&mut doc, Sel::caret(p(0, 17)));
        assert_eq!(end, p(0, 17));
    }

    #[test]
    fn multi_paragraph_selection_delete_and_replace() {
        let mut doc = d("= Title\n\nfirst para\n\nsecond para\n\nthird");
        let at = delete_range(&mut doc, p(1, 6), p(2, 7));
        assert_eq!(texts(&doc), ["Title", "first para", "third"]);
        assert_eq!(at, p(1, 6));
        let mut doc = d("one\n\ntwo\n\nthree");
        let at = insert_text(&mut doc, Sel { anchor: p(0, 1), head: p(2, 2) }, "X", Style::default());
        assert_eq!(texts(&doc), ["oXree"]);
        assert_eq!(at, p(0, 2));
    }

    #[test]
    fn pasting_lines_makes_paragraphs() {
        let mut doc = d("ab");
        let at = insert_text(&mut doc, Sel::caret(p(0, 1)), "1\n2\n3", Style::default());
        assert_eq!(texts(&doc), ["a1", "2", "3b"]);
        assert_eq!(at, p(2, 1));
    }

    #[test]
    fn enter_in_lists_continues_and_ends_them() {
        let mut doc = d("- one");
        let at = enter(&mut doc, p(0, 3));
        assert_eq!(doc.paras[1].kind, ParaKind::Bullet { indent: 0 });
        let at = enter(&mut doc, at);
        assert_eq!(doc.paras[1].kind, ParaKind::Body, "Enter on an empty item ends the list");
        assert_eq!(at, p(1, 0));
        let mut c = d("- ☑ done");
        enter(&mut c, p(0, 4));
        assert_eq!(c.paras[1].kind, ParaKind::Check { done: false });
    }

    #[test]
    fn heading_then_enter_gives_body_and_backspace_unlists() {
        let mut doc = d("= Title");
        enter(&mut doc, p(0, 5));
        assert_eq!(doc.paras[1].kind, ParaKind::Body);
        let mut b = d("- item");
        let at = backspace(&mut b, Sel::caret(p(0, 0)));
        assert_eq!(b.paras[0].kind, ParaKind::Body);
        assert_eq!(at, p(0, 0));
        let mut h = d("= Title");
        enter(&mut h, p(0, 0));
        assert_eq!(h.paras[0].kind, ParaKind::Body);
        assert_eq!(h.paras[1].kind, ParaKind::Heading(1));
    }

    #[test]
    fn backspace_at_start_removes_a_preceding_object() {
        let mut doc = d("text");
        insert_object(&mut doc, p(0, 4), ParaKind::Rule);
        assert!(matches!(doc.paras[1].kind, ParaKind::Rule));
        let at = backspace(&mut doc, Sel::caret(p(2, 0)));
        assert!(doc.paras.iter().all(|q| q.kind != ParaKind::Rule));
        assert_eq!(at, p(1, 0));
    }

    #[test]
    fn word_motion_and_selection_helpers() {
        let doc = d("hello, brave new world");
        assert_eq!(word_right(&doc, p(0, 0)), p(0, 5));
        assert_eq!(word_right(&doc, p(0, 5)), p(0, 12));
        assert_eq!(word_left(&doc, p(0, 12)), p(0, 7));
        assert_eq!(word_range(&doc, p(0, 8)), (p(0, 7), p(0, 12)));
        assert_eq!(para_range(&doc, p(0, 3)), (p(0, 0), p(0, 22)));
        assert_eq!(select_all(&doc).head, p(0, 22));
        let two = d("a\n\nb");
        assert_eq!(move_right(&two, p(0, 1)), p(1, 0));
        assert_eq!(move_left(&two, p(1, 0)), p(0, 1));
    }

    #[test]
    fn formatting_toggles_on_ranges_and_across_paragraphs() {
        let mut doc = d("hello world\n\nsecond");
        assert!(toggle_format(&mut doc, p(0, 6), p(1, 3), Fmt::Bold));
        assert_eq!(doc.to_typst(), "hello *world*\n\n#strong[sec]ond");
        assert!(all_have(&doc, p(0, 6), p(1, 3), Fmt::Bold));
        assert!(!toggle_format(&mut doc, p(0, 6), p(1, 3), Fmt::Bold));
        assert_eq!(doc.to_typst(), "hello world\n\nsecond");
        toggle_kind(&mut doc, p(0, 0), p(1, 0), ParaKind::Heading(2));
        assert!(doc.paras.iter().all(|q| q.kind == ParaKind::Heading(2)));
        toggle_kind(&mut doc, p(0, 0), p(1, 0), ParaKind::Heading(2));
        assert!(doc.paras.iter().all(|q| q.kind == ParaKind::Body));
        assert!(style_at(&d("*bold* x"), p(0, 3)).bold);
    }

    #[test]
    fn autoformat_markers_and_inline_math() {
        let mut doc = d("");
        let at = insert_text(&mut doc, Sel::caret(p(0, 0)), "- ", Style::default());
        let at = autoformat_space(&mut doc, at).unwrap();
        assert_eq!((doc.paras[0].kind.clone(), at), (ParaKind::Bullet { indent: 0 }, p(0, 0)));
        for (typed, kind) in [("1. ", ParaKind::Numbered { indent: 0 }), ("[] ", ParaKind::Check { done: false }), ("## ", ParaKind::Heading(2)), ("> ", ParaKind::Quote)] {
            let mut x = d("");
            let at = insert_text(&mut x, Sel::caret(p(0, 0)), typed, Style::default());
            autoformat_space(&mut x, at).unwrap();
            assert_eq!(x.paras[0].kind, kind, "{typed}");
            assert!(x.paras[0].runs.is_empty());
        }
        let mut m = d("see ");
        let at = insert_text(&mut m, Sel::caret(p(0, 4)), "$x^2$", Style::default());
        let at = autoformat_dollar(&mut m, at).unwrap();
        assert_eq!(m.paras[0].runs[1], Run::Math { typst: "x^2".into() });
        assert_eq!(at, p(0, 5));
        // a lone dollar sign or a mid-paragraph "- " is left alone
        let mut price = d("costs 5");
        let at = insert_text(&mut price, Sel::caret(p(0, 7)), " $", Style::default());
        assert!(autoformat_dollar(&mut price, at).is_none());
        let mut mid = d("a ");
        let at = insert_text(&mut mid, Sel::caret(p(0, 2)), "- ", Style::default());
        assert!(autoformat_space(&mut mid, at).is_none());
    }

    #[test]
    fn inline_math_is_one_caret_step_and_deletes_whole() {
        let mut doc = d("a $x$ b");
        assert_eq!(doc.paras[0].text_len(), 5);
        let at = backspace(&mut doc, Sel::caret(p(0, 3)));
        assert_eq!(doc.to_typst(), "a  b");
        assert_eq!(at, p(0, 2));
    }

    #[test]
    fn slice_plain_for_copy() {
        let doc = d("one two\n\nthree $x$");
        assert_eq!(slice_plain(&doc, p(0, 4), p(1, 7)), "two\nthree $x$");
    }

    #[test]
    fn deleting_everything_leaves_one_empty_paragraph() {
        let mut doc = d("a\n\nb");
        let at = delete_range(&mut doc, p(0, 0), p(1, 1));
        assert_eq!(texts(&doc), [""]);
        assert_eq!(at, p(0, 0));
        let mut o = Doc::new();
        o.push(ParaKind::Rule, vec![]);
        delete_range(&mut o, p(0, 0), p(0, 1));
        assert_eq!(o.paras.len(), 1);
    }

    #[test]
    fn indent_levels_stay_in_range() {
        let mut doc = d("- a\n- b");
        assert!(indent(&mut doc, p(1, 0), p(1, 0), 1));
        assert_eq!(doc.paras[1].kind, ParaKind::Bullet { indent: 1 });
        assert!(!indent(&mut doc, p(0, 0), p(0, 0), -1));
    }
}

#[cfg(test)]
mod mixed_tests {
    use super::*;

    fn p(para: usize, off: usize) -> Pos {
        Pos { para, off }
    }

    fn doc_with_rule() -> Doc {
        let mut d = Doc::from_markup("before\n\nafter");
        d.paras.insert(1, Para { id: 99, kind: ParaKind::Rule, runs: vec![] });
        d
    }

    #[test]
    fn selection_through_an_object_removes_it_and_joins_nothing_wrongly() {
        let mut d = doc_with_rule();
        let at = delete_range(&mut d, p(0, 3), p(2, 2));
        assert_eq!(d.paras.iter().map(|x| x.plain()).collect::<Vec<_>>(), ["befter"]);
        assert_eq!(at, p(0, 3));
    }

    #[test]
    fn selection_ending_on_an_object_keeps_the_object_when_it_is_not_covered() {
        let mut d = doc_with_rule();
        delete_range(&mut d, p(0, 3), p(1, 0));
        assert_eq!(d.paras.len(), 3);
        assert_eq!(d.paras[0].plain(), "bef");
        assert!(matches!(d.paras[1].kind, ParaKind::Rule));
    }

    #[test]
    fn selection_starting_after_an_object_keeps_it() {
        let mut d = doc_with_rule();
        delete_range(&mut d, p(1, 1), p(2, 2));
        assert!(matches!(d.paras[1].kind, ParaKind::Rule));
        assert_eq!(d.paras[2].plain(), "ter");
    }

    #[test]
    fn whole_object_selected_then_typing_replaces_it_with_text() {
        let mut d = doc_with_rule();
        let at = insert_text(&mut d, Sel { anchor: p(1, 0), head: p(1, 1) }, "x", Style::default());
        assert!(d.paras.iter().all(|q| q.kind != ParaKind::Rule));
        assert!(d.paras.iter().any(|q| q.plain() == "x"), "{:?}", d.paras);
        assert_eq!(d.paras[at.para].plain(), "x");
    }

    #[test]
    fn enter_and_backspace_never_panic_on_any_position() {
        let base = {
            let mut d = Doc::from_markup("= T\n\n- a\n- \n\nb $x$ c\n\n#table(columns: 1, [a])");
            d.paras.insert(2, Para { id: 77, kind: ParaKind::Rule, runs: vec![] });
            d
        };
        for i in 0..base.paras.len() {
            for off in 0..=base.paras[i].text_len().max(1) {
                for op in 0..4 {
                    let mut d = base.clone();
                    let at = Sel::caret(p(i, off));
                    let r = match op {
                        0 => enter(&mut d, at.head),
                        1 => backspace(&mut d, at),
                        2 => delete_forward(&mut d, at),
                        _ => insert_text(&mut d, at, "q\nw", Style::default()),
                    };
                    assert!(!d.paras.is_empty());
                    let c = clamp(&d, r);
                    assert_eq!(c, r, "op {op} at {i}:{off} returned an out-of-range caret");
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------ page links

/// While typing `[[Ti`: the offset of the `[[` and the text typed after it (no `]` yet).
pub fn link_query(doc: &Doc, pos: Pos) -> Option<(usize, String)> {
    let p = doc.paras.get(pos.para)?;
    if !p.kind.is_text() {
        return None;
    }
    let c = chars_of(p);
    let end = pos.off.min(c.len());
    let k = (0..end.saturating_sub(1)).rev().find(|&k| c[k] == '[' && c[k + 1] == '[')?;
    let q: String = c[k + 2..end].iter().collect();
    (!q.contains([']', '\u{FFFC}', '\n']) && q.chars().count() <= 80).then_some((k, q))
}

/// Replace `[[query` before the caret with a link to `title`. Returns the caret after the link.
pub fn accept_link(doc: &mut Doc, pos: Pos, title: &str) -> Option<Pos> {
    let (k, _) = link_query(doc, pos)?;
    let p = &mut doc.paras[pos.para];
    let mut runs = slice_runs(&p.runs, 0, k);
    runs.push(Run::Text { text: title.to_string(), style: Style::default(), link: Some(Link::Page(title.to_string())) });
    runs.extend(slice_runs(&p.runs, pos.off, usize::MAX));
    p.runs = normalize(runs);
    Some(Pos { para: pos.para, off: k + title.chars().count() })
}

/// Call right after `]` was typed (caret after it): `[[Title]]` becomes a link to that page.
pub fn autoformat_link(doc: &mut Doc, pos: Pos) -> Option<Pos> {
    let p = doc.paras.get(pos.para)?;
    let c = chars_of(p);
    if pos.off < 4 || c.get(pos.off - 1) != Some(&']') || c.get(pos.off - 2) != Some(&']') {
        return None;
    }
    let (k, _) = link_query(doc, Pos { para: pos.para, off: pos.off - 2 })?;
    let title: String = chars_of(&doc.paras[pos.para])[k + 2..pos.off - 2].iter().collect();
    let title = title.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let p = &mut doc.paras[pos.para];
    let mut runs = slice_runs(&p.runs, 0, k);
    runs.push(Run::Text { text: title.clone(), style: Style::default(), link: Some(Link::Page(title.clone())) });
    runs.extend(slice_runs(&p.runs, pos.off, usize::MAX));
    p.runs = normalize(runs);
    Some(Pos { para: pos.para, off: k + title.chars().count() })
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn typing_brackets_makes_a_link_and_autocomplete_replaces_the_query() {
        let mut d = Doc::from_markup("see ");
        let at = insert_text(&mut d, Sel::caret(Pos { para: 0, off: 4 }), "[[Lim", Style::default());
        assert_eq!(link_query(&d, at), Some((4, "Lim".to_string())));
        let after = accept_link(&mut d, at, "Limits").unwrap();
        assert_eq!(d.paras[0].plain(), "see Limits");
        assert_eq!(after, Pos { para: 0, off: 10 });
        assert!(matches!(&d.paras[0].runs[1], Run::Text { link: Some(Link::Page(t)), .. } if t == "Limits"));

        let mut e = Doc::from_markup("x ");
        let at = insert_text(&mut e, Sel::caret(Pos { para: 0, off: 2 }), "[[Other]]", Style::default());
        let at = autoformat_link(&mut e, at).unwrap();
        assert_eq!(e.to_typst(), "x [[Other]]");
        assert_eq!(at, Pos { para: 0, off: 7 });
        // no query once the brackets are closed, and a lone [ is nothing
        assert!(link_query(&e, at).is_none());
        assert!(link_query(&Doc::from_markup("a [b"), Pos { para: 0, off: 4 }).is_none());
        assert!(autoformat_link(&mut Doc::from_markup("[[]]"), Pos { para: 0, off: 4 }).is_none());
    }
}
