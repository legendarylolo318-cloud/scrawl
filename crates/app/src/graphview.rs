//! Graph view state: expression lines, MATLAB scripts and `.mat` data, view pan/zoom.

use media::graph::{self, Frame, Graph, Kind, Viewport};
use media::matlab::{self, Interp, Mat};

pub struct GraphView {
    pub graph: Graph,
    pub vp: Viewport,
    pub size: (u32, u32),
    pub dark: bool,
    lines: String,
    data: Vec<Vec<(f64, f64)>>,
    interp: Interp,
}

impl Default for GraphView {
    fn default() -> Self {
        let mut g = GraphView { graph: Graph::default(), vp: Viewport::default(), size: (600, 400), dark: true, lines: String::new(), data: vec![], interp: Interp::new() };
        g.set_text("y = a sin(x)\na = 2");
        g
    }
}

impl GraphView {
    /// One expression per line; blank lines and `#`/`%` comments are ignored.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.to_string();
        let old: Vec<(String, graph::Param)> = self.graph.params.iter().map(|(k, v)| (k.clone(), *v)).collect();
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('%')).collect();
        self.graph.set_lines(&lines);
        // keep slider positions the user already set
        for (k, p) in old {
            if let Some(cur) = self.graph.params.get(&k) {
                let explicit = lines.iter().any(|l| l.trim_start().starts_with(&format!("{k} =")) || l.trim_start().starts_with(&format!("{k}=")));
                if !explicit && (cur.value - 1.0).abs() < f64::EPSILON {
                    self.graph.set_param(&k, p.value);
                }
            }
        }
        for d in &self.data {
            self.graph.items.push(graph::Item { text: "data".into(), kind: Kind::Data(d.clone()), visible: true });
        }
    }

    pub fn frame(&self) -> Frame {
        graph::render(&self.graph, &self.vp, self.size.0, self.size.1, self.dark)
    }

    /// Roots / extrema / intersections currently visible.
    pub fn info(&self) -> String {
        let (x0, _) = self.vp.to_world(0.0, 0.0, self.size.0 as f64, self.size.1 as f64);
        let (x1, _) = self.vp.to_world(self.size.0 as f64, 0.0, self.size.0 as f64, self.size.1 as f64);
        let mut pts = graph::interesting_points(&self.graph, x0, x1);
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6 && a.2 == b.2);
        pts.iter().take(14).map(|(x, y, l)| format!("{l}: ({:.4}, {:.4})", x, y)).collect::<Vec<_>>().join("\n")
    }

    /// Run a MATLAB script; plotted series become graph data. Returns printed output / error.
    pub fn run_matlab(&mut self, script: &str) -> String {
        match self.interp.run(script) {
            Ok(out) => {
                self.data = out.series.iter().map(|s| s.x.iter().cloned().zip(s.y.iter().cloned()).collect()).collect();
                let text = self.lines.clone();
                self.set_text(&text);
                if let Some(all) = self.fit_data() {
                    self.vp = all;
                }
                let mut msg = out.text;
                if !out.title.is_empty() {
                    msg = format!("[{}]\n{msg}", out.title);
                }
                if msg.trim().is_empty() { format!("{} series plotted", self.data.len()) } else { msg }
            }
            Err(e) => format!("MATLAB error: {e}"),
        }
    }

    /// Load variables from a `.mat` file so scripts can use them.
    pub fn load_mat(&mut self, bytes: &[u8]) -> String {
        match matlab::load_mat(bytes) {
            Ok(vars) => {
                let names: Vec<String> = vars.iter().map(|v| format!("{} ({}x{})", v.name, v.value.rows, v.value.cols)).collect();
                for v in vars {
                    self.interp.set_var(&v.name, v.value);
                }
                format!("Loaded: {}", names.join(", "))
            }
            Err(e) => e,
        }
    }

    pub fn set_var(&mut self, name: &str, m: Mat) {
        self.interp.set_var(name, m);
    }

    fn fit_data(&self) -> Option<Viewport> {
        let pts: Vec<&(f64, f64)> = self.data.iter().flatten().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
        if pts.is_empty() {
            return None;
        }
        let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        let scale = ((w * 0.85) / (x1 - x0).max(1e-9)).min((h * 0.85) / (y1 - y0).max(1e-9));
        Some(Viewport { cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, scale })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_lines_make_sliders_and_info() {
        let mut g = GraphView::default();
        g.set_text("y = x^2 - 4\ny = k x\nk = 1");
        assert!(g.graph.params.contains_key("k"));
        let info = g.info();
        assert!(info.contains("zero: (-2.0000, 0.0000)") && info.contains("intersection"), "{info}");
        g.graph.set_param("k", 3.0);
        g.set_text("y = x^2 - 4\ny = k x\nk = 3");
        assert_eq!(g.graph.params["k"].value, 3.0);
    }

    #[test]
    fn matlab_script_becomes_graph_data_and_view_fits() {
        let mut g = GraphView::default();
        let out = g.run_matlab("x = 0:0.1:10; y = x.^2; plot(x, y); title('Squares')");
        assert!(out.contains("Squares"), "{out}");
        assert!(g.graph.items.iter().any(|i| matches!(i.kind, Kind::Data(ref d) if d.len() == 101)));
        assert!(g.vp.cx > 4.0 && g.vp.cy > 40.0, "view fits the data: {:?}", g.vp);
        assert!(g.run_matlab("for i=1:3\nend").starts_with("MATLAB error"));
        // typed expressions still work after a script ran
        g.set_text("y = x");
        assert!(g.graph.items.iter().any(|i| matches!(i.kind, Kind::Explicit(_))) && g.graph.items.iter().any(|i| matches!(i.kind, Kind::Data(_))));
        let f = g.frame();
        assert_eq!(f.rgba.len(), (f.width * f.height * 4) as usize);
    }
}
