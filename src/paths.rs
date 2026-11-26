//! Mapping between filesystem paths and URLs served to the content webview.
//!
//! A path becomes the URL path of `condorfile://localhost/...`, mirroring the directory
//! structure so that relative links and images in documents resolve naturally. The
//! OS-specific mapping is in `platform`.

pub const FILE_SCHEME: &str = "condorfile";
pub const UI_SCHEME: &str = "condorui";

/// The origin the content webview navigates to for the file scheme.
pub fn file_origin() -> String {
    crate::platform::scheme_origin(FILE_SCHEME)
}

pub fn encode_segment(seg: &str, out: &mut String) {
    for b in seg.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "-._~:".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub use crate::platform::{path_to_url_path, url_path_to_path};

pub fn path_to_url(path: &str) -> String {
    format!("{}{}", file_origin(), path_to_url_path(path))
}

/// Full content-webview URL -> path, if it is one of ours.
/// The path a `file:` URL names: file:///C:/a%20b.md -> C:\a b.md,
/// file://server/share/x -> \\server\share\x (on Windows). None for other URLs.
pub fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file:").or_else(|| url.strip_prefix("FILE:"))?;
    let rest = rest.split(['#', '?']).next().unwrap_or("");
    let rest = percent_decode(rest);
    let (host, path) = match rest.strip_prefix("//") {
        Some(r) => r.split_once('/').map_or((r, ""), |(h, p)| (h, p)),
        None => ("", rest.trim_start_matches('/')),
    };
    let host = if host.eq_ignore_ascii_case("localhost") { "" } else { host };
    if path.is_empty() && host.is_empty() {
        return None;
    }
    let p = if cfg!(windows) {
        let p = path.replace('/', "\\");
        if host.is_empty() { p } else { format!("\\\\{host}\\{p}") }
    } else if host.is_empty() {
        format!("/{path}")
    } else {
        return None;
    };
    Some(crate::platform::normalise_path(&p))
}

pub fn url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix(&file_origin())?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    url_path_to_path(rest)
}
