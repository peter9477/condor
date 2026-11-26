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
pub fn url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix(&file_origin())?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    url_path_to_path(rest)
}
