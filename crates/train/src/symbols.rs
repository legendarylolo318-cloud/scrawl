//! Class list. `symbols.tsv`: `label<TAB>comma-separated HASYv2 latex names`
//! (empty second column = synthetic-only class).

pub struct Class {
    pub label: String,
    pub hasy: Vec<String>,
}

pub fn load() -> Vec<Class> {
    include_str!("symbols.tsv")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (a, b) = l.split_once('\t').unwrap_or((l, ""));
            Class { label: a.to_string(), hasy: b.split(',').filter(|s| !s.is_empty()).map(String::from).collect() }
        })
        .collect()
}
