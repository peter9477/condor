//! Mapping between filesystem paths and URLs served to the content webview.
//!
//! A path becomes the URL path of `condorfile://localhost/...` (which WebView2 sees as
//! `http://condorfile.localhost/...`), mirroring the directory structure so that relative
//! links and images in documents resolve naturally:
//!
//!   C:\docs\a b.md                -> /C:/docs/a%20b.md
//!   \\wsl$\Debian\code\README.md  -> /UNC/wsl%24/Debian/code/README.md

pub const FILE_SCHEME: &str = "condorfile";
pub const UI_SCHEME: &str = "condorui";

/// The prefix WebView2 actually navigates to for the file scheme (wry's Windows workaround).
pub fn file_origin() -> String {
    format!("http://{FILE_SCHEME}.localhost")
}

fn encode_segment(seg: &str, out: &mut String) {
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

/// Windows path string -> URL path (starting with '/').
pub fn path_to_url_path(path: &str) -> String {
    let p = path.replace('/', "\\");
    let mut out = String::new();
    let rest: &str = if let Some(r) = p.strip_prefix("\\\\?\\UNC\\").or_else(|| p.strip_prefix("\\\\")) {
        out.push_str("/UNC");
        r
    } else if let Some(r) = p.strip_prefix("\\\\?\\") {
        r
    } else {
        &p
    };
    for seg in rest.split('\\').filter(|s| !s.is_empty()) {
        out.push('/');
        encode_segment(seg, &mut out);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

pub fn path_to_url(path: &str) -> String {
    format!("{}{}", file_origin(), path_to_url_path(path))
}

/// URL path (already stripped of query/fragment) -> Windows path string. None for "/".
pub fn url_path_to_path(url_path: &str) -> Option<String> {
    let segs: Vec<String> = url_path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect();
    if segs.is_empty() {
        return None;
    }
    if segs.iter().any(|s| s == ".." || s.contains('\\')) {
        return None;
    }
    if segs[0] == "UNC" {
        if segs.len() < 3 {
            return None;
        }
        Some(format!("\\\\{}", segs[1..].join("\\")))
    } else {
        let mut p = segs.join("\\");
        if segs.len() == 1 {
            p.push('\\'); // "C:" -> "C:\"
        }
        Some(p)
    }
}

/// Full content-webview URL -> Windows path, if it is one of ours.
pub fn url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix(&file_origin())?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    url_path_to_path(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        for p in [
            "C:\\docs\\a b.md",
            "\\\\wsl$\\Debian\\code\\README.md",
            "\\\\10.0.10.124\\Public\\x#y%z.md",
            "D:\\",
        ] {
            let u = path_to_url(p);
            assert_eq!(url_to_path(&u).as_deref(), Some(p), "{u}");
        }
        assert_eq!(path_to_url_path("\\\\wsl$\\Debian\\a"), "/UNC/wsl%24/Debian/a");
    }
}
