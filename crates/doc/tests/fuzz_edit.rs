//! Random editing sessions (seeded, so failures reproduce): the document must never panic, never become
//! empty, keep every returned caret valid, and survive Typst and Markdown round trips without changing again.

use layout::Expr;
use notedoc::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const PIECES: &[&str] = &["a", "word ", "É", "e\u{301}", "👩‍🔬", "日本", "שלום", " ", "[[", "]]", "$", "x$", "*", "#tag", "- ", "1. ", "\\", "\t", "`", "<u>", "==", "Page", "[", "]"];

fn valid(doc: &Doc, p: Pos) -> bool {
    clamp(doc, p) == p
}

fn len_of(doc: &Doc, para: usize) -> usize {
    doc.paras.get(para).map_or(0, |p| if p.kind.is_text() { p.text_len() } else { 1 })
}

fn random_pos(r: &mut Rng, doc: &Doc) -> Pos {
    let para = r.below(doc.paras.len());
    Pos { para, off: r.below(len_of(doc, para) + 1) }
}

#[test]
fn random_editing_sessions_keep_the_document_valid() {
    for seed in 1..=300u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = Doc::from_markup("Start");
        let mut sel = Sel::caret(Pos { para: 0, off: 5 });
        let mut log: Vec<String> = Vec::new();
        for step in 0..120 {
            let op = r.below(17);
            log.push(format!("{op}@{sel:?}"));
            let before = doc.clone();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut d = before.clone();
                let mut s = sel;
                let (a, b) = s.ordered();
                let caret = match op {
                    0..=2 => {
                        let t = PIECES[r.below(PIECES.len())];
                        let st = style_at(&d, a);
                        Some(insert_text(&mut d, s, t, st))
                    }
                    3 => Some(enter(&mut d, s.head)),
                    4 => Some(backspace(&mut d, s)),
                    5 => Some(delete_forward(&mut d, s)),
                    6 => {
                        toggle_format(&mut d, a, b, [Fmt::Bold, Fmt::Italic, Fmt::Underline, Fmt::Code][r.below(4)]);
                        None
                    }
                    7 => {
                        let kinds = [ParaKind::Body, ParaKind::Heading(1), ParaKind::Heading(3), ParaKind::Bullet { indent: 0 }, ParaKind::Numbered { indent: 1 }, ParaKind::Check { done: false }, ParaKind::Quote, ParaKind::Callout(CalloutKind::Warning)];
                        toggle_kind(&mut d, a, b, kinds[r.below(kinds.len())].clone());
                        None
                    }
                    8 => {
                        indent(&mut d, a, b, if r.below(2) == 0 { 1 } else { -1 });
                        None
                    }
                    9 => Some(insert_math(&mut d, s, "x^2")),
                    10 => {
                        let objs = [ParaKind::Rule, ParaKind::MathDisplay { expr: Expr::Row(vec![]), source: Some("a/b".into()) }, ParaKind::Markup("#table(columns: 1, [a])".into())];
                        Some(insert_object(&mut d, s.head, objs[r.below(objs.len())].clone()))
                    }
                    11 => autoformat_space(&mut d, s.head),
                    12 => autoformat_dollar(&mut d, s.head),
                    13 => autoformat_link(&mut d, s.head),
                    14 => accept_link(&mut d, s.head, "Linked page"),
                    15 => {
                        s = Sel { anchor: random_pos(&mut r, &d), head: random_pos(&mut r, &d) };
                        None
                    }
                    _ => {
                        let para = s.head.para.min(d.paras.len().saturating_sub(1));
                        toggle_check(&mut d, para);
                        None
                    }
                };
                if let Some(c) = caret {
                    s = Sel::caret(c);
                }
                (d, s)
            }));
            let (d, s) = match res {
                Ok(x) => x,
                Err(_) => panic!("seed {seed} step {step}: panic in op {op}\nlog: {log:?}\ndoc: {:?}", before.to_typst()),
            };
            doc = d;
            sel = s;
            assert!(!doc.paras.is_empty(), "seed {seed} step {step}: no paragraphs after op {op}");
            assert!(valid(&doc, sel.head) && valid(&doc, sel.anchor), "seed {seed} step {step}: caret {sel:?} invalid after op {op} in {:?}", doc.to_typst());
            // every paragraph's text kind/runs agree
            for p in &doc.paras {
                assert!(p.kind.is_text() || p.runs.is_empty(), "seed {seed} step {step}: object with text runs");
            }
        }
        // Typst round trip is stable after one pass
        let t1 = doc.to_typst();
        let t2 = Doc::from_markup(&t1).to_typst();
        let t3 = Doc::from_markup(&t2).to_typst();
        assert_eq!(t2, t3, "seed {seed}: Typst round trip not stable\nfirst: {t1:?}");
        let back1 = Doc::from_markup(&t1);
        // Typst page links are `[[target]]`: a link whose text was edited shows its target there (known limit)
        let mut doc_t = doc.clone();
        for p in doc_t.paras.iter_mut() {
            for r in p.runs.iter_mut() {
                if let Run::Text { text, link: Some(Link::Page(t)), .. } = r {
                    *text = t.clone();
                }
            }
        }
        let nonempty = |d: &Doc| d.paras.iter().filter(|p| (p.kind.is_text() && !p.plain().trim().is_empty()) || matches!(p.kind, ParaKind::Markup(_))).map(|p| format!("{:?}: {:?}", if matches!(p.kind, ParaKind::Callout(_)) { ParaKind::Body } else { p.kind.clone() }, p.plain().trim().to_string())).collect::<Vec<_>>();
        assert_eq!(nonempty(&back1), nonempty(&doc_t), "seed {seed}: Typst round trip changed paragraphs\n{t1:?}");

        // Markdown round trip keeps the text
        let id = |s: &str| s.to_string();
        let md = doc.to_markdown(&id, &id);
        let back = Doc::from_markdown(&md);
        let squash = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(squash(back.plain_text()), squash(doc.plain_text()), "seed {seed}: Markdown round trip changed the text\n{md}");
    }
}
