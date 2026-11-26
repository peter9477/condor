//! Windows: drive-letter and UNC paths (including \\wsl$), WebView2, %APPDATA%.

use crate::paths::{encode_segment, percent_decode};
use std::path::PathBuf;

/// Subfolder of `local_data_dir` for the webview's own data.
pub const WEBVIEW_DATA: &str = "WebView2";
pub const SEP: char = '\\';
pub const CASE_INSENSITIVE: bool = true;

/// Absolute, backslash-separated, without a `\\?\` prefix or trailing separator (except
/// for a drive root such as `C:\`).
pub fn normalise_path(p: &str) -> String {
    let p = p.trim().trim_matches('"').replace('/', "\\");
    let p = if std::path::Path::new(&p).is_absolute() || p.starts_with("\\\\") {
        p
    } else {
        std::env::current_dir()
            .map(|c| c.join(&p).to_string_lossy().into_owned())
            .unwrap_or(p)
    };
    let p = p.strip_prefix("\\\\?\\UNC\\").map(|s| format!("\\\\{s}")).unwrap_or(p);
    let p = p.strip_prefix("\\\\?\\").map(|s| s.to_string()).unwrap_or(p);
    if p.len() > 3 && p.ends_with('\\') { p.trim_end_matches('\\').to_string() } else { p }
}

/// Path -> URL path, mirroring the directory structure so relative links resolve:
///
///   C:\docs\a b.md                -> /C:/docs/a%20b.md
///   \\wsl$\Debian\code\README.md  -> /UNC/wsl%24/Debian/code/README.md
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

/// URL path (without query or fragment) -> path. None for "/" and for anything unsafe.
pub fn url_path_to_path(url_path: &str) -> Option<String> {
    let segs: Vec<String> = url_path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    if segs.is_empty() || segs.iter().any(|s| s == ".." || s.contains('\\')) {
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

/// WebView2 can't navigate to custom schemes directly; wry maps `scheme://localhost/` to this.
pub fn scheme_origin(scheme: &str) -> String {
    format!("http://{scheme}.localhost")
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("Condor")
}

pub fn local_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Condor")).unwrap_or_else(config_dir)
}

/// Dot files, plus anything with the Windows "hidden" attribute.
pub fn is_hidden(entry: &std::fs::DirEntry, name: &str) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    // Cheap: the attributes come with the directory listing.
    name.starts_with('.') || entry.metadata().map_or(false, |m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

pub fn launch(url: &str) {
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
}

pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

pub fn tune_ui_webview(b: wry::WebViewBuilder<'_>) -> wry::WebViewBuilder<'_> {
    use wry::WebViewBuilderExtWindows;
    // Stop Ctrl+F, F5 and similar from acting on the tree page itself.
    b.with_browser_accelerator_keys(false)
}

/// Step a webview's history (the same history Alt+Left and the mouse's back button use).
pub fn history_go(wv: &wry::WebView, back: bool) {
    use wry::WebViewExtWindows;
    let w = wv.webview();
    // SAFETY: plain COM calls on a live ICoreWebView2, made on the UI thread.
    unsafe {
        let _ = if back { w.GoBack() } else { w.GoForward() };
    }
}

/// Whether there is somewhere to go (back, forward).
pub fn history_state(wv: &wry::WebView) -> Option<(bool, bool)> {
    use wry::WebViewExtWindows;
    let w = wv.webview();
    let (mut b, mut f) = (0i32, 0i32);
    // SAFETY: as above; BOOL is a transparent i32.
    unsafe {
        w.CanGoBack(&mut b as *mut i32 as *mut _).ok()?;
        w.CanGoForward(&mut f as *mut i32 as *mut _).ok()?;
    }
    Some((b != 0, f != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        for p in ["C:\\docs\\a b.md", "\\\\wsl$\\Debian\\code\\README.md", "\\\\10.0.10.124\\Public\\x#y%z.md", "D:\\"] {
            assert_eq!(url_path_to_path(&path_to_url_path(p)).as_deref(), Some(p));
        }
        assert_eq!(path_to_url_path("\\\\wsl$\\Debian\\a"), "/UNC/wsl%24/Debian/a");
        assert!(crate::platform::is_under("C:\\Docs\\x", "c:\\docs"));
        assert!(!crate::platform::is_under("C:\\docsx", "C:\\docs"));
    }
}
