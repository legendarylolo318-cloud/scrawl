//! Handwriting -> plain text for the Text ink mode. The recognizer classifies single symbols, so
//! words are rebuilt here: symbols are sorted into lines, split into words at wide gaps, mapped from
//! math look-alikes back to letters (η→n, √→r, ∂→d …), and each word is matched against the English
//! dictionary using the top guesses for every letter. Works best with letters printed separately;
//! joined-up cursive is often one blob and comes out wrong.

use crate::pagedoc::Speller;
use crate::session::SymbolRec;

/// Map a recognizer label to the text character it most likely is in ordinary writing.
fn as_text(label: &str) -> Option<char> {
    let c = label.chars().next()?;
    if label.chars().count() == 1 && (c.is_ascii_alphanumeric() || ".,;:!?'-()&/\"".contains(c)) {
        return Some(c);
    }
    Some(match label {
        "η" | "∩" => 'n',
        "√" => 'r',
        "∂" | "δ" => 'd',
        "ν" | "∨" => 'v',
        "ι" => 'i',
        "κ" | "ϰ" => 'k',
        "ρ" | "ϱ" => 'p',
        "τ" => 't',
        "∪" | "υ" => 'u',
        "ω" => 'w',
        "ℓ" | "|" => 'l',
        "×" => 'x',
        "ε" | "ϵ" | "∈" => 'e',
        "α" | "∝" => 'a',
        "β" => 'B',
        "γ" => 'y',
        "σ" | "°" | "∘" => 'o',
        "χ" => 'x',
        "μ" => 'u',
        "∫" => 'f',
        "ς" => 'c',
        "⊂" | "∁" => 'c',
        "∖" => '\\',
        "−" => '-',
        "′" => '\'',
        _ => return None,
    })
}

/// Per-symbol options as (char, probability), best first, letters and digits only.
fn options(s: &SymbolRec) -> Vec<(char, f32)> {
    let mut v: Vec<(char, f32)> = Vec::new();
    for c in &s.cands {
        if let Some(ch) = as_text(&c.label) {
            match v.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(&ch)) {
                Some(e) => e.1 += c.score,
                None => v.push((ch, c.score.max(1e-4))),
            }
        }
    }
    if v.is_empty() {
        if let Some(c) = s.label.chars().next() {
            v.push((c, 0.01));
        }
    }
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v.truncate(4);
    v
}

/// Choose the word: the most likely letter combination that is a dictionary word, else the raw
/// best guess (spell-corrected when the dictionary has a close suggestion).
/// The word as ranked guesses, best first: dictionary words from the letter beam, then the raw reading.
fn word_options(opts: &[Vec<(char, f32)>], speller: Option<&Speller>) -> Vec<String> {
    let raw: String = opts.iter().map(|o| o[0].0).collect();
    let Some(sp) = speller else { return vec![raw] };
    if opts.is_empty() || !opts.iter().all(|o| o.iter().any(|c| c.0.is_alphabetic())) {
        return vec![raw]; // numbers and symbols are left alone
    }
    // beam search over letter choices, scored by the product of probabilities
    let mut beam: Vec<(String, f32)> = vec![(String::new(), 1.0)];
    for o in opts {
        let mut next = Vec::new();
        for (s, p) in &beam {
            for (c, q) in o {
                next.push((format!("{s}{c}"), p * q));
            }
        }
        next.sort_by(|a, b| b.1.total_cmp(&a.1));
        next.truncate(256);
        beam = next;
    }
    let mut out: Vec<String> = Vec::new();
    let push = |s: String, out: &mut Vec<String>| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for (cand, _) in &beam {
        let lower = cand.to_lowercase();
        if sp.is_word(&lower) || sp.is_word(cand) {
            push(case_like(&lower, &raw), &mut out);
            if out.len() >= 4 {
                break;
            }
        }
    }
    let lower = raw.to_lowercase();
    if let Some(s) = sp.suggest_one(&lower) {
        if s.chars().count().abs_diff(lower.chars().count()) <= 1 && edit_distance(&s.to_lowercase(), &lower) <= 2 {
            push(case_like(&s, &raw), &mut out);
        }
    }
    push(lower, &mut out);
    out
}

/// Keep a capital first letter when the writer clearly made one; the rest lower case (the
/// recognizer often confuses c/C, o/O, s/S, x/X).
fn case_like(word: &str, raw: &str) -> String {
    let mut out = word.to_lowercase();
    if raw.chars().next().is_some_and(|c| c.is_uppercase()) && !"COSXVWZPUK".contains(raw.chars().next().unwrap_or('a')) {
        let mut cs = out.chars();
        if let Some(f) = cs.next() {
            out = f.to_uppercase().collect::<String>() + cs.as_str();
        }
    }
    out
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Turn recognized symbols into lines of words.
#[cfg(test)]
pub fn to_text(symbols: &[SymbolRec], speller: Option<&Speller>) -> String {
    to_text_alts(symbols, speller).0
}

/// The text plus up to five whole-text alternatives (one word swapped for its next-best reading).
pub fn to_text_alts(symbols: &[SymbolRec], speller: Option<&Speller>) -> (String, Vec<String>) {
    if symbols.is_empty() {
        return (String::new(), vec![]);
    }
    let lay = word_layout(symbols);
    let mut line_words: Vec<Vec<Vec<String>>> = Vec::new();
    for line in &lay.lines {
        let ranked: Vec<Vec<String>> = line.iter().map(|w| word_options(&w.iter().map(|&i| letter_options(&symbols[i], lay.dotted.contains(&i))).collect::<Vec<_>>(), speller)).collect();
        line_words.push(ranked);
    }
    let render = |pick: &dyn Fn(usize, usize) -> usize| -> String {
        let mut n = 0;
        line_words.iter().map(|l| l.iter().map(|w| { let i = pick(n, w.len()); n += 1; w[i.min(w.len() - 1)].clone() }).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n")
    };
    let best = render(&|_, _| 0);
    let total: usize = line_words.iter().map(Vec::len).sum();
    let mut alts: Vec<String> = Vec::new();
    for rank in 1..3 {
        for w in 0..total {
            let t = render(&|n, len| if n == w && rank < len { rank } else { 0 });
            if t != best && !alts.contains(&t) {
                alts.push(t);
            }
        }
    }
    alts.truncate(5);
    (best, alts)
}

/// Symbols in reading order: lines of words of symbol indices. Small dots written just above another
/// symbol (the dot of an i or j) are folded into it and listed in `dotted`.
pub struct WordLayout {
    pub lines: Vec<Vec<Vec<usize>>>,
    pub dotted: Vec<usize>,
}

pub fn word_layout(symbols: &[SymbolRec]) -> WordLayout {
    let mut lay = WordLayout { lines: vec![], dotted: vec![] };
    if symbols.is_empty() {
        return lay;
    }
    let mut heights: Vec<f32> = symbols.iter().map(|s| s.rect.height().max(4.0)).collect();
    heights.sort_by(f32::total_cmp);
    let h = heights[heights.len() / 2];
    // dots of i and j: tiny, just above a taller symbol that they sit over
    let mut is_dot = vec![false; symbols.len()];
    for (d, s) in symbols.iter().enumerate() {
        let small = s.rect.width() < h * 0.35 && s.rect.height() < h * 0.35;
        if !small {
            continue;
        }
        let cx = (s.rect.x0 + s.rect.x1) / 2.0;
        let base = symbols.iter().enumerate().filter(|&(b, t)| {
            b != d && t.rect.height() > s.rect.height() * 1.8 && cx >= t.rect.x0 - h * 0.25 && cx <= t.rect.x1 + h * 0.25 && s.rect.y1 <= t.rect.y0 + h * 0.2 && s.rect.y0 >= t.rect.y0 - h * 0.9
        }).min_by(|a, b| (a.1.rect.y0 - s.rect.y1).abs().total_cmp(&(b.1.rect.y0 - s.rect.y1).abs()));
        if let Some((b, _)) = base {
            is_dot[d] = true;
            lay.dotted.push(b);
        }
    }
    // lines: by vertical centre
    let mut idx: Vec<usize> = (0..symbols.len()).filter(|&i| !is_dot[i]).collect();
    let cy = |i: usize| (symbols[i].rect.y0 + symbols[i].rect.y1) / 2.0;
    idx.sort_by(|&a, &b| cy(a).total_cmp(&cy(b)));
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for i in idx {
        match lines.last_mut() {
            Some(l) if (cy(i) - l.iter().map(|&j| cy(j)).sum::<f32>() / l.len() as f32).abs() < h * 0.9 => l.push(i),
            _ => lines.push(vec![i]),
        }
    }
    for mut line in lines {
        line.sort_by(|&a, &b| symbols[a].rect.x0.total_cmp(&symbols[b].rect.x0));
        let mut words: Vec<Vec<usize>> = vec![vec![]];
        let mut last_x1: Option<f32> = None;
        for &i in &line {
            let r = symbols[i].rect;
            if let Some(x1) = last_x1 {
                if r.x0 - x1 > h * 0.55 {
                    words.push(vec![]);
                }
            }
            last_x1 = Some(last_x1.map_or(r.x1, |x| x.max(r.x1)));
            words.last_mut().unwrap().push(i);
        }
        words.retain(|w| !w.is_empty());
        lay.lines.push(words);
    }
    lay
}

/// Letter options for one symbol; a dot above makes it an i (or a j when that was a guess).
fn letter_options(s: &SymbolRec, dotted: bool) -> Vec<(char, f32)> {
    let base = options(s);
    if !dotted {
        return base;
    }
    let j = base.iter().any(|(c, _)| c.eq_ignore_ascii_case(&'j'));
    let mut v = if j { vec![('j', 0.6), ('i', 0.35)] } else { vec![('i', 0.8), ('j', 0.1)] };
    v.extend(base.into_iter().filter(|(c, _)| !c.eq_ignore_ascii_case(&'i') && !c.eq_ignore_ascii_case(&'j')).map(|(c, p)| (c, p * 0.1)));
    v.truncate(4);
    v
}

/// Which symbol each letter of a chosen reading belongs to, for words whose length matches the ink.
/// Used to learn from corrections.
pub fn letters_for(symbols: &[SymbolRec], text: &str) -> Vec<(usize, char)> {
    let lay = word_layout(symbols);
    let mut out = Vec::new();
    for (line, tl) in lay.lines.iter().zip(text.split('\n')) {
        for (word, tw) in line.iter().zip(tl.split_whitespace()) {
            if word.len() == tw.chars().count() {
                out.extend(word.iter().copied().zip(tw.chars()));
            }
        }
    }
    out
}

/// The letter a symbol currently reads as in text (its best text option).
pub fn reads_as(s: &SymbolRec) -> Option<char> {
    options(s).first().map(|o| o.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink::Rect;
    use recog::Candidate;

    fn sym(x: f32, cands: &[(&str, f32)]) -> SymbolRec {
        SymbolRec {
            stroke_ids: vec![],
            rect: Rect { x0: x, y0: 0.0, x1: x + 20.0, y1: 30.0 },
            label: cands[0].0.into(),
            cands: cands.iter().map(|(l, s)| Candidate { label: l.to_string(), score: *s }).collect(),
        }
    }

    #[test]
    fn look_alikes_and_dictionary_fix_the_screenshot_case() {
        // what the math recognizer made of a handwritten "brandy"
        let s = vec![
            sym(0.0, &[("b", 0.8), ("6", 0.1)]),
            sym(24.0, &[("√", 0.6), ("r", 0.3)]),
            sym(48.0, &[("α", 0.5), ("a", 0.4)]),
            sym(72.0, &[("η", 0.7), ("n", 0.2)]),
            sym(96.0, &[("d", 0.6), ("∂", 0.3)]),
            sym(120.0, &[(">", 0.4), ("y", 0.3), ("γ", 0.2)]),
        ];
        let sp = Speller::new();
        assert_eq!(to_text(&s, sp.as_ref()), "brandy");
    }

    #[test]
    fn gaps_split_words_and_rows_split_lines() {
        let mut s = vec![sym(0.0, &[("h", 0.9)]), sym(24.0, &[("i", 0.9)]), sym(100.0, &[("y", 0.9)]), sym(124.0, &[("o", 0.9)]), sym(148.0, &[("u", 0.9)])];
        let mut second = sym(0.0, &[("o", 0.9)]);
        second.rect.y0 = 80.0;
        second.rect.y1 = 110.0;
        let mut k = sym(24.0, &[("k", 0.9)]);
        k.rect.y0 = 80.0;
        k.rect.y1 = 110.0;
        s.push(second);
        s.push(k);
        let sp = Speller::new();
        assert_eq!(to_text(&s, sp.as_ref()), "hi you\nok");
    }

    #[test]
    fn doubtful_words_offer_whole_text_alternatives() {
        let s = vec![sym(0.0, &[("c", 0.5), ("e", 0.45)]), sym(24.0, &[("a", 0.9)]), sym(48.0, &[("t", 0.9)])];
        let (best, alts) = to_text_alts(&s, Speller::new().as_ref());
        assert_eq!(best, "cat");
        assert!(alts.contains(&"eat".to_string()), "{alts:?}");
    }

    #[test]
    fn the_dot_of_an_i_joins_its_stem() {
        // "hi": h, a stem the recognizer reads as l or 1, and a separate dot above the stem
        let h = sym(0.0, &[("h", 0.9)]);
        let stem = sym(24.0, &[("l", 0.6), ("1", 0.3)]);
        let mut dot = sym(30.0, &[(".", 0.9)]);
        dot.rect = ink::Rect { x0: 31.0, y0: -10.0, x1: 35.0, y1: -6.0 };
        let s = vec![h, stem, dot];
        let (best, _) = to_text_alts(&s, Speller::new().as_ref());
        assert_eq!(best, "hi");
        assert_eq!(letters_for(&s, "hi"), vec![(0, 'h'), (1, 'i')]);
    }

    #[test]
    fn letters_map_back_to_symbols_only_when_lengths_match() {
        let s = vec![sym(0.0, &[("c", 0.9)]), sym(24.0, &[("a", 0.9)]), sym(48.0, &[("t", 0.9)]), sym(120.0, &[("o", 0.9)]), sym(144.0, &[("k", 0.9)])];
        assert_eq!(letters_for(&s, "eat ok"), vec![(0, 'e'), (1, 'a'), (2, 't'), (3, 'o'), (4, 'k')]);
        assert_eq!(letters_for(&s, "cart ok"), vec![(3, 'o'), (4, 'k')], "a word that changed length teaches nothing");
    }

    #[test]
    fn numbers_are_not_spell_corrected() {
        let s = vec![sym(0.0, &[("4", 0.9)]), sym(24.0, &[("2", 0.9)])];
        assert_eq!(to_text(&s, Speller::new().as_ref()), "42");
    }
}
