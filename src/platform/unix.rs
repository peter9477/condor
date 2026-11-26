//! macOS and Linux. Not yet built or tested: Condor only ships for Windows so far.
//! wry uses WKWebView on macOS and WebKitGTK on Linux.

use crate::paths::{encode_segment, percent_decode};
use std::path::PathBuf;

/// Subfolder of `local_data_dir` for the webview's own data.
pub const WEBVIEW_DATA: &str = "webview";
pub const SEP: char = '/';
/// macOS volumes are usually case-insensitive, but comparing exactly is the safe default.
pub const CASE_INSENSITIVE: bool = false;

pub fn normalise_path(p: &str) -> String {
    let p = p.trim().trim_matches('"');
    let p = if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest).to_string_lossy().into_owned()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        std::env::current_dir().map(|c| c.join(p).to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string())
    };
    if p.len() > 1 { p.trim_end_matches('/').to_string() } else { p }
}

/// Path -> URL path: the path itself, percent-encoded per segment.
pub fn path_to_url_path(path: &str) -> String {
    let mut out = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        encode_segment(seg, &mut out);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// URL path -> path. None for "/" (the welcome page) and for anything unsafe.
pub fn url_path_to_path(url_path: &str) -> Option<String> {
    let segs: Vec<String> = url_path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    if segs.is_empty() || segs.iter().any(|s| s == ".." || s.contains('/')) {
        return None;
    }
    Some(format!("/{}", segs.join("/")))
}

pub fn scheme_origin(scheme: &str) -> String {
    format!("{scheme}://localhost")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn xdg(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(default))
}

pub fn config_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Condor")
    } else {
        xdg("XDG_CONFIG_HOME", ".config").join("condor")
    }
}

pub fn local_data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Caches/Condor")
    } else {
        xdg("XDG_DATA_HOME", ".local/share").join("condor")
    }
}

pub fn is_hidden(_entry: &std::fs::DirEntry, name: &str) -> bool {
    name.starts_with('.')
}

pub fn launch(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

/// Uses the system's own dialog tools, to avoid extra dependencies until a port is real.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    let out = if cfg!(target_os = "macos") {
        let script = format!("POSIX path of (choose folder with prompt \"{}\")", title.replace('"', ""));
        std::process::Command::new("osascript").args(["-e", &script]).output()
    } else {
        std::process::Command::new("zenity").args(["--file-selection", "--directory", "--title", title]).output()
    }
    .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then(|| PathBuf::from(s))
}

/// Nothing to adjust on these platforms (generic so this file doesn't depend on wry).
pub fn tune_ui_webview<B>(b: B) -> B {
    b
}

/// Step a webview's history.
pub fn history_go(wv: &wry::WebView, back: bool) {
    let _ = wv.evaluate_script(if back { "history.back()" } else { "history.forward()" });
}

/// Not known here; the buttons simply stay enabled.
pub fn history_state(_wv: &wry::WebView) -> Option<(bool, bool)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        for p in ["/home/me/a b.md", "/x#y%z.md", "/"] {
            let back = url_path_to_path(&path_to_url_path(p));
            assert_eq!(back.as_deref(), if p == "/" { None } else { Some(p) });
        }
        assert!(crate::platform::is_under("/a/b/c", "/a/b"));
        assert!(!crate::platform::is_under("/a/bc", "/a/b"));
        assert_eq!(normalise_path("/a/b/"), "/a/b");
    }
}
