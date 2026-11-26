//! Serves files to the content webview: Markdown rendered to HTML, text and source wrapped
//! in <pre>, directories as listings, everything else raw with a content type.
//!
//! Everything served here is untrusted. The content webview has no IPC handler, so pages
//! cannot reach the app. Rendered Markdown, text and listings also get a CSP that
//! forbids scripts.

use crate::paths;
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;
use std::io::Read;
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

fn page(title: &str, class: &str, body: &str) -> Vec<u8> {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><style>{}</style></head><body class=\"{}\">{}</body></html>",
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
    let name = path.rsplit('\\').next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches('\\').rsplit('\\').next().unwrap_or(path)
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

fn is_markdown(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdown" | "mkd" | "mdx")
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
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => return message(StatusCode::NOT_FOUND, "Cannot open", &format!("{path}: {e}")),
    };
    if meta.is_dir() {
        return listing(&path, url_path);
    }
    let ext = ext_of(&path);
    // Rendered views of a request that isn't a top-level document (e.g. <img src="x.svg">)
    // still get the raw bytes, since only types we render specially are affected.
    if !raw && is_markdown(&ext) {
        return match read_text(&path, meta.len()) {
            Ok(t) => {
                let body = render_markdown(&t);
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
    match read_text(&path, meta.len()) {
        Ok(t) => html_page(
            StatusCode::OK,
            file_name(&path),
            "text",
            &format!("<pre class=\"text\">{}</pre>", escape(&t)),
        ),
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

pub fn render_markdown(src: &str) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_DEFINITION_LIST;
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
    let mut out = String::with_capacity(src.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut out, events.into_iter());
    out
}
