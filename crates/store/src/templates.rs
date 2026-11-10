use crate::model::*;

pub const BUILTIN: [&str; 7] = ["blank", "notes", "lecture notes", "cornell notes", "problem set", "lab report", "formula sheet"];

fn text(m: &str) -> Block {
    Block::Text { markup: m.into() }
}

/// Built-in page templates. `problems` is used by the problem-set template.
pub fn builtin(name: &str, id: &str, date: &str, problems: usize) -> Option<Page> {
    let mut p = Page::new(id, name, date);
    match name {
        "blank" => {
            p.title = "Untitled".into();
            p.paper.kind = "blank".into();
        }
        "notes" => {
            // plain notes: handwriting becomes words, typing goes straight onto the page
            p.title = "Notes".into();
            p.ink_mode = Some("text".into());
        }
        "lecture notes" => {
            p.title = "Lecture".into();
            p.blocks = vec![text("== Topic"), text("== Key ideas"), Block::Callout { kind: CalloutKind::Definition, markup: String::new() }, text("== Examples"), text("== Questions")];
        }
        "cornell notes" => {
            p.title = "Cornell notes".into();
            p.blocks = vec![
                text("#grid(columns: (1fr, 3fr), gutter: 12pt, [*Cues*], [*Notes*])"),
                Block::Rule,
                text("*Summary*"),
            ];
        }
        "problem set" => {
            p.title = "Problem set".into();
            p.paper.kind = "grid".into();
            for i in 1..=problems.clamp(1, 50) {
                p.blocks.push(text(&format!("+ *Problem {i}.*")));
                p.blocks.push(Block::Rule);
            }
        }
        "lab report" => {
            p.title = "Lab report".into();
            p.blocks = vec![text("== Objective"), text("== Apparatus"), text("== Procedure"), text("== Data"), text("== Analysis"), text("== Conclusion")];
        }
        "formula sheet" => {
            p.title = "Formula sheet".into();
            p.paper.kind = "grid".into();
            p.blocks = vec![text("== Formulas"), Block::Callout { kind: CalloutKind::Theorem, markup: String::new() }];
        }
        _ => return None,
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtins_exist_and_problem_set_is_numbered() {
        for n in BUILTIN {
            assert!(builtin(n, "x", "2026-01-01", 3).is_some(), "{n}");
        }
        assert!(builtin("nope", "x", "d", 1).is_none());
        let p = builtin("problem set", "x", "d", 4).unwrap();
        assert_eq!(p.blocks.iter().filter(|b| matches!(b, Block::Text { markup } if markup.contains("Problem"))).count(), 4);
    }
}
