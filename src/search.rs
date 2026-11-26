//! Search the root folders for documents matching some words: file names, paths and
//! contents, including text inside Office files (.docx/.xlsx/.pptx/.vsdx, OpenDocument),
//! PDFs and, roughly, the old binary Office formats.
//!
//! No index on disk: folders are walked per search (with the tree's filters) and extracted
//! text is cached in memory by path, size and modification time, so later searches are fast.
//! Ranking favours documents that match every word, then name over path over contents.

use crate::filters::Filters;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

const MAX_FILES: usize = 50_000;
const MAX_RESULTS: usize = 60;
const MAX_TEXT_FILE: u64 = 8 << 20;
const MAX_DOC_FILE: u64 = 60 << 20;
const THREADS: usize = 8;

#[derive(Serialize, Debug)]
pub struct Hit {
    pub path: String,
    pub name: String,
    /// The folder, relative to its root (with the root's name first).
    pub folder: String,
    pub score: f64,
    /// Text around the best match: (text, is_match) pieces.
    pub snippet: Vec<(String, bool)>,
    /// Query words found in the name (true) for each query word, else in path or contents.
    pub in_name: bool,
}

/// Lower-case word stems, so "Capabilities" finds "capability" and "appendices" "appendix".
fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    let w = w.strip_suffix("'s").unwrap_or(&w).to_string();
    let n = w.chars().count();
    if n > 4 && w.ends_with("ices") {
        return format!("{}ix", &w[..w.len() - 4]); // appendices -> appendix, indices -> indix
    }
    if n > 4 && w.ends_with("ies") {
        return format!("{}y", &w[..w.len() - 3]);
    }
    if n > 4 && (w.ends_with("ches") || w.ends_with("shes") || w.ends_with("xes") || w.ends_with("sses")) {
        return w[..w.len() - 2].to_string();
    }
    if n > 3 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") && !w.ends_with("is") {
        return w[..w.len() - 1].to_string();
    }
    w
}

/// Words of `text` with their byte ranges.
fn words(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut it = text.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some(&(_, c)) = it.peek() {
            if c.is_alphanumeric() {
                break;
            }
            it.next();
        }
        let (start, _) = *it.peek()?;
        let mut end = start;
        while let Some(&(i, c)) = it.peek() {
            if !(c.is_alphanumeric() || c == '\'') {
                break;
            }
            end = i + c.len_utf8();
            it.next();
        }
        Some((start, end, &text[start..end]))
    })
}

fn terms_of(query: &str) -> Vec<String> {
    let mut t: Vec<String> = words(query).map(|(_, _, w)| stem(w)).filter(|w| !w.is_empty()).collect();
    t.dedup();
    t
}

/// Does a word (already stemmed) match a query term? Exact stem, or a prefix for longer terms
/// ("config" finds "configuration").
fn matches(word: &str, term: &str) -> bool {
    word == term || (term.chars().count() >= 4 && word.starts_with(term))
}

// ----- text extraction ----------------------------------------------------------------

type Sig = (u64, Option<SystemTime>);

fn cache() -> &'static Mutex<HashMap<String, (Sig, Arc<str>)>> {
    static C: OnceLock<Mutex<HashMap<String, (Sig, Arc<str>)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn ext_of(path: &str) -> String {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The searchable text of a file (cached), or None if it has none.
fn text_of(path: &str, meta: &std::fs::Metadata) -> Option<Arc<str>> {
    let sig = (meta.len(), meta.modified().ok());
    if let Some((s, t)) = cache().lock().ok()?.get(path) {
        if *s == sig {
            return Some(t.clone());
        }
    }
    let text: Arc<str> = extract(path, meta.len()).unwrap_or_default().into();
    if let Ok(mut c) = cache().lock() {
        c.insert(path.to_string(), (sig, text.clone()));
    }
    Some(text)
}

fn extract(path: &str, len: u64) -> Option<String> {
    let ext = ext_of(path);
    let xml_parts: &[&str] = match ext.as_str() {
        "docx" | "docm" | "dotx" | "dotm" => &["word/document.xml", "word/footnotes.xml", "word/header", "word/footer"],
        "xlsx" | "xlsm" | "xltx" => &["xl/sharedStrings.xml", "xl/workbook.xml"],
        "pptx" | "pptm" | "ppsx" => &["ppt/slides/slide", "ppt/notesSlides/"],
        "vsdx" | "vsdm" => &["visio/pages/page"],
        "odt" | "ods" | "odp" | "odg" => &["content.xml"],
        _ => &[],
    };
    if len > MAX_DOC_FILE {
        return None;
    }
    if !xml_parts.is_empty() {
        return zip_text(path, xml_parts);
    }
    match ext.as_str() {
        "pdf" => {
            let bytes = std::fs::read(path).ok()?;
            // The PDF parser can panic on unusual files; treat that as "no text".
            std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes).ok()).ok().flatten()
        }
        "doc" | "xls" | "ppt" | "vsd" | "msg" | "pub" => binary_strings(&std::fs::read(path).ok()?),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "avif" | "zip" | "7z" | "gz" | "exe" | "dll" | "hex"
        | "bin" | "o" | "obj" | "lib" | "a" | "so" | "pdb" | "mp3" | "mp4" | "wav" => None,
        _ => {
            if len > MAX_TEXT_FILE {
                return None;
            }
            let mut b = Vec::new();
            std::fs::File::open(path).ok()?.take(MAX_TEXT_FILE).read_to_end(&mut b).ok()?;
            if b.iter().take(8192).any(|&c| c == 0) {
                return None;
            }
            Some(String::from_utf8_lossy(&b).into_owned())
        }
    }
}

/// Text of the XML parts (by name prefix) of an Office Open XML / OpenDocument file.
fn zip_text(path: &str, parts: &[&str]) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).ok()?;
    let mut names: Vec<String> =
        z.file_names().filter(|n| n.ends_with(".xml") && parts.iter().any(|p| n.starts_with(p))).map(String::from).collect();
    names.sort_by_key(|n| (n.len(), n.clone())); // slide2 before slide10
    let mut out = String::new();
    for n in names {
        let mut xml = String::new();
        if z.by_name(&n).ok()?.take(64 << 20).read_to_string(&mut xml).is_ok() {
            xml_to_text(&xml, &mut out);
            out.push('\n');
        }
    }
    Some(out)
}

/// Strip XML tags. Text runs inside a paragraph join without spaces (Word splits words
/// across runs); paragraphs, cells and line breaks become separators.
fn xml_to_text(xml: &str, out: &mut String) {
    let mut rest = xml;
    // Word field codes (PAGEREF _Toc...) and deleted tracked changes aren't document text.
    let mut skip = false;
    while let Some(lt) = rest.find('<') {
        if !skip {
            decode_entities(&rest[..lt], out);
        }
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        let name = tag.trim_start_matches('/').split([' ', '/', '\t', '\n']).next().unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        let closing = tag.starts_with('/');
        let empty = tag.ends_with('/');
        if matches!(local, "instrText" | "delText" | "delInstrText") && !empty {
            skip = !closing;
        }
        if (closing && matches!(local, "p" | "si" | "tc" | "row" | "c" | "h" | "Text" | "sheet" | "cp" | "pp"))
            || (empty && matches!(local, "tab" | "br" | "cr" | "s" | "line-break" | "sheet"))
        {
            out.push(if local == "tab" || local == "c" || local == "tc" || local == "s" { ' ' } else { '\n' });
        }
        if local == "sheet" && !closing {
            // Workbook sheet names: <sheet name="Budget" .../>
            if let Some(v) = tag.split("name=\"").nth(1).and_then(|s| s.split('"').next()) {
                decode_entities(v, out);
                out.push('\n');
            }
        }
        rest = &rest[lt + gt + 1..];
    }
    decode_entities(rest, out);
}

fn decode_entities(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match after.find(';').filter(|&i| i <= 10) {
            Some(semi) => {
                let ent = &after[..semi];
                let ch = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                    e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
                    _ => None,
                };
                match ch {
                    Some(c) => {
                        out.push(c);
                        rest = &after[semi + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = after;
                    }
                }
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

/// Readable runs from a binary file (old .doc/.xls/...): UTF-16LE and 8-bit text of 4+ letters.
fn binary_strings(b: &[u8]) -> Option<String> {
    let mut out = String::new();
    let printable = |c: u32| (0x20..0x7f).contains(&c) || (0xa0..0x250).contains(&c);
    // UTF-16LE
    let mut run = String::new();
    for pair in b.chunks_exact(2) {
        let c = u16::from_le_bytes([pair[0], pair[1]]) as u32;
        if printable(c) {
            run.push(char::from_u32(c).unwrap_or(' '));
        } else {
            if run.chars().filter(|c| c.is_alphabetic()).count() >= 4 {
                out.push_str(&run);
                out.push('\n');
            }
            run.clear();
        }
    }
    // 8-bit (older Word stores text this way)
    let mut run = Vec::new();
    for &c in b {
        if (0x20..0x7f).contains(&c) {
            run.push(c);
        } else {
            if run.len() >= 6 && run.iter().filter(|c| c.is_ascii_alphabetic()).count() * 2 > run.len() {
                out.push_str(&String::from_utf8_lossy(&run));
                out.push('\n');
            }
            run.clear();
        }
    }
    (!out.is_empty()).then_some(out)
}

// ----- walking and ranking ------------------------------------------------------------

fn walk(roots: &[String], filters: &Filters) -> Vec<(String, String, std::fs::Metadata)> {
    let mut files = Vec::new();
    for root in roots {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            let keep = filters.for_dir(&dir);
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(e.path()) else { continue };
                let p = e.path();
                if !keep.keep(&p, &name, meta.is_dir(), crate::platform::is_hidden(&e, &name)) {
                    continue;
                }
                let ps = p.to_string_lossy().into_owned();
                if meta.is_dir() {
                    stack.push(ps);
                } else {
                    files.push((root.clone(), ps, meta));
                    if files.len() >= MAX_FILES {
                        return files;
                    }
                }
            }
        }
    }
    files
}

struct Scored {
    path: String,
    root: String,
    /// Per query term: (in name, in folder path, count in contents).
    per_term: Vec<(bool, bool, usize)>,
    text: Option<Arc<str>>,
}

/// Find documents under `roots` matching `query`, best first.
pub fn search(roots: &[String], filters: &Filters, query: &str) -> Vec<Hit> {
    let terms = terms_of(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let files = walk(roots, filters);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found: Mutex<Vec<Scored>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((root, path, meta)) = files.get(i) else { break };
                let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = path.get(root.len()..path.len() - name.len()).unwrap_or("");
                let in_words = |s: &str, t: &str| words(s).any(|(_, _, w)| matches(&stem(w), t));
                let text = text_of(path, meta);
                let mut per_term = Vec::with_capacity(terms.len());
                let mut counts = vec![0usize; terms.len()];
                if let Some(t) = &text {
                    for (_, _, w) in words(t) {
                        let w = stem(w);
                        for (k, term) in terms.iter().enumerate() {
                            if matches(&w, term) {
                                counts[k] += 1;
                            }
                        }
                    }
                }
                for (k, term) in terms.iter().enumerate() {
                    per_term.push((in_words(&name, term), in_words(folder, term), counts[k]));
                }
                if per_term.iter().any(|&(a, b, c)| a || b || c > 0) {
                    if let Ok(mut f) = found.lock() {
                        f.push(Scored { path: path.clone(), root: root.clone(), per_term, text });
                    }
                }
            });
        }
    });
    let found = found.into_inner().unwrap_or_default();
    // Rarer words count for more (inverse document frequency over the files searched).
    let n = files.len().max(1) as f64;
    let idf: Vec<f64> = (0..terms.len())
        .map(|k| {
            let df = found.iter().filter(|s| { let (a, b, c) = s.per_term[k]; a || b || c > 0 }).count() as f64;
            (1.0 + n / (1.0 + df)).ln()
        })
        .collect();
    let mut hits: Vec<Hit> = found
        .into_iter()
        .map(|s| {
            let mut score = 0.0;
            let mut covered = 0;
            for (k, &(in_name, in_path, count)) in s.per_term.iter().enumerate() {
                let mut t = 0.0;
                if in_name {
                    t += 6.0;
                }
                if in_path {
                    t += 2.0;
                }
                if count > 0 {
                    t += 1.0 + (count as f64).ln();
                }
                if t > 0.0 {
                    covered += 1;
                }
                score += t * idf[k];
            }
            // Matching every word matters most.
            let cover = covered as f64 / terms.len() as f64;
            score *= cover * cover;
            let name = Path::new(&s.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let root_name = Path::new(&s.root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.root.clone());
            let folder = format!("{root_name}{}", s.path.get(s.root.len()..s.path.len() - name.len()).unwrap_or(""));
            let folder = folder.trim_end_matches(['\\', '/']).to_string();
            Hit {
                snippet: s.text.as_deref().map(|t| snippet(t, &terms)).unwrap_or_default(),
                in_name: s.per_term.iter().any(|p| p.0),
                path: s.path,
                name,
                folder,
                score,
            }
        })
        .collect();
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    hits.truncate(MAX_RESULTS);
    hits
}

/// About 30 words around the place where the most different query words occur together.
fn snippet(text: &str, terms: &[String]) -> Vec<(String, bool)> {
    const SPAN: usize = 30;
    let ws: Vec<(usize, usize, Option<usize>)> =
        words(text).map(|(s, e, w)| { let st = stem(w); (s, e, terms.iter().position(|t| matches(&st, t))) }).collect();
    let hits: Vec<usize> = (0..ws.len()).filter(|&i| ws[i].2.is_some()).collect();
    if hits.is_empty() {
        return Vec::new();
    }
    // The window starting a few words before a hit that holds the most distinct terms.
    let mut best = (0, hits[0]);
    for &h in &hits {
        let end = (h + SPAN).min(ws.len());
        let mut seen = vec![false; terms.len()];
        for w in &ws[h..end] {
            if let Some(k) = w.2 {
                seen[k] = true;
            }
        }
        let n = seen.iter().filter(|&&s| s).count();
        if n > best.0 {
            best = (n, h);
        }
    }
    let start = best.1.saturating_sub(6);
    let end = (start + SPAN).min(ws.len());
    let mut out = Vec::new();
    let mut pos = ws[start].0;
    if start > 0 {
        out.push(("\u{2026}".to_string(), false));
    }
    for w in &ws[start..end] {
        if w.2.is_some() {
            out.push((clean(&text[pos..w.0]), false));
            out.push((text[w.0..w.1].to_string(), true));
            pos = w.1;
        }
    }
    out.push((clean(&text[pos..ws[end - 1].1]), false));
    if end < ws.len() {
        out.push(("\u{2026}".to_string(), false));
    }
    out.retain(|(s, _)| !s.is_empty());
    out
}

/// Whitespace runs (newlines from documents) as single spaces.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stems() {
        assert_eq!(stem("Capabilities"), "capability");
        assert_eq!(stem("capability"), "capability");
        assert_eq!(stem("Appendices"), "appendix");
        assert_eq!(stem("appendix"), "appendix");
        assert_eq!(stem("boxes"), "box");
        assert_eq!(stem("status"), "status");
        assert_eq!(stem("analysis"), "analysis");
    }
    #[test]
    fn xml() {
        let mut out = String::new();
        xml_to_text(r#"<w:p><w:r><w:t>Capab</w:t></w:r><w:r><w:t>ilities &amp; more</w:t></w:r></w:p><w:p><w:r><w:t>Next</w:t></w:r></w:p>"#, &mut out);
        assert_eq!(out, "Capabilities & more\nNext\n");
        let mut out = String::new();
        xml_to_text(r#"<w:p><w:r><w:instrText> PAGEREF _Toc1 \h </w:instrText></w:r><w:r><w:t>4</w:t></w:r></w:p>"#, &mut out);
        assert_eq!(out, "4\n");
    }
    #[test]
    fn snip() {
        let t = "Intro text. The capabilities appendix lists everything the radio can do.";
        let s = snippet(t, &terms_of("capabilities appendix"));
        let marked: Vec<_> = s.iter().filter(|p| p.1).map(|p| p.0.as_str()).collect();
        assert_eq!(marked, ["capabilities", "appendix"]);
    }
}
