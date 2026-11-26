//! Serves files to the content webview: Markdown rendered to HTML, text and source wrapped
//! in <pre>, directories as listings, everything else raw with a content type.
//!
//! Everything served here is untrusted. The content webview has no IPC handler, so pages
//! cannot reach the app. Rendered Markdown, text and listings also get a CSP that
//! forbids scripts.

use crate::paths;
use crate::highlight;
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;
use wry::http::{header, Response, StatusCode};

pub type Resp = Response<Cow<'static, [u8]>>;

const MAX_TEXT: u64 = 20 * 1024 * 1024;
const STRICT_CSP: &str = "default-src 'self' data: blob:; script-src 'none'; style-src 'self' 'unsafe-inline'; img-src * data: blob:; object-src 'none'; frame-src 'self'";

const DOC_CSS: &str = include_str!("../ui/doc.css");

fn respond(status: StatusCode, ctype: &str, csp: Option<&str>, body: Vec<u8>) -> Resp {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(c) = csp {
        b = b.header("Content-Security-Policy", c);
    }
    b.body(Cow::Owned(body)).unwrap()
}

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// How documents use the pane's width: "narrow", "wide" or "full" (see doc.css).
static WIDTH: RwLock<&'static str> = RwLock::new("wide");

pub fn set_width(mode: &str) {
    let m = match mode {
        "narrow" => "narrow",
        "full" => "full",
        _ => "wide",
    };
    if let Ok(mut w) = WIDTH.write() {
        *w = m;
    }
}

fn width_class() -> String {
    format!("w-{}", WIDTH.read().map_or("wide", |w| *w))
}

fn page(title: &str, class: &str, body: &str) -> Vec<u8> {
    format!(
        "<!doctype html><html class=\"{}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{}</style></head><body class=\"{}\">{}</body></html>",
        width_class(),
        escape(title),
        DOC_CSS,
        class,
        body
    )
    .into_bytes()
}

fn html_page(status: StatusCode, title: &str, class: &str, body: &str) -> Resp {
    respond(status, "text/html; charset=utf-8", Some(STRICT_CSP), page(title, class, body))
}

fn message(status: StatusCode, title: &str, msg: &str) -> Resp {
    html_page(
        status,
        title,
        "message",
        &format!("<div class=\"msg\"><h2>{}</h2><p>{}</p></div>", escape(title), escape(msg)),
    )
}

fn ext_of(path: &str) -> String {
    let name = crate::platform::file_name(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn file_name(path: &str) -> &str {
    crate::platform::file_name(path)
}

/// Content type for files served raw.
fn raw_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "html" | "htm" | "xhtml" => "text/html; charset=utf-8",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        _ => return None,
    })
}

/// Whether a file has both a rendered view and a source view worth switching between.
pub fn has_source_view(path: &str) -> bool {
    let ext = ext_of(path);
    is_markdown(&ext) || matches!(ext.as_str(), "html" | "htm" | "xhtml" | "svg")
}

/// True if a content URL asks for the source view.
pub fn is_source_url(url: &str) -> bool {
    url.split('#').next().and_then(|u| u.split_once('?')).map_or(false, |(_, q)| q.split('&').any(|p| p == "source"))
}

fn is_markdown(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd" | "mdx")
}

/// Whether Condor shows this file itself; false for binary files (.xlsx, .doc, ...) and
/// text too large to show, which get Open / Open with... instead. Reads at most 8 KB.
pub fn previewable(path: &str) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return true };
    let ext = ext_of(path);
    if meta.is_dir() || is_markdown(&ext) || raw_type(&ext).is_some() {
        return true;
    }
    if meta.len() > MAX_TEXT {
        return false;
    }
    let mut head = Vec::with_capacity(8192);
    match std::fs::File::open(path).and_then(|f| f.take(8192).read_to_end(&mut head)) {
        Ok(_) => !head.contains(&0),
        Err(_) => true, // the page itself explains the error
    }
}

/// Handle a request for `url_path` (path part of the URL), with optional query string.
pub fn serve(url_path: &str, query: Option<&str>) -> Resp {
    let Some(path) = paths::url_path_to_path(url_path) else {
        return html_page(
            StatusCode::OK,
            "Condor",
            "message",
            "<div class=\"msg welcome\"><h2>Condor</h2><p>Choose a file in the tree to view it.</p></div>",
        );
    };
    let raw = query.map_or(false, |q| q.split('&').any(|p| p == "raw"));
    let source = query.map_or(false, |q| q.split('&').any(|p| p == "source"));
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return message(StatusCode::NOT_FOUND, "Cannot open", &format!("{path}: {e}")),
    };
    if meta.is_dir() {
        return listing(&path, url_path);
    }
    let ext = ext_of(&path);
    if source {
        return text_view(&path, &ext, meta.len());
    }
    // Rendered views of a request that isn't a top-level document (e.g. <img src="x.svg">)
    // still get the raw bytes, since only types we render specially are affected.
    if !raw && is_markdown(&ext) {
        return match read_text(&path, meta.len()) {
            Ok(t) => {
                let body = render_markdown(&t, Path::new(&path).parent());
                html_page(StatusCode::OK, file_name(&path), "markdown-body", &body)
            }
            Err(r) => r,
        };
    }
    // HTML pages run as ordinary web pages (scripts allowed) in the isolated content webview.
    if let Some(ct) = raw_type(&ext) {
        return match std::fs::read(&path) {
            Ok(b) => respond(StatusCode::OK, ct, None, b),
            Err(e) => message(StatusCode::INTERNAL_SERVER_ERROR, "Read error", &format!("{path}: {e}")),
        };
    }
    if raw {
        return match std::fs::read(&path) {
            Ok(b) => respond(StatusCode::OK, "text/plain; charset=utf-8", Some(STRICT_CSP), b),
            Err(e) => message(StatusCode::INTERNAL_SERVER_ERROR, "Read error", &format!("{path}: {e}")),
        };
    }
    text_view(&path, &ext, meta.len())
}

/// A text file as (highlighted, where the language is known) source.
fn text_view(path: &str, ext: &str, len: u64) -> Resp {
    match read_text(path, len) {
        Ok(t) => {
            let body = match highlight::for_file(ext, &t).and_then(|s| highlight::to_html(&t, s)) {
                Some(h) => format!("{}<pre class=\"text src\">{}</pre>", highlight::style_tag(), h),
                None => format!("<pre class=\"text\">{}</pre>", escape(&t)),
            };
            html_page(StatusCode::OK, file_name(path), "text", &body)
        }
        Err(r) => r,
    }
}

/// Read a file as text; returns an explanatory page for binary or huge files.
fn read_text(path: &str, len: u64) -> Result<String, Resp> {
    if len > MAX_TEXT {
        return Err(message(
            StatusCode::OK,
            "File too large",
            &format!("{} is {} MB; Condor shows text files up to {} MB.", file_name(path), len >> 20, MAX_TEXT >> 20),
        ));
    }
    let mut buf = Vec::with_capacity(len as usize);
    if let Err(e) = std::fs::File::open(path).and_then(|f| f.take(MAX_TEXT).read_to_end(&mut buf)) {
        return Err(message(StatusCode::INTERNAL_SERVER_ERROR, "Read error", &format!("{path}: {e}")));
    }
    let head = &buf[..buf.len().min(8192)];
    if head.contains(&0) {
        return Err(message(
            StatusCode::OK,
            "Binary file",
            &format!("{} ({} bytes) looks like a binary file.", file_name(path), len),
        ));
    }
    let s = String::from_utf8_lossy(&buf);
    Ok(s.strip_prefix('\u{feff}').unwrap_or(&s).to_string())
}

fn listing(path: &str, url_path: &str) -> Resp {
    let mut entries: Vec<(bool, String)> = match std::fs::read_dir(path) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| {
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    || (e.file_type().map(|t| t.is_symlink()).unwrap_or(false)
                        && e.path().is_dir());
                (is_dir, e.file_name().to_string_lossy().into_owned())
            })
            .collect(),
        Err(e) => return message(StatusCode::INTERNAL_SERVER_ERROR, "Cannot list folder", &format!("{path}: {e}")),
    };
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let base = if url_path.ends_with('/') { url_path.to_string() } else { format!("{url_path}/") };
    let mut body = format!("<h1>{}</h1><ul class=\"listing\">", escape(path));
    for (d, n) in entries {
        let mut href = base.clone();
        href.push_str(&paths::path_to_url_path(&n)[1..]);
        body.push_str(&format!(
            "<li class=\"{}\"><a href=\"{}\">{}{}</a></li>",
            if d { "dir" } else { "file" },
            escape(&href),
            escape(&n),
            if d { "/" } else { "" }
        ));
    }
    body.push_str("</ul>");
    html_page(StatusCode::OK, file_name(path), "markdown-body", &body)
}

/// GitHub-style heading anchor: lowercase, spaces to '-', drop most punctuation.
fn slugify(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                Some(c.to_lowercase().next().unwrap_or(c))
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

pub fn render_markdown(src: &str, dir: Option<&Path>) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS;
    let mut events: Vec<Event> = Parser::new_ext(src, opts).collect();

    // Give headings GitHub-style ids so in-page links (#section) work.
    let mut used: std::collections::HashMap<String, usize> = Default::default();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { id: None, .. }) = &events[i] {
            let mut text = String::new();
            let mut j = i + 1;
            while j < events.len() && !matches!(events[j], Event::End(TagEnd::Heading(_))) {
                if let Event::Text(t) | Event::Code(t) = &events[j] {
                    text.push_str(t);
                }
                j += 1;
            }
            let base = slugify(&text);
            let n = used.entry(base.clone()).or_insert(0);
            let slug = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
            *n += 1;
            if let Event::Start(Tag::Heading { id, .. }) = &mut events[i] {
                *id = Some(CowStr::from(slug));
            }
            i = j;
        }
        i += 1;
    }
    let events = match dir {
        Some(d) => link_code_paths(events, d),
        None => events,
    };
    let events = front_matter(events);
    let (events, coloured) = highlight_code_blocks(events);
    let mut out = String::with_capacity(src.len() * 3 / 2);
    if coloured {
        out.push_str(highlight::style_tag());
    }
    pulldown_cmark::html::push_html(&mut out, events.into_iter());
    out
}

/// Front matter (`---` YAML or `+++` TOML at the top) as a small key/value table, or as
/// preformatted text if it isn't simple `key: value` lines.
fn front_matter(events: Vec<Event>) -> Vec<Event> {
    let mut out = Vec::with_capacity(events.len());
    let mut it = events.into_iter();
    while let Some(ev) = it.next() {
        if !matches!(ev, Event::Start(Tag::MetadataBlock(_))) {
            out.push(ev);
            continue;
        }
        let mut text = String::new();
        for e in it.by_ref() {
            match e {
                Event::End(TagEnd::MetadataBlock(_)) => break,
                Event::Text(t) => text.push_str(&t),
                _ => {}
            }
        }
        out.push(Event::Html(CowStr::from(front_matter_html(&text))));
    }
    out
}

fn front_matter_html(text: &str) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // `key: value` (YAML) or `key = value` (TOML) at the start of a line.
        let key_line = (!line.starts_with([' ', '\t', '-']))
            .then(|| line.split_once([':', '=']))
            .flatten()
            .filter(|(k, _)| !k.trim().is_empty() && !k.trim().contains(' '));
        match key_line {
            Some((k, v)) => rows.push((k.trim().to_string(), v.trim().trim_matches('"').to_string())),
            // Continuation lines (lists, indented or wrapped values) join the previous key.
            None => match rows.last_mut() {
                Some((_, v)) => {
                    if !v.is_empty() {
                        v.push('\n');
                    }
                    v.push_str(t);
                }
                None => return format!("<pre class=\"front-matter\">{}</pre>", escape(text)),
            },
        }
    }
    let mut html = String::from("<table class=\"front-matter\"><tbody>");
    for (k, v) in rows {
        let v = escape(&v).replace('\n', "<br>");
        html.push_str(&format!("<tr><th>{}</th><td>{}</td></tr>", escape(&k), v));
    }
    html.push_str("</tbody></table>\n");
    html
}

/// Replace fenced code blocks whose language we know with highlighted HTML.
fn highlight_code_blocks(events: Vec<Event>) -> (Vec<Event>, bool) {
    let mut out = Vec::with_capacity(events.len());
    let mut coloured = false;
    let mut it = events.into_iter();
    while let Some(ev) = it.next() {
        let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) = &ev else {
            out.push(ev);
            continue;
        };
        let Some(syntax) = highlight::for_lang(lang) else {
            out.push(ev);
            continue;
        };
        let mut inner = Vec::new();
        let mut code = String::new();
        for e in it.by_ref() {
            if matches!(e, Event::End(TagEnd::CodeBlock)) {
                break;
            }
            if let Event::Text(t) = &e {
                code.push_str(t);
            }
            inner.push(e);
        }
        match highlight::to_html(&code, syntax) {
            Some(h) => {
                coloured = true;
                out.push(Event::Html(CowStr::from(format!(
                    "<pre><code class=\"language-{}\">{}</code></pre>\n",
                    escape(lang),
                    h
                ))));
            }
            None => {
                out.push(ev);
                out.extend(inner);
                out.push(Event::End(TagEnd::CodeBlock));
            }
        }
    }
    (out, coloured)
}

/// The tree's root folders, so `src/main.rs` in a document can be found relative to the
/// project as well as to the document's own folder.
static ROOTS: RwLock<Vec<String>> = RwLock::new(Vec::new());

pub fn set_roots(roots: &[String]) {
    if let Ok(mut r) = ROOTS.write() {
        *r = roots.to_vec();
    }
}

/// At most this many file lookups per document, so a long one stays quick over \\wsl$.
const MAX_LOOKUPS: usize = 300;

/// If inline code looks like a relative file path (`src/main.rs`, `../README.md`,
/// `ui/index.html:42`), the path part, without any `:line[:col]` suffix.
fn path_like(code: &str) -> Option<&str> {
    let mut p = code.trim();
    while let Some((head, tail)) = p.rsplit_once(':') {
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        p = head;
    }
    let chars_ok = p.chars().all(|c| c.is_alphanumeric() || "._-+@/\\".contains(c));
    let has_ext = || {
        p.rsplit_once('.').map_or(false, |(stem, ext)| {
            !stem.is_empty()
                && (1..=10).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        })
    };
    let ok = !p.is_empty()
        && p.len() <= 260
        && chars_ok
        && !p.starts_with(['/', '\\'])
        && !p.contains("//")
        && (p.contains(['/', '\\']) || has_ext());
    ok.then_some(p)
}

/// `base` joined with a relative path, with `.` and `..` resolved (content URLs can't
/// contain `..`). None if it would climb above the base's root.
fn join_rel(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in base.join(rel.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR)).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    return None;
                }
                out.pop();
            }
            c => out.push(c),
        }
    }
    Some(out)
}

/// Turn inline code naming an existing file or folder into a link to it. Looked up relative to the
/// document's folder, then to the root folder(s) containing it.
fn link_code_paths<'a>(events: Vec<Event<'a>>, dir: &Path) -> Vec<Event<'a>> {
    let dir_s = dir.to_string_lossy();
    let mut bases = vec![dir.to_path_buf()];
    if let Ok(roots) = ROOTS.read() {
        for r in roots.iter().filter(|r| crate::platform::is_under(&dir_s, r)) {
            if crate::platform::path_key(r) != crate::platform::path_key(&dir_s) {
                bases.push(PathBuf::from(r));
            }
        }
    }
    let mut lookups = 0;
    let mut in_link = 0usize;
    let mut out = Vec::with_capacity(events.len());
    for ev in events {
        match &ev {
            Event::Start(Tag::Link { .. }) => in_link += 1,
            Event::End(TagEnd::Link) => in_link = in_link.saturating_sub(1),
            Event::Code(t) if in_link == 0 && lookups < MAX_LOOKUPS => {
                if let Some(rel) = path_like(t) {
                    let found = bases.iter().filter_map(|b| join_rel(b, rel)).find(|p| {
                        lookups += 1;
                        lookups <= MAX_LOOKUPS && std::fs::metadata(p).is_ok()
                    });
                    if let Some(p) = found {
                        let href = paths::path_to_url_path(&p.to_string_lossy());
                        out.push(Event::Html(CowStr::from(format!("<a class=\"path\" href=\"{}\">", escape(&href)))));
                        out.push(ev);
                        out.push(Event::Html(CowStr::from("</a>")));
                        continue;
                    }
                }
            }
            _ => {}
        }
        out.push(ev);
    }
    out
}

#[cfg(test)]
mod link_tests {
    use super::*;
    #[test]
    fn path_like_cases() {
        assert_eq!(path_like("src/main.rs"), Some("src/main.rs"));
        assert_eq!(path_like("ui/index.html:42:7"), Some("ui/index.html"));
        assert_eq!(path_like("README.md"), Some("README.md"));
        assert_eq!(path_like("tr-rocket/rcp-102819/"), Some("tr-rocket/rcp-102819/"));
        assert_eq!(path_like("../x/y"), Some("../x/y"));
        for no in ["v1.2", "a b.md", "/etc/passwd", "https://x.org/a", "foo", "x.", ".", "~/a.md"] {
            assert_eq!(path_like(no), None, "{no}");
        }
    }
    #[test]
    fn join() {
        let b = Path::new("/a/b");
        assert_eq!(join_rel(b, "../c/d.md"), Some(PathBuf::from("/a/c/d.md")));
        assert_eq!(join_rel(b, "./x"), Some(PathBuf::from("/a/b/x")));
        assert_eq!(join_rel(b, "../../../x"), None);
    }
}
