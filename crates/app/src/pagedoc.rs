//! The typeset side of a page: rendering each block with Typst (cached by content), the rich-text
//! formatting buttons, outline, `[[links]]`, word count, find/replace and spell checking.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use store::{Block, CalloutKind, Page};

pub fn kind(b: &Block) -> &'static str {
    match b {
        Block::Math { .. } => "math",
        Block::Text { .. } => "text",
        Block::Callout { .. } => "callout",
        Block::Image { .. } => "image",
        Block::Plot { .. } => "plot",
        Block::Rule => "rule",
    }
}

/// Editable text of a block (what the editor box shows when you press Edit).
pub fn edit_text(b: &Block) -> String {
    match b {
        Block::Text { markup } | Block::Callout { markup, .. } => markup.clone(),
        Block::Math { expr, source } => source.clone().filter(|_| expr.is_empty()).unwrap_or_else(|| expr.to_typst()),
        Block::Plot { functions, .. } => functions.join("\n"),
        _ => String::new(),
    }
}

/// `[[Title]]` links shown underlined in the rendered page.
fn show_links(markup: &str) -> String {
    let mut out = String::new();
    let mut rest = markup;
    while let Some(i) = rest.find("[[") {
        let Some(j) = rest[i + 2..].find("]]") else { break };
        out += &rest[..i];
        out += &format!("#underline[{}]", &rest[i + 2..i + 2 + j]);
        rest = &rest[i + 2 + j + 2..];
    }
    out + rest
}

#[derive(Clone)]
pub enum Render {
    Ok(Arc<render::Rendered>),
    Err(String),
}

/// Content-addressed cache of block renders: unchanged blocks are never re-rendered.
#[derive(Default)]
pub struct BlockCache {
    map: HashMap<u64, Render>,
    pub misses: usize,
}

fn rgba_of(png: &[u8]) -> Option<render::Rendered> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut px = img.into_raw();
    for p in px.chunks_mut(4) {
        let a = p[3] as u16;
        for c in &mut p[..3] {
            *c = ((*c as u16 * a) / 255) as u8;
        }
    }
    Some(render::Rendered { width: w, height: h, rgba_premul: px })
}

impl BlockCache {
    /// Render one block `width_px` logical pixels wide at `scale` device pixels per logical pixel.
    pub fn render(&mut self, b: &Block, media: &[(String, Vec<u8>)], width_px: f32, scale: f32, color: [u8; 3]) -> Render {
        let width_pt = (width_px * 0.75).max(60.0);
        let fill = format!("rgb({}, {}, {})", color[0], color[1], color[2]);
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let body = match b {
            Block::Text { markup } => show_links(markup),
            Block::Callout { kind, markup } => {
                let (name, tint) = match kind {
                    CalloutKind::Definition => ("Definition", "#7aa2f7"),
                    CalloutKind::Theorem => ("Theorem", "#9ece6a"),
                    CalloutKind::Example => ("Example", "#e0af68"),
                    CalloutKind::Warning => ("Warning", "#f7768e"),
                };
                format!("#block(stroke: (left: 3pt + rgb(\"{tint}\")), inset: (left: 8pt, y: 4pt), width: 100%)[*{name}.* {}]", show_links(markup))
            }
            Block::Image { file, .. } => {
                match media.iter().find(|(n, _)| n == file) {
                    Some(m) => files.push(m.clone()),
                    None => return Render::Err(format!("image {file} is missing")),
                }
                // shown at most as wide as the panel
                let mut b2 = b.clone();
                if let Block::Image { w, h, .. } = &mut b2 {
                    if *w > width_pt - 8.0 {
                        *h *= (width_pt - 8.0) / *w;
                        *w = width_pt - 8.0;
                    }
                }
                store::block_typst(&b2, None)
            }
            Block::Plot { .. } => match export::plot_svg(b) {
                Some(svg) => {
                    files.push(("plot.svg".into(), svg.into_bytes()));
                    "#image(\"plot.svg\", width: 100%)".into()
                }
                None => return Render::Err("could not plot these functions".into()),
            },
            other => store::block_typst(other, None),
        };
        let src = format!("#set page(width: {width_pt:.1}pt, height: auto, margin: 4pt, fill: none)\n#set text(size: 11pt, fill: {fill})\n#show math.equation: set text(size: 13pt)\n#set line(stroke: {fill})\n{body}\n");
        let mut hs = std::collections::hash_map::DefaultHasher::new();
        (&src, scale.to_bits()).hash(&mut hs);
        for (n, bytes) in &files {
            (n, bytes.len(), bytes.get(..64)).hash(&mut hs);
        }
        let key = hs.finish();
        if let Some(r) = self.map.get(&key) {
            return r.clone();
        }
        self.misses += 1;
        let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
        let r = match render::compile(&src, &refs) {
            Ok(doc) => match render::png(&doc, 0, scale / 0.75).and_then(|png| rgba_of(&png)) {
                Some(px) => Render::Ok(Arc::new(px)),
                None => Render::Err("empty".into()),
            },
            Err(e) => Render::Err(e),
        };
        if self.map.len() > 600 {
            self.map.clear();
        }
        self.map.insert(key, r.clone());
        r
    }
}

/// Markup the formatting buttons add to the editor text.
pub fn apply_format(text: &str, fmt: &str) -> String {
    let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
    // inline snippets need a space before them, or Typst reads e.g. `*a*_b_` as one unclosed run
    let sp = if text.is_empty() || text.ends_with(char::is_whitespace) { "" } else { " " };
    let line_prefix = |p: &str| -> String {
        // prefix the last line if it has content, otherwise start a new one
        match text.rsplit_once('\n') {
            Some((head, last)) if !last.trim().is_empty() => format!("{head}\n{p}{last}"),
            None if !text.trim().is_empty() => format!("{p}{text}"),
            _ => format!("{text}{p}"),
        }
    };
    match fmt {
        "h1" => line_prefix("= "),
        "h2" => line_prefix("== "),
        "list" => line_prefix("- "),
        "numbered" => line_prefix("+ "),
        "quote" => format!("{text}{sep}#quote(block: true)[Quote]"),
        "bold" => format!("{text}{sp}*bold* "),
        "italic" => format!("{text}{sp}_italic_ "),
        "math" => format!("{text}{sp}$x^2$ "),
        "link" => format!("{text}{sp}[[Page title]] "),
        "code" => format!("{text}{sep}```rust\nfn main() {{}}\n```\n"),
        "table" => format!("{text}{sep}#table(columns: 3, [*A*], [*B*], [*C*], [1], [2], [3])\n"),
        _ => text.to_string(),
    }
}

/// Headings in the page's text blocks: (block index, level, title).
pub fn outline(p: &Page) -> Vec<(usize, usize, String)> {
    let mut v = Vec::new();
    for (i, b) in p.blocks.iter().enumerate() {
        if let Block::Text { markup } = b {
            for line in markup.lines() {
                let t = line.trim_start();
                let lvl = t.chars().take_while(|c| *c == '=').count();
                if lvl > 0 && t[lvl..].starts_with(' ') {
                    v.push((i, lvl, t[lvl..].trim().to_string()));
                }
            }
        }
    }
    v
}

/// Titles linked with `[[Title]]` from this page.
pub fn links(p: &Page) -> Vec<String> {
    let mut v = Vec::new();
    for b in &p.blocks {
        if let Block::Text { markup } | Block::Callout { markup, .. } = b {
            let mut rest = markup.as_str();
            while let Some(i) = rest.find("[[") {
                let Some(j) = rest[i + 2..].find("]]") else { break };
                let t = rest[i + 2..i + 2 + j].trim().to_string();
                if !t.is_empty() && !v.contains(&t) {
                    v.push(t);
                }
                rest = &rest[i + 2 + j + 2..];
            }
        }
    }
    v
}

/// Words in titles and text (math blocks count as one word each).
pub fn word_count(p: &Page) -> usize {
    let words = |s: &str| s.split(|c: char| c.is_whitespace() || matches!(c, '=' | '*' | '_' | '-' | '+' | '#')).filter(|w| w.chars().any(char::is_alphanumeric)).count();
    words(&p.title)
        + p.blocks
            .iter()
            .map(|b| match b {
                Block::Text { markup } | Block::Callout { markup, .. } => words(markup),
                Block::Math { .. } => 1,
                _ => 0,
            })
            .sum::<usize>()
}

/// Count (and optionally replace) case-sensitive matches in the title and text blocks.
pub fn find_replace(p: &mut Page, find: &str, replace: &str, apply: bool) -> usize {
    if find.is_empty() {
        return 0;
    }
    let mut n = p.title.matches(find).count();
    if apply {
        p.title = p.title.replace(find, replace);
    }
    for b in &mut p.blocks {
        if let Block::Text { markup } | Block::Callout { markup, .. } = b {
            n += markup.matches(find).count();
            if apply {
                *markup = markup.replace(find, replace);
            }
        }
    }
    n
}

/// Spell checker over the bundled en_US Hunspell dictionary (SCOWL). Skips math, code, links,
/// markup commands and words with digits or capitals inside (acronyms, variable names).
pub struct Speller {
    dict: spellbook::Dictionary,
}

impl Speller {
    pub fn new() -> Option<Self> {
        let aff = include_str!("../dict/en_US.aff");
        let dic = include_str!("../dict/en_US.dic");
        spellbook::Dictionary::new(aff, dic).ok().map(|dict| Self { dict })
    }

    /// Misspelled words with the first suggestion ("" if none). At most `limit` results.
    pub fn check(&self, markup: &str, limit: usize) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut in_code = false;
        for line in markup.lines() {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
                continue;
            }
            if in_code || line.trim_start().starts_with('#') {
                continue;
            }
            // drop inline math and links
            let mut clean = String::new();
            let mut depth_math = false;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '$' {
                    depth_math = !depth_math;
                    continue;
                }
                if c == '[' && chars.peek() == Some(&'[') {
                    for d in chars.by_ref() {
                        if d == ']' {
                            break;
                        }
                    }
                    continue;
                }
                clean.push(if depth_math { ' ' } else { c });
            }
            for w in clean.split(|c: char| !(c.is_alphabetic() || c == '\'')) {
                let w = w.trim_matches('\'');
                if w.chars().count() < 2 || w.chars().skip(1).any(|c| c.is_uppercase()) || out.iter().any(|(m, _)| m == w) {
                    continue;
                }
                if !self.dict.check(w) {
                    let mut sug = Vec::new();
                    self.dict.suggest(w, &mut sug);
                    out.push((w.to_string(), sug.into_iter().next().unwrap_or_default()));
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }
}

/// Replace one whole-word occurrence pattern everywhere in `text`.
pub fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word == from {
            out.push_str(to);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_alphabetic() || c == '\'' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        let mut p = Page::new("p", "Chain rule notes", "2026-11-10");
        p.blocks = vec![
            Block::Text { markup: "= Intro\nSee [[Limits]] and [[Series]].\n== Proof\nThe derivitive of $f(g(x))$.".into() },
            Block::Math { expr: layout::Expr::sym("x"), source: None },
        ];
        p
    }

    #[test]
    fn outline_links_and_counts() {
        let p = page();
        assert_eq!(outline(&p).iter().map(|o| (o.1, o.2.as_str())).collect::<Vec<_>>(), vec![(1, "Intro"), (2, "Proof")]);
        assert_eq!(links(&p), vec!["Limits", "Series"]);
        assert!(word_count(&p) >= 10);
    }

    #[test]
    fn find_and_replace_in_title_and_text() {
        let mut p = page();
        assert_eq!(find_replace(&mut p, "rule", "law", false), 1);
        assert_eq!(find_replace(&mut p, "rule", "law", true), 1);
        assert_eq!(p.title, "Chain law notes");
    }

    #[test]
    fn spell_check_finds_typos_but_not_math_or_links() {
        let s = Speller::new().expect("dictionary loads");
        let bad = s.check("The derivitive of $sinx$ is [[Cosx page]] and teh rest. NASA x2", 10);
        let words: Vec<&str> = bad.iter().map(|b| b.0.as_str()).collect();
        assert!(words.contains(&"derivitive") && words.contains(&"teh"), "{bad:?}");
        assert!(!words.iter().any(|w| w.contains("sinx") || w.contains("Cosx")), "{bad:?}");
        assert_eq!(bad.iter().find(|b| b.0 == "teh").map(|b| b.1.as_str()), Some("the"));
        assert_eq!(replace_word("teh cat teh", "teh", "the"), "the cat the");
    }

    #[test]
    fn formatting_buttons_produce_typst_that_compiles() {
        let mut t = String::from("Some text");
        for f in ["h1", "list", "numbered", "bold", "italic", "math", "link", "code", "table", "quote", "h2"] {
            t = apply_format(&t, f);
        }
        let mut c = BlockCache::default();
        match c.render(&Block::Text { markup: t.clone() }, &[], 400.0, 1.0, [0, 0, 0]) {
            Render::Ok(r) => assert!(r.width > 10 && r.height > 10),
            Render::Err(e) => panic!("{e}\n{t}"),
        }
        let misses = c.misses;
        let _ = c.render(&Block::Text { markup: t }, &[], 400.0, 1.0, [0, 0, 0]);
        assert_eq!(c.misses, misses, "cached");
    }
}
