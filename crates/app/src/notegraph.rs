//! The graph of linked notes (like Obsidian's graph view): pages are dots, `[[links]]` are lines.
//! Positions come from a small force-directed layout in the unit square; rendering is tiny-skia.

use crate::workspace::Workspace;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

pub struct Node {
    /// Page title (also what a link to it is called).
    pub title: String,
    /// False for a link whose page does not exist yet.
    pub exists: bool,
    pub current: bool,
    pub x: f32,
    pub y: f32,
    pub deg: usize,
}

#[derive(Default)]
pub struct NoteGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<(usize, usize)>,
}

const MAX_NODES: usize = 600;

/// Page title positions (x, y in logical px, text).
pub type Labels = Vec<(f32, f32, String)>;

impl NoteGraph {
    /// Pages of the open notebook and the links between them.
    pub fn build(ws: &Workspace) -> NoteGraph {
        let mut g = NoteGraph::default();
        let mut index: std::collections::HashMap<String, usize> = Default::default();
        let mut pages: Vec<(usize, Vec<String>)> = Vec::new();
        for sec in &ws.nb.sections {
            for r in &sec.pages {
                if g.nodes.len() >= MAX_NODES {
                    break;
                }
                let key = r.title.trim().to_lowercase();
                if key.is_empty() || index.contains_key(&key) {
                    continue;
                }
                let mut page = if r.id == ws.page.id { Some(ws.page.clone()) } else { ws.load_other(&r.id) };
                if let Some(p) = page.as_mut() {
                    p.sync_blocks();
                }
                let links = page.as_ref().map(crate::pagedoc::links).unwrap_or_default();
                index.insert(key, g.nodes.len());
                pages.push((g.nodes.len(), links));
                g.nodes.push(Node { title: r.title.trim().to_string(), exists: true, current: r.id == ws.page.id, x: 0.0, y: 0.0, deg: 0 });
            }
        }
        for (from, links) in pages {
            for l in links {
                let key = l.trim().to_lowercase();
                if key.is_empty() {
                    continue;
                }
                let to = match index.get(&key) {
                    Some(&i) => i,
                    None if g.nodes.len() < MAX_NODES => {
                        index.insert(key, g.nodes.len());
                        g.nodes.push(Node { title: l.trim().to_string(), exists: false, current: false, x: 0.0, y: 0.0, deg: 0 });
                        g.nodes.len() - 1
                    }
                    None => continue,
                };
                let e = (from.min(to), from.max(to));
                if from != to && !g.edges.contains(&e) {
                    g.edges.push(e);
                }
            }
        }
        for &(a, b) in &g.edges {
            g.nodes[a].deg += 1;
            g.nodes[b].deg += 1;
        }
        g.layout(220);
        g
    }

    /// Fruchterman-Reingold in the unit square, deterministic.
    pub fn layout(&mut self, iterations: usize) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        if n == 1 {
            self.nodes[0].x = 0.5;
            self.nodes[0].y = 0.5;
            return;
        }
        for (i, nd) in self.nodes.iter_mut().enumerate() {
            let a = i as f32 / n as f32 * std::f32::consts::TAU * 2.0;
            let r = 0.15 + 0.3 * (i as f32 / n as f32);
            nd.x = 0.5 + r * a.cos();
            nd.y = 0.5 + r * a.sin();
        }
        let k = (1.0 / n as f32).sqrt() * 0.8;
        let mut temp = 0.1f32;
        for _ in 0..iterations {
            let mut dx = vec![0.0f32; n];
            let mut dy = vec![0.0f32; n];
            for i in 0..n {
                for j in i + 1..n {
                    let (mut ux, mut uy) = (self.nodes[i].x - self.nodes[j].x, self.nodes[i].y - self.nodes[j].y);
                    let mut d = (ux * ux + uy * uy).sqrt();
                    if d < 1e-4 {
                        ux = 1e-3 * ((i + j) as f32 + 1.0);
                        uy = 1e-3;
                        d = (ux * ux + uy * uy).sqrt();
                    }
                    let f = k * k / d;
                    dx[i] += ux / d * f;
                    dy[i] += uy / d * f;
                    dx[j] -= ux / d * f;
                    dy[j] -= uy / d * f;
                }
            }
            for &(a, b) in &self.edges {
                let (ux, uy) = (self.nodes[a].x - self.nodes[b].x, self.nodes[a].y - self.nodes[b].y);
                let d = (ux * ux + uy * uy).sqrt().max(1e-4);
                let f = d * d / k;
                dx[a] -= ux / d * f;
                dy[a] -= uy / d * f;
                dx[b] += ux / d * f;
                dy[b] += uy / d * f;
            }
            for i in 0..n {
                // a little pull to the middle keeps unlinked notes on screen
                dx[i] += (0.5 - self.nodes[i].x) * 0.15 * k;
                dy[i] += (0.5 - self.nodes[i].y) * 0.15 * k;
                let d = (dx[i] * dx[i] + dy[i] * dy[i]).sqrt().max(1e-6);
                let s = d.min(temp) / d;
                self.nodes[i].x = (self.nodes[i].x + dx[i] * s).clamp(0.04, 0.96);
                self.nodes[i].y = (self.nodes[i].y + dy[i] * s).clamp(0.06, 0.94);
            }
            temp = (temp * 0.97).max(0.002);
        }
    }

    pub fn radius(&self, i: usize) -> f32 {
        5.0 + 2.2 * (self.nodes[i].deg as f32).sqrt()
    }

    /// The node under a point given as fractions of the picture (0..1), within `slack` logical px.
    pub fn hit(&self, fx: f32, fy: f32, w: f32, h: f32, slack: f32) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let (dx, dy) = ((n.x - fx) * w, (n.y - fy) * h);
                (i, (dx * dx + dy * dy).sqrt() - self.radius(i))
            })
            .filter(|(_, d)| *d <= slack)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Draw lines and dots at `scale` device px per logical px. Returns the pixmap and label positions
    /// (logical px) for the page titles.
    pub fn render(&self, w: f32, h: f32, scale: f32, text: [u8; 3], muted: [u8; 3], accent: [u8; 3]) -> Option<(Pixmap, Labels)> {
        let mut pm = Pixmap::new((w * scale).ceil().max(1.0) as u32, (h * scale).ceil().max(1.0) as u32)?;
        let at = |n: &Node| (n.x * w * scale, n.y * h * scale);
        let mut line = Paint::default();
        line.set_color(Color::from_rgba8(muted[0], muted[1], muted[2], 150));
        line.anti_alias = true;
        for &(a, b) in &self.edges {
            let (p, q) = (at(&self.nodes[a]), at(&self.nodes[b]));
            let mut pb = PathBuilder::new();
            pb.move_to(p.0, p.1);
            pb.line_to(q.0, q.1);
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &line, &Stroke { width: 1.3 * scale, ..Default::default() }, Transform::identity(), None);
            }
        }
        let mut labels = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let (x, y) = at(n);
            let r = self.radius(i) * scale;
            let mut p = Paint::default();
            let c = if n.current { accent } else if n.exists { text } else { muted };
            p.set_color(Color::from_rgba8(c[0], c[1], c[2], if n.exists { 255 } else { 150 }));
            p.anti_alias = true;
            if let Some(path) = PathBuilder::from_circle(x, y, r) {
                pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
                if n.current {
                    if let Some(ring) = PathBuilder::from_circle(x, y, r + 4.0 * scale) {
                        pm.stroke_path(&ring, &p, &Stroke { width: 2.0 * scale, ..Default::default() }, Transform::identity(), None);
                    }
                }
            }
            // labels go to the right of a dot, or to its left near the right edge so they stay inside
            let text_w = n.title.chars().count() as f32 * 7.0;
            let lx = if n.x * w + text_w + 20.0 > w { n.x * w - self.radius(i) - 5.0 - text_w } else { n.x * w + self.radius(i) + 5.0 };
            labels.push((lx.max(2.0), n.y * h - 8.0, n.title.clone()));
        }
        Some((pm, labels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> NoteGraph {
        let mk = |t: &str, exists| Node { title: t.into(), exists, current: false, x: 0.0, y: 0.0, deg: 0 };
        let mut g = NoteGraph { nodes: vec![mk("A", true), mk("B", true), mk("C", true), mk("D", true), mk("Ghost", false)], edges: vec![(0, 1), (1, 2), (2, 0), (3, 4)] };
        for &(a, b) in &g.edges.clone() {
            g.nodes[a].deg += 1;
            g.nodes[b].deg += 1;
        }
        g.layout(220);
        g
    }

    #[test]
    fn layout_keeps_everything_in_the_square_and_apart() {
        let g = graph();
        for n in &g.nodes {
            assert!((0.0..=1.0).contains(&n.x) && (0.0..=1.0).contains(&n.y));
        }
        for i in 0..g.nodes.len() {
            for j in i + 1..g.nodes.len() {
                let d = ((g.nodes[i].x - g.nodes[j].x).powi(2) + (g.nodes[i].y - g.nodes[j].y).powi(2)).sqrt();
                assert!(d > 0.03, "nodes {i} and {j} overlap");
            }
        }
        let len = |a: usize, b: usize| ((g.nodes[a].x - g.nodes[b].x).powi(2) + (g.nodes[a].y - g.nodes[b].y).powi(2)).sqrt();
        assert!(len(0, 1) < len(0, 3), "linked notes sit closer than unlinked ones");
    }

    #[test]
    fn layout_is_deterministic_and_handles_tiny_graphs() {
        let (a, b) = (graph(), graph());
        assert!(a.nodes.iter().zip(&b.nodes).all(|(x, y)| x.x == y.x && x.y == y.y));
        let mut empty = NoteGraph::default();
        empty.layout(10);
        let mut one = NoteGraph { nodes: vec![Node { title: "x".into(), exists: true, current: true, x: 0.0, y: 0.0, deg: 0 }], edges: vec![] };
        one.layout(10);
        assert_eq!((one.nodes[0].x, one.nodes[0].y), (0.5, 0.5));
    }

    #[test]
    fn clicking_picks_the_nearest_node_or_nothing() {
        let g = graph();
        let n = &g.nodes[2];
        assert_eq!(g.hit(n.x, n.y, 800.0, 500.0, 4.0), Some(2));
        // just outside the dot but within the slack still counts; far away does not
        assert_eq!(g.hit(n.x + (g.radius(2) + 3.0) / 800.0, n.y, 800.0, 500.0, 4.0), Some(2));
        assert!(g.hit(n.x + (g.radius(2) + 30.0) / 800.0, n.y, 800.0, 500.0, 4.0).is_none_or(|i| i != 2));
        assert!(g.hit(2.0, 2.0, 800.0, 500.0, 4.0).is_none());
    }

    #[test]
    fn rendering_draws_dots_and_returns_one_label_per_node() {
        let g = graph();
        let (pm, labels) = g.render(400.0, 300.0, 2.0, [255, 255, 255], [120, 120, 120], [100, 150, 255]).unwrap();
        assert_eq!((pm.width(), pm.height()), (800, 600));
        assert!(pm.data().chunks(4).filter(|p| p[3] > 0).count() > 400);
        assert_eq!(labels.len(), 5);
    }
}
