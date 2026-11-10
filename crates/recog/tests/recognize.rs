use ink::{InkMode, PenKind, Point, Stroke};
use recog::Classifier;
use std::time::Instant;

fn stroke(pts: &[(f32, f32)]) -> Stroke {
    // densify so the rasterizer sees a continuous line
    let mut out = Vec::new();
    for w in pts.windows(2) {
        for i in 0..12 {
            let t = i as f32 / 12.0;
            out.push(Point::new(w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t, out.len() as u32 * 8));
        }
    }
    out.push(Point::new(pts.last().unwrap().0, pts.last().unwrap().1, out.len() as u32 * 8));
    Stroke { id: 0, points: out, color: [255; 4], width: 3.0, kind: PenKind::Pen, mode: InkMode::MathText, t0_ms: 0 }
}

fn circle(cx: f32, cy: f32, rx: f32, ry: f32) -> Stroke {
    let pts: Vec<(f32, f32)> = (0..=40).map(|i| {
        let a = i as f32 / 40.0 * std::f32::consts::TAU;
        (cx + rx * a.cos(), cy + ry * a.sin())
    }).collect();
    stroke(&pts)
}

fn top(clf: &Classifier, s: &[Stroke], k: usize) -> Vec<String> {
    let refs: Vec<&Stroke> = s.iter().collect();
    clf.classify(&refs, k).unwrap().into_iter().map(|c| c.label).collect()
}

#[test]
fn bundled_model_loads_and_is_small() {
    assert!(recog::BUNDLED_MODEL.len() < 5_000_000, "model must be under 5 MB");
    let clf = Classifier::bundled().unwrap();
    assert_eq!(clf.labels().len(), 191);
}

#[test]
fn recognizes_simple_hand_built_symbols() {
    let clf = Classifier::bundled().unwrap();
    // plus sign: two crossing strokes
    let plus = [stroke(&[(0.0, 20.0), (40.0, 20.0)]), stroke(&[(20.0, 0.0), (20.0, 40.0)])];
    assert!(top(&clf, &plus, 3).contains(&"+".to_string()), "{:?}", top(&clf, &plus, 5));
    // equals sign: two parallel strokes
    let eq = [stroke(&[(0.0, 0.0), (40.0, 0.0)]), stroke(&[(0.0, 14.0), (40.0, 14.0)])];
    assert!(top(&clf, &eq, 3).contains(&"=".to_string()), "{:?}", top(&clf, &eq, 5));
    // a tall ellipse reads as 0 / O / o
    let zero = [circle(0.0, 0.0, 12.0, 20.0)];
    let t = top(&clf, &zero, 5);
    assert!(t.iter().any(|l| l == "0" || l == "O" || l == "o"), "{t:?}");
    // X as two strokes
    let x = [stroke(&[(0.0, 0.0), (30.0, 40.0)]), stroke(&[(30.0, 0.0), (0.0, 40.0)])];
    let t = top(&clf, &x, 5);
    assert!(t.iter().any(|l| l == "x" || l == "X" || l == "×"), "{t:?}");
}

#[test]
fn latency_under_30ms_per_symbol() {
    let clf = Classifier::bundled().unwrap();
    let s = [circle(0.0, 0.0, 12.0, 20.0)];
    let refs: Vec<&Stroke> = s.iter().collect();
    clf.classify(&refs, 5).unwrap(); // warm up
    let n = 50;
    let t = Instant::now();
    for _ in 0..n {
        clf.classify(&refs, 5).unwrap();
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("recognition latency: {per:.2} ms/symbol");
    assert!(per < 30.0, "{per} ms");
}
