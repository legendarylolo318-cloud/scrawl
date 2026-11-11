//! Markdown (Obsidian flavour) <-> document paragraphs: `[[wikilinks]]`, `#tags`, `- [ ]` tasks,
//! `$math$`, `> [!note]` callouts, `![[image]]`, front matter. The parser never fails: anything it
//! does not understand stays as plain text.

use crate::model::*;
use layout::Expr;

/// Title and tags found in a note's front matter.
#[derive(Debug, Default, PartialEq)]
pub struct MdMeta {
    pub title: Option<String>,
    pub tags: Vec<String>,
}

/// `#tag` words in plain text (not headings, not inside words). Lower case, without the `#`.
pub fn hashtags(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if c != '#' || (i > 0 && !chars[i - 1].is_whitespace() && !"([{".contains(chars[i - 1])) {
            continue;
        }
        let tag: String = chars[i + 1..].iter().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).collect();
        // a tag has a letter in it (so "#1" and "# heading" are not tags)
        if tag.chars().any(char::is_alphabetic) {
            let t = tag.trim_end_matches(['-', '/']).to_lowercase();
            if !t.is_empty() && !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

impl Doc {
    /// Tags typed into the text of this document.
    pub fn hashtags(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.paras {
                let text: String = p
                    .runs
                    .iter()
                    .map(|r| match r {
                        Run::Text { text, style, .. } if !style.code => text.clone(),
                        _ => " ".into(),
                    })
                    .collect();
                for t in hashtags(&text) {
                    if !out.contains(&t) {
                        out.push(t);
                    }
                }
        }
        out
    }
}

// ------------------------------------------------------------------------------------------ writing

fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        // "==" is a highlight mark when read back, so a "=" followed by another one is escaped
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '$' | '|' | '~') || (c == '=' && it.peek() == Some(&'=')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn inline_md(runs: &[Run], link_name: &dyn Fn(&str) -> String) -> String {
    let mut s = String::new();
    for r in runs {
        match r {
            Run::Math { typst } => s += &format!("${typst}$"),
            Run::Text { text, style, link } => {
                let mut b = if style.code && !text.contains('`') { format!("`{text}`") } else { md_escape(text) };
                match link {
                    Some(Link::Page(t)) => {
                        let target = link_name(t);
                        // spaces at the ends of the link text go outside the brackets (they are trimmed when read)
                        let core = text.trim();
                        let (lead, trail) = (&text[..text.len() - text.trim_start().len()], &text[text.trim_end().len()..]);
                        if !core.is_empty() {
                            b = if target == core { format!("{lead}[[{target}]]{trail}") } else { format!("{lead}[[{target}|{core}]]{trail}") };
                        }
                    }
                    Some(Link::Url(u)) => b = format!("[{b}]({u})"),
                    None => {}
                }
                if style.italic {
                    b = format!("*{b}*");
                }
                if style.bold {
                    b = format!("**{b}**");
                }
                if style.underline {
                    b = format!("<u>{b}</u>");
                }
                s += &b;
            }
        }
    }
    s
}

impl Doc {
    /// Markdown for the whole document. `link_name` maps a page title to the file name it is saved under;
    /// `image_name` maps an image's media name to the name used in `![[...]]`.
    pub fn to_markdown(&self, link_name: &dyn Fn(&str) -> String, image_name: &dyn Fn(&str) -> String) -> String {
        let mut out = String::new();
        let mut prev_list = false;
        for p in &self.paras {
            let body = || inline_md(&p.runs, link_name);
            let line = match &p.kind {
                ParaKind::Body => {
                    let b = body();
                    // keep text that merely looks like a list or heading literal
                    let t = b.trim_start();
                    let digits = |n: &str| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit());
                    if t.starts_with(['#', '-', '+', '>']) || t.split_once(". ").is_some_and(|(n, _)| digits(n)) || t.trim_end().strip_suffix('.').is_some_and(digits) {
                        format!("\\{b}")
                    } else {
                        b
                    }
                }
                ParaKind::Heading(l) => format!("{} {}", "#".repeat((*l).clamp(1, 6) as usize), body()),
                ParaKind::Bullet { indent } => format!("{}- {}", "  ".repeat(*indent as usize), body()),
                ParaKind::Numbered { indent } => format!("{}1. {}", "  ".repeat(*indent as usize), body()),
                ParaKind::Check { done } => format!("- [{}] {}", if *done { "x" } else { " " }, body()),
                ParaKind::Quote => format!("> {}", body()),
                ParaKind::Callout(k) => format!("> [!{}] {}", callout_tag(*k), body()),
                ParaKind::MathDisplay { expr, source } => format!("$$\n{}\n$$", if expr.is_empty() { source.clone().unwrap_or_default() } else { expr.to_latex() }),
                ParaKind::Image { file, .. } => format!("![[{}]]", image_name(file)),
                ParaKind::Plot { functions, .. } => format!("*Plot: {}*", functions.join(", ")),
                ParaKind::Rule => "---".into(),
                ParaKind::Markup(src) => {
                    if src.starts_with("```") {
                        src.clone()
                    } else {
                        format!("```typst\n{src}\n```")
                    }
                }
            };
            if !out.is_empty() {
                out += if prev_list && p.kind.is_list() { "\n" } else { "\n\n" };
            }
            out += &line;
            prev_list = p.kind.is_list();
        }
        out.push('\n');
        out
    }
}

fn callout_tag(k: CalloutKind) -> &'static str {
    match k {
        CalloutKind::Definition => "note",
        CalloutKind::Theorem => "tip",
        CalloutKind::Example => "example",
        CalloutKind::Warning => "warning",
    }
}

fn callout_of(tag: &str) -> Option<CalloutKind> {
    Some(match tag.to_lowercase().as_str() {
        "note" | "info" | "abstract" | "summary" | "definition" => CalloutKind::Definition,
        "tip" | "success" | "theorem" | "check" => CalloutKind::Theorem,
        "example" | "quote" | "question" => CalloutKind::Example,
        "warning" | "danger" | "failure" | "bug" | "caution" | "important" => CalloutKind::Warning,
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------------ reading

/// Split `---` front matter from the body.
pub fn front_matter(src: &str) -> (MdMeta, &str) {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut meta = MdMeta::default();
    let Some(rest) = src.strip_prefix("---\n").or_else(|| src.strip_prefix("---\r\n")) else { return (meta, src) };
    let Some(end) = rest.find("\n---") else { return (meta, src) };
    let yaml = &rest[..end];
    // a page that merely starts with a --- rule is not front matter: every line must look like YAML
    // (`- item` lines only count right after a `key:` line without a value)
    let mut list_ok = false;
    for l in yaml.lines() {
        let t = l.trim();
        let key = t.split_once(':').filter(|(k, _)| !k.is_empty() && !k.starts_with('-') && k.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' ' | '.')));
        if let Some((_, v)) = key {
            list_ok = v.trim().is_empty();
        } else if !(t.is_empty() || t.starts_with('#') || (list_ok && t.starts_with("- "))) {
            return (meta, src);
        }
    }
    let after = &rest[end + 4..];
    let body = after.strip_prefix("\r\n").or_else(|| after.strip_prefix('\n')).unwrap_or(after);
    let mut in_tags = false;
    for line in yaml.lines() {
        let t = line.trim_end();
        if let Some(v) = t.strip_prefix("title:") {
            meta.title = Some(v.trim().trim_matches(['"', '\'']).to_string()).filter(|s| !s.is_empty());
            in_tags = false;
        } else if let Some(v) = t.strip_prefix("tags:").or_else(|| t.strip_prefix("tag:")) {
            in_tags = v.trim().is_empty();
            for x in v.trim().trim_matches(['[', ']']).split([',', ' ']) {
                let x = x.trim().trim_matches(['"', '\'', '#']);
                if !x.is_empty() {
                    meta.tags.push(x.to_lowercase());
                }
            }
        } else if in_tags && t.trim_start().starts_with("- ") {
            let x = t.trim_start()[2..].trim().trim_matches(['"', '\'', '#']);
            if !x.is_empty() {
                meta.tags.push(x.to_lowercase());
            }
        } else if !t.starts_with(' ') && !t.starts_with('-') {
            in_tags = false;
        }
    }
    (meta, body)
}

struct Scan<'a> {
    c: &'a [char],
    i: usize,
    /// Last search per pattern: (searched from, found at). Searches only move forward, so remembering
    /// them keeps a long line linear instead of quadratic.
    memo: std::collections::HashMap<&'static str, (usize, Option<usize>)>,
}

impl Scan<'_> {
    fn at(&self, pat: &str) -> bool {
        (self.i..).zip(pat.chars()).all(|(k, p)| self.c.get(k) == Some(&p))
    }

    /// First position >= `from` where `pat` starts (for "$": a "$" not right after a space).
    fn find(&mut self, from: usize, pat: &'static str) -> Option<usize> {
        if let Some(&(f0, hit)) = self.memo.get(pat) {
            if from >= f0 && hit.is_none_or(|h| from <= h) {
                return hit;
            }
        }
        let p: Vec<char> = pat.chars().collect();
        let c = self.c;
        let hit = (from..c.len().saturating_sub(p.len() - 1)).find(|&j| c[j..j + p.len()] == p[..] && (pat != "$" || j == 0 || !c[j - 1].is_whitespace()) && (pat != "]" || j == 0 || c[j - 1] != '\\'));
        self.memo.insert(pat, (from, hit));
        hit
    }
}

/// End of a Markdown link URL starting at `from`: the `)` that balances, at most 4096 characters on.
fn url_end(c: &[char], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (j, &ch) in c.iter().enumerate().skip(from).take(4096) {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(j),
            ')' => depth -= 1,
            ' ' => return None,
            _ => {}
        }
    }
    None
}

/// Text with Markdown backslash escapes removed.
fn unescape(s: &[char]) -> String {
    let mut out = String::new();
    let mut k = 0;
    while k < s.len() {
        if s[k] == '\\' && k + 1 < s.len() {
            k += 1;
        }
        out.push(s[k]);
        k += 1;
    }
    out
}

fn inline_runs(text: &str) -> Vec<Run> {
    let c: Vec<char> = text.chars().collect();
    let mut sc = Scan { c: &c, i: 0, memo: Default::default() };
    let mut runs: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let mut st = Style::default();
    let flush = |buf: &mut String, runs: &mut Vec<Run>, st: Style| {
        if !buf.is_empty() {
            runs.push(Run::Text { text: std::mem::take(buf), style: st, link: None });
        }
    };
    while sc.i < c.len() {
        let ch = c[sc.i];
        if ch == '\\' && sc.i + 1 < c.len() {
            buf.push(c[sc.i + 1]);
            sc.i += 2;
        } else if sc.at("[[") {
            if let Some(end) = sc.find(sc.i + 2, "]]") {
                let inner: String = c[sc.i + 2..end].iter().collect();
                let (target, label) = inner.split_once('|').map_or((inner.as_str(), inner.as_str()), |(a, b)| (a, b));
                // links to headings or blocks (`Page#Heading`) go to the page
                let page = target.split(['#', '^']).next().unwrap_or(target).trim().to_string();
                flush(&mut buf, &mut runs, st);
                if page.is_empty() {
                    buf.push_str(label);
                } else {
                    runs.push(Run::Text { text: label.trim().to_string(), style: st, link: Some(Link::Page(page)) });
                }
                sc.i = end + 2;
                continue;
            }
            buf.push(ch);
            sc.i += 1;
        } else if ch == '`' {
            if let Some(end) = sc.find(sc.i + 1, "`") {
                flush(&mut buf, &mut runs, st);
                runs.push(Run::Text { text: c[sc.i + 1..end].iter().collect(), style: Style { code: true, ..st }, link: None });
                sc.i = end + 1;
            } else {
                buf.push(ch);
                sc.i += 1;
            }
        } else if ch == '$' && sc.i + 1 < c.len() && c[sc.i + 1] != '$' && !c[sc.i + 1].is_whitespace() {
            match sc.find(sc.i + 1, "$") {
                Some(end) => {
                    flush(&mut buf, &mut runs, st);
                    let latex: String = c[sc.i + 1..end].iter().collect();
                    let typst = if layout::looks_like_latex(&latex) { layout::latex_to_typst(&latex) } else { latex.trim().to_string() };
                    runs.push(Run::Math { typst });
                    sc.i = end + 1;
                }
                None => {
                    buf.push(ch);
                    sc.i += 1;
                }
            }
        } else if ch == '[' {
            // [text](url)
            let close = sc.find(sc.i + 1, "]");
            match close {
                // the URL ends at the ")" that balances its "(" (links like `wiki/A_(b)` are common)
                Some(cl) if c.get(cl + 1) == Some(&'(') => match url_end(&c, cl + 2) {
                    Some(pe) => {
                        flush(&mut buf, &mut runs, st);
                        let label = unescape(&c[sc.i + 1..cl]);
                        let url: String = c[cl + 2..pe].iter().collect();
                        runs.push(Run::Text { text: label, style: st, link: Some(Link::Url(url)) });
                        sc.i = pe + 1;
                    }
                    None => {
                        buf.push(ch);
                        sc.i += 1;
                    }
                },
                _ => {
                    buf.push(ch);
                    sc.i += 1;
                }
            }
        } else if sc.at("**") || sc.at("__") {
            flush(&mut buf, &mut runs, st);
            st.bold = !st.bold;
            sc.i += 2;
        } else if (ch == '*' || ch == '_') && (sc.i == 0 || !c[sc.i - 1].is_alphanumeric() || !c.get(sc.i + 1).is_some_and(|n| n.is_alphanumeric()) || (ch == '*' && (st.italic || sc.find(sc.i + 1, "*").is_some()))) {
            // `*` may start or end emphasis inside a word (`wo*rd*s`), `_` may not (`snake_case`)
            flush(&mut buf, &mut runs, st);
            st.italic = !st.italic;
            sc.i += 1;
        } else if sc.at("<u>") {
            flush(&mut buf, &mut runs, st);
            st.underline = true;
            sc.i += 3;
        } else if sc.at("</u>") {
            flush(&mut buf, &mut runs, st);
            st.underline = false;
            sc.i += 4;
        } else if sc.at("==") || sc.at("~~") {
            sc.i += 2; // highlight and strike-through marks are dropped, the text stays
        } else {
            buf.push(ch);
            sc.i += 1;
        }
    }
    flush(&mut buf, &mut runs, st);
    normalize(runs)
}

fn is_rule(t: &str) -> bool {
    let c: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    c.len() >= 3 && ["-", "*", "_"].iter().any(|m| c.iter().all(|x| m.starts_with(*x)))
}

fn table_markup(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let esc = |s: &str| crate::escape_text(s.trim());
    let mut cells = Vec::new();
    for (ri, r) in rows.iter().enumerate() {
        for ci in 0..cols {
            let t = esc(r.get(ci).map_or("", String::as_str));
            cells.push(if ri == 0 { format!("[*{t}*]") } else { format!("[{t}]") });
        }
    }
    format!("#table(columns: {cols}, {})", cells.join(", "))
}

impl Doc {
    /// Parse Markdown (front matter is skipped; use `front_matter` to read it).
    pub fn from_markdown(src: &str) -> Doc {
        let (_, body) = front_matter(src);
        let lines: Vec<&str> = body.lines().collect();
        let mut d = Doc::new();
        let mut para: Vec<String> = Vec::new();
        let flush = |d: &mut Doc, para: &mut Vec<String>| {
            if !para.is_empty() {
                d.push(ParaKind::Body, inline_runs(&para.join(" ")));
                para.clear();
            }
        };
        let mut i = 0;
        while i < lines.len() {
            let raw = lines[i].trim_end();
            let t = raw.trim_start();
            let indent = ((raw.len() - t.len()) / 2).min(5) as u8;
            if t.is_empty() {
                flush(&mut d, &mut para);
            } else if t.starts_with("```") || t.starts_with("~~~") {
                flush(&mut d, &mut para);
                let fence = &t[..3];
                let mut j = i + 1;
                while j < lines.len() && !lines[j].trim_start().starts_with(fence) {
                    j += 1;
                }
                let end = (j + 1).min(lines.len());
                let block = lines[i..end].join("\n");
                d.push(ParaKind::Markup(if block.trim_end().ends_with("```") || end == j { block } else { format!("{block}\n```") }), vec![]);
                i = end;
                continue;
            } else if let Some(after) = t.strip_prefix("$$") {
                flush(&mut d, &mut para);
                let first = after.trim();
                let (latex, next) = if let Some(one) = first.strip_suffix("$$") {
                    (one.trim().to_string(), i + 1)
                } else {
                    let mut j = i + 1;
                    let mut acc = vec![first.to_string()];
                    while j < lines.len() && !lines[j].trim_end().ends_with("$$") {
                        acc.push(lines[j].to_string());
                        j += 1;
                    }
                    if j < lines.len() {
                        acc.push(lines[j].trim_end().trim_end_matches("$$").to_string());
                    }
                    (acc.join("\n").trim().to_string(), (j + 1).min(lines.len()))
                };
                // Obsidian writes LaTeX; Scrawl's own export writes the Typst source, which must stay as it is
                let typst = if layout::looks_like_latex(&latex) { layout::latex_to_typst(&latex) } else { latex };
                d.push(ParaKind::MathDisplay { expr: Expr::Row(vec![]), source: Some(typst) }, vec![]);
                i = next;
                continue;
            } else if let Some(h) = t.strip_prefix('#').map(|r| (1 + r.chars().take_while(|c| *c == '#').count(), r.trim_start_matches('#'))).filter(|(n, r)| *n <= 6 && (r.is_empty() || r.starts_with(' '))) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Heading(h.0.min(3) as u8), inline_runs(h.1.trim()));
            } else if is_rule(t) && !t.starts_with("- ") && !t.starts_with("* ") {
                flush(&mut d, &mut para);
                d.push(ParaKind::Rule, vec![]);
            } else if t.starts_with('|') && lines.get(i + 1).is_some_and(|n| n.trim_start().starts_with('|') && n.contains("---")) {
                flush(&mut d, &mut para);
                let mut rows: Vec<Vec<String>> = Vec::new();
                let mut j = i;
                while j < lines.len() && lines[j].trim_start().starts_with('|') {
                    if j != i + 1 {
                        rows.push(lines[j].trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect());
                    }
                    j += 1;
                }
                d.push(ParaKind::Markup(table_markup(&rows)), vec![]);
                i = j;
                continue;
            } else if matches!(t, "-" | "*" | "+") {
                flush(&mut d, &mut para);
                d.push(ParaKind::Bullet { indent }, vec![]);
            } else if matches!(t, "- [ ]" | "* [ ]") {
                flush(&mut d, &mut para);
                d.push(ParaKind::Check { done: false }, vec![]);
            } else if matches!(t, "- [x]" | "- [X]" | "* [x]") {
                flush(&mut d, &mut para);
                d.push(ParaKind::Check { done: true }, vec![]);
            } else if let Some(rest) = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("* [ ] ")) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Check { done: false }, inline_runs(rest));
            } else if let Some(rest) = t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")).or_else(|| t.strip_prefix("* [x] ")) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Check { done: true }, inline_runs(rest));
            } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Bullet { indent }, inline_runs(rest));
            } else if t.strip_suffix('.').is_some_and(|n| !n.is_empty() && n.len() < 6 && n.chars().all(|c| c.is_ascii_digit())) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Numbered { indent }, vec![]);
            } else if let Some(rest) = t.split_once(". ").filter(|(n, _)| !n.is_empty() && n.len() < 6 && n.chars().all(|c| c.is_ascii_digit())).map(|(_, r)| r) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Numbered { indent }, inline_runs(rest));
            } else if let Some(q) = t.strip_prefix('>') {
                flush(&mut d, &mut para);
                let q = q.trim_start();
                match q.strip_prefix("[!").and_then(|r| r.split_once(']')) {
                    Some((tag, text)) => {
                        let kind = callout_of(tag.trim_end_matches(['+', '-'])).unwrap_or(CalloutKind::Definition);
                        d.push(ParaKind::Callout(kind), inline_runs(text.trim()));
                    }
                    None => {
                        d.push(ParaKind::Quote, inline_runs(q));
                    }
                }
            } else if let Some(img) = t.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")).map(|r| r.split('|').next().unwrap_or(r).to_string()).or_else(|| t.strip_prefix("![").and_then(|r| r.split_once("](")).and_then(|(_, u)| u.strip_suffix(')')).map(String::from)) {
                flush(&mut d, &mut para);
                d.push(ParaKind::Image { file: img, x: 0.0, y: 0.0, w: 360.0, h: 240.0, crop: None }, vec![]);
            } else {
                // like Obsidian's default, every line is its own paragraph
                para.push(t.to_string());
                flush(&mut d, &mut para);
            }
            i += 1;
        }
        flush(&mut d, &mut para);
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_name(s: &str) -> String {
        s.to_string()
    }

    fn doc() -> Doc {
        Doc::from_markup("= Chain rule\nSee [[Limits]] and *this* with $x^2$ and `code` and \\#calculus.\n\n- one\n  - nested\n- ☐ todo\n- ☑ done\n+ first\n\n#quote(block: true)[A saying]")
    }

    #[test]
    fn markdown_round_trip_keeps_structure() {
        let d = doc();
        let md = d.to_markdown(&plain_name, &plain_name);
        assert!(md.contains("# Chain rule"), "{md}");
        assert!(md.contains("[[Limits]]") && md.contains("*this*") && md.contains("$x^2$") && md.contains("`code`"), "{md}");
        assert!(md.contains("- [ ] todo") && md.contains("- [x] done") && md.contains("  - nested"), "{md}");
        let back = Doc::from_markdown(&md);
        let kinds = |d: &Doc| d.paras.iter().map(|p| std::mem::discriminant(&p.kind)).collect::<Vec<_>>();
        assert_eq!(kinds(&back), kinds(&d), "{md}");
        assert_eq!(back.paras[1].plain(), d.paras[1].plain());
        assert_eq!(back.paras[1].runs.len(), d.paras[1].runs.len());
    }

    #[test]
    fn obsidian_features_import() {
        let md = "---\ntitle: My note\ntags: [math, \"uni/year1\"]\n---\n# Heading\n\nSome ==marked== text with [[Other page|alias]] and [[Page#Section]] and [site](https://example.com).\n\n> [!warning] Careful\n> second line\n\n> plain quote\n\n```rust\nfn main() {}\n```\n\n$$\n\\frac{a}{b}\n$$\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\n![[pic.png]]\n![](folder/other.jpg)\n1. one\n2. two\n";
        let (meta, _) = front_matter(md);
        assert_eq!(meta.title.as_deref(), Some("My note"));
        assert_eq!(meta.tags, vec!["math", "uni/year1"]);
        let d = Doc::from_markdown(md);
        assert!(matches!(d.paras[0].kind, ParaKind::Heading(1)));
        let links: Vec<&Link> = d.paras[1].runs.iter().filter_map(|r| if let Run::Text { link: Some(l), .. } = r { Some(l) } else { None }).collect();
        assert_eq!(links, vec![&Link::Page("Other page".into()), &Link::Page("Page".into()), &Link::Url("https://example.com".into())]);
        assert!(d.paras[1].plain().contains("marked text"));
        assert!(matches!(d.paras[2].kind, ParaKind::Callout(CalloutKind::Warning)));
        assert!(d.paras.iter().any(|p| p.kind == ParaKind::Quote && p.plain() == "plain quote"));
        assert!(d.paras.iter().any(|p| matches!(&p.kind, ParaKind::Markup(s) if s.starts_with("```rust"))));
        assert!(d.paras.iter().any(|p| matches!(&p.kind, ParaKind::MathDisplay { source: Some(s), .. } if s.contains("frac"))));
        assert!(d.paras.iter().any(|p| matches!(&p.kind, ParaKind::Markup(s) if s.starts_with("#table(columns: 2"))));
        assert!(d.paras.iter().any(|p| p.kind == ParaKind::Rule));
        let imgs: Vec<&str> = d.paras.iter().filter_map(|p| if let ParaKind::Image { file, .. } = &p.kind { Some(file.as_str()) } else { None }).collect();
        assert_eq!(imgs, vec!["pic.png", "folder/other.jpg"]);
        assert_eq!(d.paras.iter().filter(|p| matches!(p.kind, ParaKind::Numbered { .. })).count(), 2);
        // and it all still converts to Typst (page links stay as [[Title]], which the app understands)
        assert!(d.to_typst().contains("[[Other page]]") && !d.to_typst().contains("!["));
    }

    #[test]
    fn hashtags_are_found_but_headings_and_numbers_are_not() {
        assert_eq!(hashtags("a #Calculus b #uni/year-1. #1 # heading c#no (#yes)"), vec!["calculus", "uni/year-1", "yes"]);
        assert_eq!(doc().hashtags(), vec!["calculus"]);
    }

    #[test]
    fn plain_text_that_looks_like_markdown_survives_a_round_trip() {
        let mut d = Doc::new();
        for t in ["# not a heading", "1. not numbered", "- not a bullet", "a*b* _c_ [d] $e", "> not a quote"] {
            d.push(ParaKind::Body, vec![Run::plain(t)]);
        }
        let back = Doc::from_markdown(&d.to_markdown(&plain_name, &plain_name));
        assert_eq!(back.paras.len(), 5);
        for (a, b) in d.paras.iter().zip(&back.paras) {
            assert_eq!(a.kind, b.kind, "{a:?} vs {b:?}");
            assert_eq!(a.plain(), b.plain());
        }
    }

    #[test]
    fn hostile_or_broken_markdown_never_panics() {
        for src in ["[[", "[[]]", "**", "$", "$$", "```", "| a |\n|---", "![[", "![](", "---\ntitle", "> [!", "\u{0}\u{1}", "[a](", "`", "*_*_"] {
            let d = Doc::from_markdown(src);
            let _ = d.to_typst();
            let _ = d.to_markdown(&plain_name, &plain_name);
        }
    }
}

#[cfg(test)]
mod line_tests {
    use super::*;

    #[test]
    fn empty_list_items_and_single_newlines_like_obsidian() {
        let d = Doc::from_markdown("**Date:** \n**People:**\n\n- \n- [ ]\n1.\n- [ ] real");
        let kinds: Vec<&ParaKind> = d.paras.iter().map(|p| &p.kind).collect();
        assert_eq!(kinds, vec![&ParaKind::Body, &ParaKind::Body, &ParaKind::Bullet { indent: 0 }, &ParaKind::Check { done: false }, &ParaKind::Numbered { indent: 0 }, &ParaKind::Check { done: false }]);
        assert_eq!(d.paras[0].plain(), "Date:");
        assert_eq!(d.paras[5].plain(), "real");
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;

    fn name(s: &str) -> String {
        s.to_string()
    }

    fn round(d: &Doc) -> Doc {
        Doc::from_markdown(&d.to_markdown(&name, &name))
    }

    #[test]
    fn body_text_survives_markdown_round_trips() {
        // "==" was dropped (taken for a highlight mark) and "2024." became an empty numbered item
        for t in ["a == b", "==", "x === y", "2024.", "1.", "1.\t", "7. ", "12.5", "x ~~ y", "a_ b", "C# and #1", "x | y", "50% * 3", "[x](y)", "a]] b", "  - x", "\\", "a\\b"] {
            let mut d = Doc::new();
            d.push(ParaKind::Body, vec![Run::plain(t)]);
            let back = round(&d);
            assert_eq!(back.paras.len(), 1, "{t:?}");
            assert_eq!(back.paras[0].kind, ParaKind::Body, "{t:?}");
            assert_eq!(back.paras[0].plain().trim_end(), t.trim_end());
        }
    }

    #[test]
    fn url_links_and_typst_display_math_survive_round_trips() {
        let mut d = Doc::new();
        d.push(ParaKind::Body, vec![Run::Text { text: "a_b *c*".into(), style: Style::default(), link: Some(Link::Url("https://x.org/a_(b)".into())) }]);
        d.push(ParaKind::MathDisplay { expr: Expr::Row(vec![]), source: Some("sum_(i=1)^n frac(a, b)".into()) }, vec![]);
        let back = round(&d);
        assert_eq!(back.paras[0].runs, d.paras[0].runs);
        assert_eq!(back.paras[1].kind, d.paras[1].kind);
        // LaTeX from Obsidian is still converted
        let o = Doc::from_markdown("$$\n\\frac{a}{b}\n$$");
        assert!(matches!(&o.paras[0].kind, ParaKind::MathDisplay { source: Some(s), .. } if s.contains("frac") && !s.contains('\\')));
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_front_matter_or_headings() {
        let (m, body) = front_matter("\u{feff}---\ntitle: T\n---\n# H");
        assert_eq!(m.title.as_deref(), Some("T"));
        assert_eq!(body, "# H");
        let d = Doc::from_markdown("\u{feff}# Heading");
        assert!(matches!(d.paras[0].kind, ParaKind::Heading(1)), "{:?}", d.paras[0]);
        assert_eq!(d.paras[0].plain(), "Heading");
    }

    #[test]
    fn long_lines_parse_in_linear_time() {
        // a 100 000 character line took 50 s (every character searched the rest of the line)
        for line in ["word ".repeat(40_000), "[[a ".repeat(40_000), "`a [b ".repeat(30_000), "$a [x](y ".repeat(20_000)] {
            let t = std::time::Instant::now();
            let d = Doc::from_markdown(&line);
            assert!(!d.paras.is_empty());
            assert!(t.elapsed().as_millis() < 1500, "{:?} for {}", t.elapsed(), &line[..8]);
        }
    }
}

#[cfg(test)]
mod empty_heading_tests {
    use super::*;

    #[test]
    fn an_empty_heading_is_not_read_back_as_hashes() {
        let mut d = Doc::new();
        d.push(ParaKind::Heading(3), vec![]);
        d.push(ParaKind::Body, vec![Run::plain("after")]);
        let back = Doc::from_markdown(&d.to_markdown(&|s| s.to_string(), &|s| s.to_string()));
        assert_eq!(back.paras[0].kind, ParaKind::Heading(3));
        assert_eq!(back.paras[0].plain(), "");
        assert!(matches!(Doc::from_markdown("#hashtag").paras[0].kind, ParaKind::Body));
    }
}

#[cfg(test)]
mod link_space_tests {
    use super::*;

    #[test]
    fn spaces_at_the_ends_of_link_text_are_kept() {
        let mut d = Doc::new();
        d.push(ParaKind::Body, vec![Run::plain("x"), Run::Text { text: " pag ".into(), style: Style::default(), link: Some(Link::Page("Linked page".into())) }, Run::plain("rt")]);
        let back = Doc::from_markdown(&d.to_markdown(&|s| s.to_string(), &|s| s.to_string()));
        assert_eq!(back.paras[0].plain(), "x pag rt");
    }
}

#[cfg(test)]
mod intraword_tests {
    use super::*;

    #[test]
    fn emphasis_inside_a_word_round_trips_and_lone_stars_stay() {
        let mut d = Doc::new();
        d.push(ParaKind::Body, vec![Run::plain("wo"), Run::styled("rd", Style { italic: true, ..Style::default() }), Run::plain("s")]);
        let back = Doc::from_markdown(&d.to_markdown(&|s| s.to_string(), &|s| s.to_string()));
        assert_eq!(back.paras[0].runs, d.paras[0].runs);
        assert_eq!(Doc::from_markdown("5*3 = 15").paras[0].plain(), "5*3 = 15");
        assert_eq!(Doc::from_markdown("snake_case_name").paras[0].plain(), "snake_case_name");
    }
}

#[cfg(test)]
mod rule_first_tests {
    use super::*;

    #[test]
    fn a_note_starting_with_a_rule_keeps_its_text() {
        // the text between two rules was taken for front matter and dropped
        let md = "---\n\nImportant text\n\n---\n\nmore";
        let d = Doc::from_markdown(md);
        assert!(d.plain_text().contains("Important text"), "{:?}", d.plain_text());
        assert_eq!(d.paras.iter().filter(|p| p.kind == ParaKind::Rule).count(), 2);
        // real front matter still works, with lists and comments
        let (m, body) = front_matter("---\ntitle: T\ntags:\n  - a\n# comment\naliases: [x]\n---\nbody");
        assert_eq!(m.title.as_deref(), Some("T"));
        assert_eq!(body, "body");
    }
}
