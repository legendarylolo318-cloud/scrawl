//! Typst markup <-> document paragraphs, for the subset the text editor produces.

use crate::model::*;

/// Escape plain text so Typst shows it literally.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '#' | '$' | '*' | '_' | '`' | '<' | '>' | '@' | '/' | '[' | ']' | '~' | '"' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(char::is_alphanumeric)
}

/// Inline markup for the runs of one paragraph.
fn runs_markup(runs: &[Run]) -> String {
    let mut s = String::new();
    for (i, r) in runs.iter().enumerate() {
        match r {
            Run::Math { typst } => s += &format!("${typst}$"),
            Run::Text { text, style, link } => {
                // Typst only reads * and _ as markup when they are not inside a word
                let before = s.chars().last();
                let after = runs.get(i + 1).and_then(|n| match n {
                    Run::Text { text, .. } => text.chars().next(),
                    Run::Math { .. } => None,
                });
                let wordy_edge = is_word(before) || is_word(after);
                let mut body = if style.code && !text.contains('`') { format!("`{text}`") } else { escape_text(text) };
                if let Some(l) = link {
                    body = match l {
                        Link::Page(t) => format!("[[{t}]]"),
                        Link::Url(u) => format!("#link(\"{}\")[{}]", u.replace('\\', "\\\\").replace('"', "\\\""), body),
                    };
                }
                if style.italic {
                    body = if wordy_edge { format!("#emph[{body}]") } else { format!("_{body}_") };
                }
                if style.bold {
                    body = if wordy_edge { format!("#strong[{body}]") } else { format!("*{body}*") };
                }
                if style.underline {
                    body = format!("#underline[{body}]");
                }
                s += &body;
            }
        }
    }
    s
}

/// Escape characters at the start of a plain paragraph that Typst would turn into list or heading markers.
fn guard_line_start(s: String) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c @ ('=' | '-' | '+')) => format!("\\{c}{}", chars.as_str()),
        Some(c) if c.is_ascii_digit() => {
            let digits = s.chars().take_while(char::is_ascii_digit).count();
            if s[digits..].starts_with('.') {
                format!("{}\\.{}", &s[..digits], &s[digits + 1..])
            } else {
                s
            }
        }
        _ => s,
    }
}

/// Typst for one paragraph (no trailing newline). Objects that are not text return `None`.
pub fn para_markup(p: &Para) -> Option<String> {
    let body = || runs_markup(&p.runs);
    Some(match &p.kind {
        ParaKind::Body => guard_line_start(body()),
        ParaKind::Heading(l) => format!("{} {}", "=".repeat((*l).clamp(1, 3) as usize), body()),
        ParaKind::Bullet { indent } => format!("{}- {}", "  ".repeat(*indent as usize), body()),
        ParaKind::Numbered { indent } => format!("{}+ {}", "  ".repeat(*indent as usize), body()),
        ParaKind::Check { done } => format!("- {} {}", if *done { "☑" } else { "☐" }, body()),
        ParaKind::Quote => format!("#quote(block: true)[{}]", body()),
        ParaKind::Callout(_) => body(),
        ParaKind::Markup(src) => src.clone(),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------- parsing

struct P<'a> {
    c: &'a [char],
    i: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Stop {
    End,
    Star,
    Under,
    Bracket,
}

impl P<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn starts(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, ch)| self.peek(k) == Some(ch))
    }
    fn find_close(&self, from: usize, close: char) -> Option<usize> {
        let mut j = from;
        while j < self.c.len() {
            if self.c[j] == '\\' {
                j += 2;
                continue;
            }
            if self.c[j] == close {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    fn inline(&mut self, mut style: Style, stop: Stop, link: Option<Link>) -> Result<Vec<Run>, ()> {
        let mut runs = Vec::new();
        let mut buf = String::new();
        macro_rules! flush {
            () => {
                if !buf.is_empty() {
                    runs.push(Run::Text { text: std::mem::take(&mut buf), style, link: link.clone() });
                }
            };
        }
        let _ = &mut style;
        while let Some(ch) = self.peek(0) {
            match ch {
                '\\' => {
                    let n = self.peek(1).ok_or(())?;
                    buf.push(n);
                    self.i += 2;
                }
                '$' => {
                    let j = self.find_close(self.i + 1, '$').ok_or(())?;
                    flush!();
                    let src: String = self.c[self.i + 1..j].iter().collect();
                    runs.push(Run::Math { typst: src });
                    self.i = j + 1;
                }
                '[' if self.peek(1) == Some('[') => {
                    let mut j = self.i + 2;
                    while j + 1 < self.c.len() && !(self.c[j] == ']' && self.c[j + 1] == ']') {
                        j += 1;
                    }
                    if j + 1 >= self.c.len() {
                        return Err(());
                    }
                    flush!();
                    let t: String = self.c[self.i + 2..j].iter().collect();
                    runs.push(Run::Text { text: t.clone(), style, link: Some(Link::Page(t)) });
                    self.i = j + 2;
                }
                '`' => {
                    let j = self.find_close(self.i + 1, '`').ok_or(())?;
                    flush!();
                    let t: String = self.c[self.i + 1..j].iter().collect();
                    runs.push(Run::Text { text: t, style: Style { code: true, ..style }, link: link.clone() });
                    self.i = j + 1;
                }
                '*' | '_' => {
                    let me = if ch == '*' { Stop::Star } else { Stop::Under };
                    let prev = if self.i > 0 { self.c.get(self.i - 1).copied() } else { None };
                    let in_word = is_word(prev) && is_word(self.peek(1));
                    if stop == me && !in_word {
                        flush!();
                        self.i += 1;
                        return Ok(runs);
                    }
                    if in_word {
                        buf.push(ch);
                        self.i += 1;
                        continue;
                    }
                    flush!();
                    self.i += 1;
                    let st = if ch == '*' { Style { bold: true, ..style } } else { Style { italic: true, ..style } };
                    let inner = self.inline(st, me, link.clone())?;
                    runs.extend(inner);
                }
                '#' => {
                    flush!();
                    let (name, st): (&str, Style) = if self.starts("#strong[") {
                        ("#strong[", Style { bold: true, ..style })
                    } else if self.starts("#emph[") {
                        ("#emph[", Style { italic: true, ..style })
                    } else if self.starts("#underline[") {
                        ("#underline[", Style { underline: true, ..style })
                    } else if self.starts("#link(\"") {
                        let mut j = self.i + 7;
                        let mut url = String::new();
                        while j < self.c.len() && self.c[j] != '"' {
                            if self.c[j] == '\\' && j + 1 < self.c.len() {
                                j += 1;
                            }
                            url.push(self.c[j]);
                            j += 1;
                        }
                        if !(self.c.get(j + 1) == Some(&')') && self.c.get(j + 2) == Some(&'[')) {
                            return Err(());
                        }
                        self.i = j + 3;
                        let inner = self.inline(style, Stop::Bracket, Some(Link::Url(url)))?;
                        runs.extend(inner);
                        continue;
                    } else {
                        return Err(());
                    };
                    self.i += name.chars().count();
                    let inner = self.inline(st, Stop::Bracket, link.clone())?;
                    runs.extend(inner);
                }
                ']' if stop == Stop::Bracket => {
                    flush!();
                    self.i += 1;
                    return Ok(runs);
                }
                ']' | '<' | '>' | '@' | '~' => return Err(()),
                _ => {
                    buf.push(ch);
                    self.i += 1;
                }
            }
        }
        if stop != Stop::End {
            return Err(());
        }
        flush!();
        Ok(runs)
    }
}

fn parse_inline(s: &str) -> Option<Vec<Run>> {
    let c: Vec<char> = s.chars().collect();
    let mut p = P { c: &c, i: 0 };
    p.inline(Style::default(), Stop::End, None).ok().map(normalize)
}

fn balance(s: &str) -> i32 {
    s.chars().map(|c| match c { '(' | '[' | '{' => 1, ')' | ']' | '}' => -1, _ => 0 }).sum()
}

impl Doc {
    /// Append the paragraphs described by Typst `markup` (the old text-block format).
    pub fn append_markup(&mut self, markup: &str) {
        self.append_markup_as(markup, None)
    }

    /// Like `append_markup`; with `callout` every text paragraph becomes a callout of that kind.
    pub fn append_markup_as(&mut self, markup: &str, callout: Option<CalloutKind>) {
        let lines: Vec<&str> = markup.lines().collect();
        let mut i = 0;
        let mut body: Vec<&str> = Vec::new();
        let flush = |doc: &mut Doc, body: &mut Vec<&str>| {
            if body.is_empty() {
                return;
            }
            let text = body.join(" ");
            body.clear();
            doc.push_text_para(ParaKind::Body, &text, callout);
        };
        while i < lines.len() {
            let raw = lines[i];
            let t = raw.trim_start();
            let indent = ((raw.len() - t.len()) / 2).min(6) as u8;
            if t.trim().is_empty() {
                flush(self, &mut body);
                i += 1;
            } else if t.starts_with("```") {
                flush(self, &mut body);
                let mut j = i + 1;
                while j < lines.len() && !lines[j].trim_start().starts_with("```") {
                    j += 1;
                }
                let end = (j + 1).min(lines.len());
                self.push(ParaKind::Markup(lines[i..end].join("\n")), vec![]);
                i = end;
            } else if t.starts_with('#') && !t.starts_with("#quote(block: true)[") && !t.starts_with("#strong[") && !t.starts_with("#emph[") && !t.starts_with("#underline[") && !t.starts_with("#link(") {
                flush(self, &mut body);
                let mut j = i;
                let mut bal = balance(lines[j]);
                while bal > 0 && j + 1 < lines.len() {
                    j += 1;
                    bal += balance(lines[j]);
                }
                self.push(ParaKind::Markup(lines[i..=j].join("\n")), vec![]);
                i = j + 1;
            } else if let Some(rest) = heading(t) {
                flush(self, &mut body);
                self.push_text_para(ParaKind::Heading(rest.0), rest.1, callout);
                i += 1;
            } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("• ")) {
                flush(self, &mut body);
                if let Some(done) = rest.strip_prefix("☐ ").map(|r| (false, r)).or_else(|| rest.strip_prefix("☑ ").map(|r| (true, r))).or_else(|| rest.strip_prefix("[ ] ").map(|r| (false, r))).or_else(|| rest.strip_prefix("[x] ").map(|r| (true, r))) {
                    self.push_text_para(ParaKind::Check { done: done.0 }, done.1, callout);
                } else {
                    self.push_text_para(ParaKind::Bullet { indent }, rest, callout);
                }
                i += 1;
            } else if let Some(rest) = t.strip_prefix("+ ") {
                flush(self, &mut body);
                self.push_text_para(ParaKind::Numbered { indent }, rest, callout);
                i += 1;
            } else if let Some(inner) = t.strip_prefix("#quote(block: true)[").and_then(|r| r.strip_suffix(']')) {
                flush(self, &mut body);
                match parse_inline(inner) {
                    Some(runs) => {
                        self.push(ParaKind::Quote, runs);
                    }
                    None => {
                        self.push(ParaKind::Markup(t.to_string()), vec![]);
                    }
                }
                i += 1;
            } else {
                body.push(t);
                i += 1;
            }
        }
        flush(self, &mut body);
    }

    fn push_text_para(&mut self, kind: ParaKind, text: &str, callout: Option<CalloutKind>) {
        let kind = match (callout, &kind) {
            (Some(k), ParaKind::Body) => ParaKind::Callout(k),
            _ => kind,
        };
        match parse_inline(text) {
            Some(runs) => {
                self.push(kind, runs);
            }
            None => {
                // unknown inline markup: keep the source line exactly
                let prefix = match &kind {
                    ParaKind::Heading(l) => format!("{} ", "=".repeat(*l as usize)),
                    ParaKind::Bullet { indent } => format!("{}- ", "  ".repeat(*indent as usize)),
                    ParaKind::Numbered { indent } => format!("{}+ ", "  ".repeat(*indent as usize)),
                    ParaKind::Check { done } => format!("- {} ", if *done { "☑" } else { "☐" }),
                    _ => String::new(),
                };
                self.push(ParaKind::Markup(format!("{prefix}{text}")), vec![]);
            }
        }
    }

    pub fn from_markup(markup: &str) -> Doc {
        let mut d = Doc::new();
        d.append_markup(markup);
        d
    }

    /// Typst source for the whole document (text paragraphs only; objects are written by the page).
    pub fn to_typst(&self) -> String {
        let mut s = String::new();
        let mut prev_list = false;
        for p in &self.paras {
            let Some(m) = para_markup(p) else { continue };
            if !s.is_empty() {
                s += if prev_list && p.kind.is_list() { "\n" } else { "\n\n" };
            }
            s += &m;
            prev_list = p.kind.is_list();
        }
        s
    }
}

/// `= Title`, `== Sub`: (level, rest).
fn heading(t: &str) -> Option<(u8, &str)> {
    let n = t.chars().take_while(|c| *c == '=').count();
    if n == 0 || n > 3 {
        return None;
    }
    t[n..].strip_prefix(' ').map(|r| (n as u8, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Style {
        Style { bold: true, ..Style::default() }
    }

    #[test]
    fn canonical_markup_round_trips() {
        for src in [
            "= Chain rule",
            "== Sub heading\n\nSome *bold* and _italic_ words with $f(g(x))$ inside.",
            "- one\n- two\n  - nested\n+ first\n+ second",
            "- ☐ buy brandy\n- ☑ done",
            "see [[Limits]] and #link(\"https://example.com\")[the site] and `code`",
            "#underline[under] and #strong[mid]word",
            "#quote(block: true)[A wise saying]",
            "#table(columns: 2, [a], [b])",
            "```rust\nfn main() {}\n```",
        ] {
            let d = Doc::from_markup(src);
            assert_eq!(d.to_typst(), src, "{src}");
        }
    }

    #[test]
    fn text_with_special_characters_is_escaped_and_comes_back() {
        let mut d = Doc::new();
        for t in ["- not a list", "1. not numbered", "= not a heading", "a*b_c $5 #hash [x] <y> @z \"q\" 'r' \\ end", "price 5% & 100/3"] {
            d.push(ParaKind::Body, vec![Run::plain(t)]);
        }
        let back = Doc::from_markup(&d.to_typst());
        assert_eq!(back.paras.len(), 5);
        for (a, b) in d.paras.iter().zip(&back.paras) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.runs, b.runs);
        }
    }

    #[test]
    fn hostile_markup_is_never_executed_only_kept_verbatim() {
        let d = Doc::from_markup("#read(\"/etc/passwd\")\n\nplain");
        assert!(matches!(&d.paras[0].kind, ParaKind::Markup(s) if s.starts_with("#read")));
        assert_eq!(d.paras[1].plain(), "plain");
        // text containing a hash is escaped when written
        let mut e = Doc::new();
        e.push(ParaKind::Body, vec![Run::plain("#read(\"/etc/passwd\") $x$")]);
        assert!(e.to_typst().starts_with("\\#read"));
    }

    #[test]
    fn consecutive_lines_join_like_typst_paragraphs() {
        let d = Doc::from_markup("first line\nsecond line\n\nnew paragraph");
        assert_eq!(d.paras.len(), 2);
        assert_eq!(d.paras[0].plain(), "first line second line");
    }

    #[test]
    fn unknown_inline_markup_keeps_the_line() {
        let d = Doc::from_markup("= Title with #super[2]\ntext #h(1em) more");
        assert!(matches!(&d.paras[0].kind, ParaKind::Markup(s) if s == "= Title with #super[2]"));
        assert!(matches!(&d.paras[1].kind, ParaKind::Markup(_)));
    }

    #[test]
    fn style_runs_merge() {
        let mut d = Doc::new();
        d.push(ParaKind::Body, vec![Run::plain("a"), Run::plain(" b "), Run::styled("c", bold())]);
        assert_eq!(d.paras[0].runs.len(), 2);
        assert_eq!(d.paras[0].plain(), "a b c");
        assert_eq!(d.to_typst(), "a b *c*");
    }

    #[test]
    fn bold_inside_a_word_uses_function_syntax() {
        let mut d = Doc::new();
        d.push(ParaKind::Body, vec![Run::plain("un"), Run::styled("believ", bold()), Run::plain("able")]);
        let t = d.to_typst();
        assert_eq!(t, "un#strong[believ]able");
        assert_eq!(Doc::from_markup(&t).paras[0].runs, d.paras[0].runs);
    }

    #[test]
    fn serde_defaults_load_minimal_json() {
        let d: Doc = serde_json::from_str(r#"{"paras":[{"id":1,"kind":"Body","runs":[{"Text":{"text":"hi"}}]}]}"#).unwrap();
        assert_eq!(d.paras[0].plain(), "hi");
    }
}
