//! Obsidian compatibility: export a notebook as a folder of Markdown files (a vault) and import a vault.
//!
//! * Pages become `Section/Title.md` with YAML front matter (title, tags, created), `[[wikilinks]]`,
//!   `- [ ]` tasks, `$math$`, `> [!note]` callouts and `![[image]]` (images go to `attachments/`).
//! * Import reads every `.md` below a folder (skipping dot-folders such as `.obsidian`), keeps the folder
//!   names as sections, `#tags` and front-matter tags as page tags, and copies the images it can find.
//! * Nothing outside the chosen folder is read or written; symbolic links are ignored.

use crate::workspace::Workspace;
use notedoc::{Doc, Link, ParaKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use store::Page;

const MAX_MD_BYTES: u64 = 5 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_FILES: usize = 5000;
const MAX_DEPTH: usize = 8;

/// A file name that is safe on Linux, Windows and macOS.
pub fn safe_name(s: &str) -> String {
    let t: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']') || c.is_control() { '-' } else { c }).collect();
    let t = t.trim().trim_matches('.').trim().to_string();
    let t: String = t.chars().take(100).collect::<String>().trim_end_matches(['.', ' ']).to_string();
    if t.is_empty() {
        return "Untitled".into();
    }
    // Windows refuses CON, PRN, AUX, NUL, COM1-9 and LPT1-9 as file names, with or without an extension
    let stem = t.split('.').next().unwrap_or("").trim().to_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str()) || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{t}")
    } else {
        t
    }
}

fn unique(used: &mut HashMap<String, usize>, base: String) -> String {
    let n = used.entry(base.to_lowercase()).or_insert(0);
    *n += 1;
    if *n == 1 {
        base
    } else {
        format!("{base} ({n})")
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub pages: usize,
    pub images: usize,
    pub skipped: Vec<String>,
}

/// Write the current notebook into `dest/<notebook name>/`.
pub fn export(ws: &mut Workspace, dest: &Path) -> Result<(Report, PathBuf), String> {
    let root = dest.join(safe_name(&ws.nb.name));
    std::fs::create_dir_all(root.join("attachments")).map_err(|e| e.to_string())?;
    let sections = ws.nb.sections.clone();
    // first pass: the file name of every page, so links point at the right files
    let mut used_in_vault: HashMap<String, usize> = HashMap::new();
    let mut file_of: HashMap<String, String> = HashMap::new();
    let mut plan: Vec<(String, String, String)> = Vec::new(); // (section dir, file stem, page id)
    for sec in &sections {
        for p in &sec.pages {
            let stem = unique(&mut used_in_vault, safe_name(&p.title));
            file_of.insert(p.title.trim().to_lowercase(), stem.clone());
            plan.push((safe_name(&sec.name), stem, p.id.clone()));
        }
    }
    let mut report = Report::default();
    let mut copied: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (sec, stem, id) in plan {
        let mut page: Page = if id == ws.page.id { ws.page.clone() } else if let Some(p) = ws.load_other(&id) { p } else { report.skipped.push(format!("{stem}: could not be read")); continue };
        page.ensure_doc();
        let Some(doc) = page.doc.as_ref() else { continue };
        let link_name = |t: &str| file_of.get(&t.trim().to_lowercase()).cloned().unwrap_or_else(|| safe_name(t));
        let image_name = |f: &str| f.to_string();
        let mut md = String::new();
        md += "---\n";
        md += &format!("title: \"{}\"\n", page.title.replace('"', "'").replace('\n', " "));
        let mut tags = page.tags.clone();
        for t in doc.hashtags() {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        if !tags.is_empty() {
            md += &format!("tags: [{}]\n", tags.iter().map(|t| t.trim().replace([' ', ','], "-")).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "));
        }
        if !page.date.is_empty() {
            md += &format!("created: {}\n", page.date);
        }
        md += "---\n\n";
        md += &doc.to_markdown(&link_name, &image_name);
        let dir = root.join(&sec);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(format!("{stem}.md")), md).map_err(|e| e.to_string())?;
        report.pages += 1;
        for p in &doc.paras {
            if let ParaKind::Image { file, .. } = &p.kind {
                if copied.insert(file.clone()) {
                    match ws.media(file) {
                        Some(bytes) => {
                            let name = safe_name(file);
                            if std::fs::write(root.join("attachments").join(&name), bytes).is_ok() {
                                report.images += 1;
                            }
                        }
                        None => report.skipped.push(format!("{stem}: image {file} is missing")),
                    }
                }
            }
        }
    }
    Ok((report, root))
}

/// Collect `.md` files and images. Returns how many notes were left out (too many or too deep).
fn walk(dir: &Path, depth: usize, md: &mut Vec<PathBuf>, images: &mut HashMap<String, PathBuf>) -> usize {
    let mut left_out = 0;
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() || name.starts_with('.') {
            continue;
        }
        let p = e.path();
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                left_out += walk(&p, depth + 1, md, images);
            } else {
                left_out += count_md(&p, 0);
            }
        } else if ft.is_file() {
            let lower = name.to_lowercase();
            if lower.ends_with(".md") {
                if md.len() < MAX_FILES {
                    md.push(p);
                } else {
                    left_out += 1;
                }
            } else if [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|x| lower.ends_with(x)) {
                images.entry(lower).or_insert(p);
            }
        }
    }
    left_out
}

/// Notes in a folder that is too deep to import (counted so the user is told; bounded work).
fn count_md(dir: &Path, depth: usize) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten().take(10_000) {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && !ft.is_symlink() && depth < 8 {
            n += count_md(&e.path(), depth + 1);
        } else if ft.is_file() && e.file_name().to_string_lossy().to_lowercase().ends_with(".md") {
            n += 1;
        }
    }
    n
}

/// How a wiki link target is matched to a file: `Folder/Note.md` and `note` both mean the file `Note.md`.
fn link_key(target: &str) -> String {
    let t = target.trim();
    let t = t.rsplit('/').next().unwrap_or(t);
    let cut = t.len().saturating_sub(3);
    let t = if t.get(cut..).is_some_and(|e| e.eq_ignore_ascii_case(".md")) { &t[..cut] } else { t };
    t.trim().to_lowercase()
}

/// Import the Markdown files below `src` as a new notebook and open it.
pub fn import(ws: &mut Workspace, src: &Path, now: u64) -> Result<Report, String> {
    let src = src.canonicalize().map_err(|e| e.to_string())?;
    let (mut files, mut images) = (Vec::new(), HashMap::new());
    let left_out = walk(&src, 0, &mut files, &mut images);
    if files.is_empty() {
        return Err("No Markdown (.md) files found in that folder".into());
    }
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "vault".into());
    ws.create_notebook(&format!("{name} (imported)"), None, now)?;
    // create_notebook opens a starter page; it is removed once the notes are in
    let placeholder = ws.page.id.clone();
    // the empty "Notes" section made by create_notebook is reused for files in the top folder
    let mut section_ids: HashMap<String, String> = HashMap::new();
    if let Some(s) = ws.nb.sections.first() {
        section_ids.insert("Notes".into(), s.id.clone());
    }
    // Obsidian links use file names; Scrawl links use page titles (they differ when front matter has a title)
    let mut title_of_stem: HashMap<String, String> = HashMap::new();
    for f in &files {
        if std::fs::metadata(f).is_ok_and(|m| m.len() <= MAX_MD_BYTES) {
            if let Ok(t) = std::fs::read_to_string(f) {
                if let Some(stem) = f.file_stem() {
                    let stem = stem.to_string_lossy().to_string();
                    title_of_stem.insert(stem.to_lowercase(), notedoc::front_matter(&t).0.title.unwrap_or(stem));
                }
            }
        }
    }
    let mut report = Report::default();
    if left_out > 0 {
        report.skipped.push(format!("{left_out} note(s) were not imported (more than {MAX_FILES} notes or folders deeper than {MAX_DEPTH})"));
    }
    let mut first_page: Option<String> = None;
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut media_done: HashMap<String, Option<(String, u32, u32)>> = HashMap::new();
    for f in files {
        let Ok(meta) = std::fs::metadata(&f) else { continue };
        let rel = f.strip_prefix(&src).unwrap_or(&f).to_path_buf();
        if meta.len() > MAX_MD_BYTES {
            report.skipped.push(format!("{}: larger than 5 MB", rel.display()));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            report.skipped.push(format!("{}: not UTF-8 text", rel.display()));
            continue;
        };
        let (fm, _) = notedoc::front_matter(&text);
        let stem = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        let title = fm.title.clone().unwrap_or(stem.clone());
        let mut doc = Doc::from_markdown(&text);
        for p in doc.paras.iter_mut() {
            for r in p.runs.iter_mut() {
                if let notedoc::Run::Text { link: Some(Link::Page(t)), .. } = r {
                    if let Some(real) = title_of_stem.get(&link_key(t)) {
                        *t = real.clone();
                    }
                }
            }
        }
        // images: copy the ones we can find, turn the rest into a visible note
        for p in doc.paras.iter_mut() {
            let ParaKind::Image { file, w, h, .. } = &mut p.kind else { continue };
            let key = Path::new(file.as_str()).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let stored = media_done
                .entry(key.clone())
                .or_insert_with(|| {
                    let path = images.get(&key)?;
                    if std::fs::metadata(path).ok()?.len() > MAX_IMAGE_BYTES {
                        return None;
                    }
                    let img = media::images::Img::decode(&std::fs::read(path).ok()?).ok()?;
                    let img = if img.size().0 > 2000 { img.fit(2000, 4000) } else { img };
                    let (iw, ih) = img.size();
                    let name = ws.add_media("png", &img.to_png()).ok()?;
                    Some((name, iw, ih))
                })
                .clone();
            match stored {
                Some((name, iw, ih)) => {
                    let dw = (iw as f32).min(360.0);
                    *file = name;
                    *w = dw;
                    *h = dw * ih as f32 / iw.max(1) as f32;
                    report.images += 1;
                }
                None => {
                    let missing = format!("[missing image: {file}]");
                    p.kind = ParaKind::Body;
                    p.runs = vec![notedoc::Run::plain(&missing)];
                }
            }
        }
        let sec_name = rel.components().next().filter(|_| rel.components().count() > 1).map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_else(|| "Notes".into());
        let sec_id = match section_ids.get(&sec_name) {
            Some(id) => id.clone(),
            None => {
                let id = store::new_id();
                ws.nb.add_section(&id, &sec_name);
                section_ids.insert(sec_name.clone(), id.clone());
                id
            }
        };
        let mut page = Page::new(&store::new_id(), &unique(&mut used, title.trim().to_string()), &store::iso_date(now));
        let mut tags = fm.tags.clone();
        for t in doc.hashtags() {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        page.tags = tags;
        page.doc = Some(doc);
        page.sync_blocks();
        if first_page.is_none() {
            first_page = Some(page.id.clone());
        }
        ws.store.create_page(&mut ws.nb, &sec_id, page, &ws.vault).map_err(|e| e.to_string())?;
        report.pages += 1;
    }
    ws.store.save_notebook(&ws.nb, &ws.vault).map_err(|e| e.to_string())?;
    if let Some(first) = first_page {
        ws.open_page(&first, now).map_err(|e| e.to_string())?;
        if placeholder != first && ws.nb.find_page(&placeholder).is_some() {
            // deleted with time 0 so the purge removes it for good right away
            ws.store.delete_page(&mut ws.nb, &placeholder, &ws.vault, 0).map_err(|e| e.to_string())?;
            ws.store.purge_trash(&ws.nb.id, &ws.vault, now.max(store::now_ms())).map_err(|e| e.to_string())?;
        }
    }
    Ok(report)
}

/// Links in imported notes that point at pages that do not exist (Obsidian keeps those as "unresolved").
/// Scrawl creates the page when the link is followed, so this only reports.
pub fn dangling_links(ws: &Workspace) -> usize {
    let titles: std::collections::HashSet<String> = ws.nb.sections.iter().flat_map(|s| s.pages.iter().map(|p| p.title.trim().to_lowercase())).collect();
    let mut n = 0;
    for sec in &ws.nb.sections {
        for p in &sec.pages {
            if let Some(page) = ws.load_other(&p.id) {
                for l in page.doc.iter().flat_map(|d| d.paras.iter()).flat_map(|p| p.runs.iter()) {
                    if let notedoc::Run::Text { link: Some(Link::Page(t)), .. } = l {
                        if !titles.contains(&t.trim().to_lowercase()) {
                            n += 1;
                        }
                    }
                }
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scrawl-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn page(title: &str, md: &str, tags: &[&str]) -> Page {
        let mut p = Page::new(&store::new_id(), title, "2026-11-10");
        p.doc = Some(Doc::from_markdown(md));
        p.tags = tags.iter().map(|t| t.to_string()).collect();
        p.sync_blocks();
        p
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(safe_name("a/b:c*d?"), "a-b-c-d-");
        assert_eq!(safe_name("  ..  "), "Untitled");
        assert_eq!(safe_name("Limits #1 [x]"), "Limits -1 -x-");
        assert!(safe_name(&"x".repeat(500)).len() <= 100);
    }

    #[test]
    fn export_then_import_keeps_pages_links_tags_and_tasks() {
        let notes = tmp("notes");
        let out = tmp("out");
        let mut ws = Workspace::open_with(&notes, None, None).unwrap();
        ws.rename("Calculus");
        ws.page.doc = Some(Doc::from_markdown("= Calculus\nSee \\[\\[x\\]\\]"));
        let mut cal = page("Calculus", "# Calculus\n\nSee [[Limits]] and [[Chain: rule]] for $x^2$.\n\n- [ ] revise\n- [x] read\n\n> [!warning] Careful", &["math"]);
        cal.id = ws.page.id.clone();
        ws.page = cal;
        ws.touch();
        ws.save(1).unwrap();
        let sec = ws.nb.sections[0].id.clone();
        for p in [page("Limits", "Back to [[Calculus]] #analysis", &[]), page("Chain: rule", "## Chain\n\n1. one\n2. two", &["math", "uni"])] {
            ws.store.create_page(&mut ws.nb, &sec, p, &ws.vault).unwrap();
        }
        ws.store.save_notebook(&ws.nb, &ws.vault).unwrap();

        let (rep, root) = export(&mut ws, &out).unwrap();
        assert_eq!(rep.pages, 3, "{rep:?}");
        let cal_md = std::fs::read_to_string(root.join("Notes").join("Calculus.md")).unwrap();
        assert!(cal_md.starts_with("---\ntitle: \"Calculus\"\ntags: [math]"), "{cal_md}");
        assert!(cal_md.contains("[[Limits]]") && cal_md.contains("[[Chain- rule|Chain: rule]]") && cal_md.contains("- [ ] revise") && cal_md.contains("- [x] read") && cal_md.contains("> [!warning] Careful"), "{cal_md}");
        assert!(root.join("Notes").join("Chain- rule.md").exists());

        let r = import(&mut ws, &root, 2).unwrap();
        assert_eq!(r.pages, 3, "{r:?}");
        assert!(ws.nb.name.ends_with("(imported)"));
        let titles: Vec<String> = ws.nb.sections.iter().flat_map(|s| s.pages.iter().map(|p| p.title.clone())).collect();
        for t in ["Calculus", "Limits", "Chain: rule"] {
            assert!(titles.contains(&t.to_string()), "{titles:?}");
        }
        let lim = ws.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.title == "Limits").unwrap().id.clone();
        let lim = ws.load_other(&lim).unwrap();
        assert!(lim.tags.contains(&"analysis".to_string()), "{:?}", lim.tags);
        assert!(lim.doc.unwrap().to_typst().contains("[[Calculus]]"));
        let cal = ws.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.title == "Calculus").unwrap().id.clone();
        let cal = ws.load_other(&cal).unwrap();
        assert!(cal.doc.unwrap().to_typst().contains("[[Chain: rule]]"), "links follow the page title, not the file name");
        std::fs::remove_dir_all(notes).ok();
        std::fs::remove_dir_all(out).ok();
    }

    #[test]
    fn import_handles_obsidian_folders_images_and_junk() {
        let notes = tmp("n2");
        let vault = tmp("v2");
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        std::fs::write(vault.join(".obsidian").join("hidden.md"), "# never read").unwrap();
        std::fs::create_dir_all(vault.join("Uni").join("Year 1")).unwrap();
        std::fs::write(vault.join("Top.md"), "---\ntitle: First\ntags: [a]\n---\nhello ![[missing.png]]\n\n![[missing.png]]").unwrap();
        std::fs::write(vault.join("Uni").join("Year 1").join("Deep.md"), "# Deep\n\n![[pic.png]]").unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255]))).write_to(&mut png, image::ImageFormat::Png).unwrap();
        std::fs::write(vault.join("Uni").join("pic.png"), png.into_inner()).unwrap();
        std::fs::write(vault.join("bad.md"), [0xff, 0xfe, 0xfd]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", vault.join("link.md")).unwrap();
        let mut ws = Workspace::open_with(&notes, None, None).unwrap();
        let r = import(&mut ws, &vault, 5).unwrap();
        assert_eq!(r.pages, 2, "{r:?}");
        assert_eq!(r.images, 1);
        assert_eq!(r.skipped.len(), 1, "{r:?}");
        let secs: Vec<&str> = ws.nb.sections.iter().map(|s| s.name.as_str()).collect();
        assert!(secs.contains(&"Notes") && secs.contains(&"Uni"), "{secs:?}");
        let first = ws.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.title == "First").unwrap().id.clone();
        let first = ws.load_other(&first).unwrap();
        assert!(first.doc.as_ref().unwrap().plain_text().contains("[missing image: missing.png]"));
        assert_eq!(first.tags, vec!["a"]);
        assert!(!ws.nb.sections.iter().flat_map(|s| s.pages.iter()).any(|p| p.title.contains("never")));
        assert!(import(&mut ws, &tmp("empty"), 6).is_err());
        std::fs::remove_dir_all(notes).ok();
        std::fs::remove_dir_all(vault).ok();
    }
}

#[cfg(test)]
mod import_bug_tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scrawl-vaultbug-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn titles(ws: &Workspace) -> Vec<String> {
        ws.nb.sections.iter().flat_map(|s| s.pages.iter().map(|p| p.title.clone())).collect()
    }

    #[test]
    fn import_adds_only_the_vault_pages() {
        // the new notebook used to keep a "Lecture notes" template page next to the imported ones
        let (notes, vault) = (tmp("n1"), tmp("v1"));
        std::fs::write(vault.join("A.md"), "a").unwrap();
        std::fs::write(vault.join("B.md"), "b").unwrap();
        let mut ws = Workspace::open_with(&notes, None, None).unwrap();
        let r = import(&mut ws, &vault, 5).unwrap();
        assert_eq!(r.pages, 2);
        let mut t = titles(&ws);
        t.sort();
        assert_eq!(t, vec!["A", "B"]);
        assert_eq!(ws.page.title, "A");
        std::fs::remove_dir_all(notes).ok();
        std::fs::remove_dir_all(vault).ok();
    }

    #[test]
    fn obsidian_path_links_and_md_suffixes_resolve_to_titles() {
        let (notes, vault) = (tmp("n2"), tmp("v2"));
        std::fs::create_dir_all(vault.join("Uni")).unwrap();
        std::fs::write(vault.join("Uni").join("Limits.md"), "---\ntitle: Limits of functions\n---\nx").unwrap();
        std::fs::write(vault.join("Uni").join("Plain.md"), "y").unwrap();
        std::fs::write(vault.join("Top.md"), "[[Uni/Limits]] [[Limits.md|l]] [[Uni/Plain]] [[Uni/Plain.md#Head]]").unwrap();
        let mut ws = Workspace::open_with(&notes, None, None).unwrap();
        import(&mut ws, &vault, 5).unwrap();
        let top = ws.nb.sections.iter().flat_map(|s| s.pages.iter()).find(|p| p.title == "Top").unwrap().id.clone();
        let top = ws.load_other(&top).unwrap();
        let links: Vec<String> = top.doc.unwrap().paras.iter().flat_map(|p| p.runs.clone()).filter_map(|r| if let notedoc::Run::Text { link: Some(Link::Page(t)), .. } = r { Some(t) } else { None }).collect();
        assert_eq!(links, vec!["Limits of functions", "Limits of functions", "Plain", "Plain"]);
        assert_eq!(dangling_links(&ws), 0);
        std::fs::remove_dir_all(notes).ok();
        std::fs::remove_dir_all(vault).ok();
    }

    #[test]
    fn notes_left_out_of_a_huge_vault_are_reported() {
        let (notes, vault) = (tmp("n3"), tmp("v3"));
        for i in 0..MAX_FILES + 3 {
            std::fs::write(vault.join(format!("n{i:05}.md")), "x").unwrap();
        }
        let mut ws = Workspace::open_with(&notes, None, None).unwrap();
        let r = import(&mut ws, &vault, 5).unwrap();
        assert_eq!(r.pages, MAX_FILES);
        assert!(r.skipped.iter().any(|s| s.contains("not imported")), "{:?}", r.skipped.first());
        std::fs::remove_dir_all(notes).ok();
        std::fs::remove_dir_all(vault).ok();
    }

    #[test]
    fn windows_reserved_names_are_not_used_for_files() {
        for n in ["con", "CON", "aux", "Nul", "com1", "LPT9", "con.txt", "aux.", "prn "] {
            let s = safe_name(n);
            let stem = s.split('.').next().unwrap().trim().to_uppercase();
            assert!(!["CON", "PRN", "AUX", "NUL", "COM1", "LPT9"].contains(&stem.as_str()), "{n} -> {s}");
        }
        assert_eq!(safe_name("console"), "console");
    }
}
