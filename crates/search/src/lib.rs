//! Full-text search over notebooks with tantivy: typed text, recognized math, tags, PDF text.

use std::path::Path;
use store::{Block, Page};
use tantivy::collector::TopDocs;
use std::ops::Bound;
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, RangeQuery};
use tantivy::schema::{Field, Schema, Value, STORED, STRING, TEXT};
use tantivy::snippet::SnippetGenerator;
use tantivy::{doc, Index, IndexReader, IndexWriter, TantivyDocument, Term};

/// What gets indexed for one page.
#[derive(Clone, Debug, Default)]
pub struct PageDoc {
    pub notebook_id: String,
    pub page_id: String,
    pub title: String,
    pub course: String,
    pub date: String,
    pub tags: Vec<String>,
    pub body: String,
    pub pdf_text: String,
}

/// Strip Typst markup noise so snippets read well.
fn plain(markup: &str) -> String {
    markup.chars().map(|c| if matches!(c, '#' | '*' | '_' | '=' | '[' | ']' | '$') { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn doc_from_page(notebook_id: &str, p: &Page, pdf_text: &str) -> PageDoc {
    let mut body = Vec::new();
    for b in &p.blocks {
        match b {
            Block::Math { expr, source } => {
                body.push(expr.to_typst());
                body.push(expr.to_latex());
                if let Some(s) = source {
                    body.push(s.clone());
                }
            }
            Block::Text { markup } | Block::Callout { markup, .. } => body.push(plain(markup)),
            Block::Plot { functions, .. } => body.extend(functions.iter().cloned()),
            _ => {}
        }
    }
    PageDoc {
        notebook_id: notebook_id.into(),
        page_id: p.id.clone(),
        title: p.title.clone(),
        course: p.course.clone(),
        date: p.date.clone(),
        tags: p.tags.clone(),
        body: body.join("\n"),
        pdf_text: pdf_text.into(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Filters {
    pub notebook: Option<String>,
    pub tag: Option<String>,
    /// Inclusive ISO dates.
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub notebook_id: String,
    pub page_id: String,
    pub title: String,
    /// HTML snippet with `<b>` around matches.
    pub snippet: String,
    pub score: f32,
}

struct Fields {
    nb: Field,
    page: Field,
    title: Field,
    course: Field,
    date: Field,
    tag: Field,
    body: Field,
    pdf: Field,
}

pub struct SearchIndex {
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    f: Fields,
}

impl SearchIndex {
    fn schema() -> (Schema, Fields) {
        let mut b = Schema::builder();
        let f = Fields {
            nb: b.add_text_field("nb", STRING | STORED),
            page: b.add_text_field("page", STRING | STORED),
            title: b.add_text_field("title", TEXT | STORED),
            course: b.add_text_field("course", TEXT),
            date: b.add_text_field("date", STRING | STORED),
            tag: b.add_text_field("tag", STRING),
            body: b.add_text_field("body", TEXT | STORED),
            pdf: b.add_text_field("pdf", TEXT | STORED),
        };
        (b.build(), f)
    }

    fn from_index(index: Index, f: Fields) -> tantivy::Result<Self> {
        let writer = index.writer(30_000_000)?;
        let reader = index.reader()?;
        Ok(Self { index, writer, reader, f })
    }

    pub fn in_memory() -> tantivy::Result<Self> {
        let (s, f) = Self::schema();
        Self::from_index(Index::create_in_ram(s), f)
    }

    /// Open (or create) the on-disk index. Call lazily, on first search or indexing.
    pub fn open(dir: &Path) -> tantivy::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let (s, f) = Self::schema();
        let index = Index::open_in_dir(dir).or_else(|_| Index::create_in_dir(dir, s))?;
        Self::from_index(index, f)
    }

    pub fn add(&mut self, d: &PageDoc) -> tantivy::Result<()> {
        let f = &self.f;
        self.writer.delete_term(Term::from_field_text(f.page, &d.page_id));
        let mut doc = doc!(
            f.nb => d.notebook_id.as_str(), f.page => d.page_id.as_str(), f.title => d.title.as_str(),
            f.course => d.course.as_str(), f.date => d.date.as_str(), f.body => d.body.as_str(), f.pdf => d.pdf_text.as_str()
        );
        for t in &d.tags {
            doc.add_text(f.tag, t);
        }
        self.writer.add_document(doc)?;
        Ok(())
    }

    pub fn remove(&mut self, page_id: &str) {
        self.writer.delete_term(Term::from_field_text(self.f.page, page_id));
    }

    pub fn commit(&mut self) -> tantivy::Result<()> {
        self.writer.commit()?;
        self.reader.reload()
    }

    pub fn search(&self, query: &str, filters: &Filters, limit: usize) -> tantivy::Result<Vec<Hit>> {
        let f = &self.f;
        let mut q = format!("({})", if query.trim().is_empty() { "*".into() } else { query.to_string() });
        if query.trim().is_empty() {
            q = String::new();
        }
        let esc = |s: &str| s.replace('"', "");
        let mut parts: Vec<String> = Vec::new();
        if !q.is_empty() {
            parts.push(q);
        }
        if let Some(n) = &filters.notebook {
            parts.push(format!("+nb:\"{}\"", esc(n)));
        }
        if let Some(t) = &filters.tag {
            parts.push(format!("+tag:\"{}\"", esc(t)));
        }
        let has_dates = filters.date_from.is_some() || filters.date_to.is_some();
        if parts.is_empty() && !has_dates {
            return Ok(vec![]);
        }
        let parser = QueryParser::for_index(&self.index, vec![f.title, f.body, f.pdf, f.course, f.tag]);
        let mut query: Box<dyn Query> = if parts.is_empty() { Box::new(AllQuery) } else { parser.parse_query_lenient(&parts.join(" ")).0 };
        if has_dates {
            let bound = |d: &Option<String>| d.as_ref().map_or(Bound::Unbounded, |s| Bound::Included(Term::from_field_text(f.date, s)));
            let range = RangeQuery::new(bound(&filters.date_from), bound(&filters.date_to));
            query = Box::new(BooleanQuery::new(vec![(Occur::Must, query), (Occur::Must, Box::new(range))]));
        }
        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &TopDocs::with_limit(limit.max(1)).order_by_score())?;
        let mut snip_body = SnippetGenerator::create(&searcher, &*query, f.body)?;
        snip_body.set_max_num_chars(140);
        let mut snip_pdf = SnippetGenerator::create(&searcher, &*query, f.pdf)?;
        snip_pdf.set_max_num_chars(140);
        let mut hits = Vec::new();
        for (score, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            let get = |fl: Field| d.get_first(fl).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut snip = snip_body.snippet_from_doc(&d).to_html();
            if snip.is_empty() {
                snip = snip_pdf.snippet_from_doc(&d).to_html();
            }
            hits.push(Hit { notebook_id: get(f.nb), page_id: get(f.page), title: get(f.title), snippet: snip, score });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::{Block, Page};

    fn page(id: &str, title: &str, text: &str, tags: &[&str], date: &str) -> PageDoc {
        let mut p = Page::new(id, title, date);
        p.tags = tags.iter().map(|s| s.to_string()).collect();
        p.blocks.push(Block::Text { markup: text.into() });
        doc_from_page("nb1", &p, "")
    }

    #[test]
    fn finds_text_math_tags_pdf_and_highlights() {
        let mut ix = SearchIndex::in_memory().unwrap();
        ix.add(&page("a", "Chain rule", "The derivative of a composite function", &["calc"], "2026-09-01")).unwrap();
        ix.add(&page("b", "Eigenvalues", "characteristic polynomial det", &["linalg"], "2026-10-01")).unwrap();
        let mut p = Page::new("c", "Fourier", "2026-10-15");
        p.blocks.push(Block::Math { expr: layout::Expr::Frac(Box::new(layout::Expr::sym("α")), Box::new(layout::Expr::sym("β"))), source: None });
        let mut d = doc_from_page("nb2", &p, "slide text about convolution theorem");
        d.pdf_text = "slide text about convolution theorem".into();
        ix.add(&d).unwrap();
        ix.commit().unwrap();
        let h = ix.search("derivative", &Filters::default(), 10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].page_id, "a");
        assert!(h[0].snippet.contains("<b>derivative</b>"), "{}", h[0].snippet);
        assert_eq!(ix.search("frac", &Filters::default(), 10).unwrap()[0].page_id, "c", "recognized math is searchable");
        assert_eq!(ix.search("convolution", &Filters::default(), 10).unwrap()[0].page_id, "c", "PDF text is searchable");
        assert_eq!(ix.search("", &Filters { tag: Some("linalg".into()), ..Default::default() }, 10).unwrap()[0].page_id, "b");
    }

    #[test]
    fn filters_by_notebook_tag_and_date_and_updates_replace() {
        let mut ix = SearchIndex::in_memory().unwrap();
        ix.add(&page("a", "Limits", "epsilon delta", &["calc"], "2026-09-01")).unwrap();
        ix.add(&page("b", "Limits two", "epsilon delta again", &["calc"], "2026-10-20")).unwrap();
        ix.commit().unwrap();
        let q = |f: Filters| ix.search("epsilon", &f, 10).unwrap().len();
        assert_eq!(q(Filters::default()), 2);
        assert_eq!(q(Filters { notebook: Some("other".into()), ..Default::default() }), 0);
        assert_eq!(q(Filters { tag: Some("calc".into()), ..Default::default() }), 2);
        assert_eq!(q(Filters { date_from: Some("2026-10-01".into()), ..Default::default() }), 1);
        assert_eq!(q(Filters { date_to: Some("2026-09-30".into()), ..Default::default() }), 1);
        ix.add(&page("a", "Limits", "completely different words", &[], "2026-09-01")).unwrap();
        ix.remove("b");
        ix.commit().unwrap();
        let q = |f: Filters| ix.search("epsilon", &f, 10).unwrap().len();
        assert_eq!(q(Filters::default()), 0);
    }

    #[test]
    fn search_is_fast_on_1000_pages() {
        let mut ix = SearchIndex::in_memory().unwrap();
        for i in 0..1000 {
            ix.add(&page(&format!("p{i}"), &format!("Page {i}"), &format!("lecture notes about topic{} and integrals of f{}", i % 37, i), &["t"], "2026-01-01")).unwrap();
        }
        ix.commit().unwrap();
        ix.search("integrals", &Filters::default(), 20).unwrap(); // warm
        let t = std::time::Instant::now();
        for _ in 0..20 {
            ix.search("topic5 integrals", &Filters::default(), 20).unwrap();
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!("search: {ms:.2} ms on 1000 pages");
        assert!(ms < 100.0, "{ms} ms");
    }

    #[test]
    fn on_disk_index_persists() {
        let d = std::env::temp_dir().join(format!("iw-search-{}", store::new_id()));
        {
            let mut ix = SearchIndex::open(&d).unwrap();
            ix.add(&page("a", "Saved", "persist me", &[], "2026-01-01")).unwrap();
            ix.commit().unwrap();
        }
        let ix = SearchIndex::open(&d).unwrap();
        assert_eq!(ix.search("persist", &Filters::default(), 5).unwrap().len(), 1);
        std::fs::remove_dir_all(d).ok();
    }
}
