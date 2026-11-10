use crate::imgops::{normalize, SIZE};
use crate::symbols::Class;
use crate::synth::{render_class, Fonts};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub const SYNTH_PER_CLASS: usize = 700;
pub const SYNTH_TEST_PER_CLASS: usize = 60;

pub struct Set {
    pub x: Vec<Vec<f32>>,
    pub y: Vec<usize>,
}

fn load_fold(root: &Path, split: &str, classes: &[Class]) -> Result<Set> {
    let mut map: HashMap<&str, usize> = HashMap::new();
    for (i, c) in classes.iter().enumerate() {
        for l in &c.hasy {
            map.insert(l.as_str(), i);
        }
    }
    let csv_path = root.join("classification-task/fold-1").join(format!("{split}.csv"));
    let text = std::fs::read_to_string(&csv_path).with_context(|| format!("reading {csv_path:?}"))?;
    let mut set = Set { x: vec![], y: vec![] };
    for line in text.lines().skip(1) {
        // path,symbol_id,latex,user_id ; latex may contain commas? (it does not in HASYv2)
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 4 {
            continue;
        }
        let latex = f[2..f.len() - 1].join(",");
        let Some(&cls) = map.get(latex.as_str()) else { continue };
        let p = csv_path.parent().unwrap().join(f[0]);
        let img = image::open(&p).with_context(|| format!("{p:?}"))?.to_luma8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let ink: Vec<f32> = img.pixels().map(|p| 1.0 - p.0[0] as f32 / 255.0).collect();
        set.x.push(normalize(&ink, w, h));
        set.y.push(cls);
    }
    debug_assert!(set.x.iter().all(|v| v.len() == SIZE * SIZE));
    Ok(set)
}

/// Returns (train, hasy_test, synthetic_test).
pub fn build(root: &Path, classes: &[Class]) -> Result<(Set, Set, Set)> {
    let mut train = load_fold(root, "train", classes)?;
    let test = load_fold(root, "test", classes)?;
    let fonts = Fonts::load();
    let mut synth_test = Set { x: vec![], y: vec![] };
    for (i, c) in classes.iter().enumerate().filter(|(_, c)| c.hasy.is_empty()) {
        let seed = c.label.chars().next().unwrap() as u64;
        let tr = render_class(&fonts, &c.label, SYNTH_PER_CLASS, seed)
            .with_context(|| format!("no installed font has a glyph for {:?}", c.label))?;
        for s in tr {
            train.x.push(s);
            train.y.push(i);
        }
        for s in render_class(&fonts, &c.label, SYNTH_TEST_PER_CLASS, seed + 1).unwrap() {
            synth_test.x.push(s);
            synth_test.y.push(i);
        }
    }
    Ok((train, test, synth_test))
}
