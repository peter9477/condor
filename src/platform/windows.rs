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

/// Open a file with its default application, as a double-click in Explorer would.
pub fn open_path(path: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// Windows has an "Open with" chooser.
pub const CAN_OPEN_WITH: bool = true;

/// Show the system's "Open with" chooser for a file.
pub fn open_with(path: &str) {
    use std::os::windows::process::CommandExt;
    // OpenAs_RunDLL takes the rest of the command line, unquoted, as the path. Windows
    // paths cannot contain '"', and nothing after the path is parsed.
    let _ = std::process::Command::new("rundll32.exe").raw_arg(format!("shell32.dll,OpenAs_RunDLL {path}")).spawn();
}

/// Whether opening the file would run it (programs, scripts, shortcuts, installers).
pub fn is_executable(path: &str) -> bool {
    const ALSO: &[&str] = &["lnk", "url", "hta", "msi", "msp", "scr", "ps1", "psm1", "reg", "cpl", "jar", "pif",
        "appref-ms", "application", "gadget", "inf", "scf", "settingcontent-ms", "msix", "appx"];
    let name = crate::platform::file_name(path);
    let Some((_, ext)) = name.rsplit_once('.') else { return false };
    let ext = ext.to_ascii_lowercase();
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".into());
    ALSO.contains(&ext.as_str()) || pathext.split(';').any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
}

/// A native OK / Cancel question; true for OK.
pub fn confirm(title: &str, text: &str) -> bool {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show()
        == rfd::MessageDialogResult::Ok
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

/// Call `f` whenever the webview gets keyboard focus (e.g. the user clicks in it).
pub fn on_focus(wv: &wry::WebView, f: impl Fn() + 'static) {
    use webview2_com::FocusChangedEventHandler;
    use wry::WebViewExtWindows;
    let c = wv.controller();
    let mut token = 0i64;
    // SAFETY: registering an event handler on a live controller, on the UI thread.
    unsafe {
        let _ = c.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new(move |_, _| {
                f();
                Ok(())
            })),
            &mut token,
        );
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
