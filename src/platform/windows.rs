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

/// Let process `pid` bring its window to the front (we were just started by the user, so
/// we may; it may not on its own).
pub fn allow_foreground(pid: u32) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
}

/// The icon linked into the exe (assets/condor.res), for title bars and the taskbar.
pub fn app_icon() -> Option<tao::window::Icon> {
    use tao::platform::windows::IconExtWindows;
    tao::window::Icon::from_resource(1, None).ok()
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

/// Put text on the clipboard.
pub fn clipboard_text(text: &str) -> bool {
    let w: Vec<u8> = text.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect();
    set_clipboard(&[(13 /* CF_UNICODETEXT */, w)])
}

/// Put rich text (an HTML fragment) and its plain-text form on the clipboard, as a
/// browser's Copy does: chat apps that take formatting (Slack) use the HTML.
pub fn clipboard_html(html: &str, text: &str) -> bool {
    // CF_HTML: a header of byte offsets, then the document with the fragment marked.
    const HEAD: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let pre = "<html><body><!--StartFragment-->";
    let post = "<!--EndFragment--></body></html>";
    let start_html = HEAD.len();
    let start_frag = start_html + pre.len();
    let end_frag = start_frag + html.len();
    let end_html = end_frag + post.len();
    let head = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_frag:010}\r\nEndFragment:{end_frag:010}\r\n"
    );
    let mut cf_html = format!("{head}{pre}{html}{post}").into_bytes();
    cf_html.push(0);
    let fmt = unsafe { windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!("HTML Format")) };
    let w: Vec<u8> = text.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect();
    set_clipboard(&[(13, w), (fmt, cf_html)])
}

/// Put files on the clipboard, as Explorer's Copy does: pasting attaches or copies them.
pub fn clipboard_files(paths: &[String]) -> bool {
    // DROPFILES { pFiles, pt.x, pt.y, fNC, fWide }, then NUL-terminated wide paths and a final NUL.
    let mut b: Vec<u8> = [20u32, 0, 0, 0, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    for p in paths {
        b.extend(p.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()));
    }
    b.extend([0, 0]);
    let effect = unsafe {
        windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!("Preferred DropEffect"))
    };
    set_clipboard(&[(15 /* CF_HDROP */, b), (effect, 1u32.to_le_bytes().to_vec() /* DROPEFFECT_COPY */)])
}

fn set_clipboard(items: &[(u32, Vec<u8>)]) -> bool {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    unsafe {
        // Another program may have the clipboard open for a moment.
        let mut open = false;
        for _ in 0..20 {
            if OpenClipboard(None).is_ok() {
                open = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !open {
            return false;
        }
        let mut ok = EmptyClipboard().is_ok();
        for (format, bytes) in items {
            let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { ok = false; continue };
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                ok = false;
                continue;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            let _ = GlobalUnlock(h);
            // On success the clipboard owns the memory.
            if SetClipboardData(*format, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(Some(HGLOBAL(h.0)));
                ok = false;
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// `\\wsl$\Distro\some\path` (or `\\wsl.localhost\...`): the distro and the Linux path.
fn wsl_parts(path: &str) -> Option<(String, String)> {
    let lower = path.to_ascii_lowercase();
    let rest = ["\\\\wsl$\\", "\\\\wsl.localhost\\"].iter().find_map(|p| lower.starts_with(p).then(|| &path[p.len()..]))?;
    let (distro, linux) = rest.split_once('\\').unwrap_or((rest, ""));
    Some((distro.to_string(), format!("/{}", linux.replace('\\', "/").trim_end_matches('/'))))
}

/// Open a terminal in `dir`: Windows Terminal if installed, else PowerShell. A WSL folder
/// gets a WSL shell in its Linux path.
pub fn open_terminal(dir: &str) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    if let Some((distro, linux)) = wsl_parts(dir) {
        let linux = if linux.is_empty() { "/".to_string() } else { linux };
        // A fresh login shell of its own: BYOBU_DISABLE stops a byobu set up to start from
        // ~/.profile from joining (and resizing) an existing session.
        let shell = ["--", "env", "BYOBU_DISABLE=1", "bash", "-l"];
        let wsl = |c: &mut Command| {
            c.args(["-d", &distro, "--cd", &linux]).args(shell);
        };
        let mut wt = Command::new("wt.exe");
        wt.arg("wsl.exe");
        wsl(&mut wt);
        if wt.spawn().is_err() {
            let mut c = Command::new("wsl.exe");
            wsl(&mut c);
            let _ = c.creation_flags(CREATE_NEW_CONSOLE).spawn();
        }
        return;
    }
    // A trailing backslash would escape the closing quote; ';' separates wt commands.
    let d = if dir.ends_with('\\') { format!("{dir}.") } else { dir.to_string() };
    if Command::new("wt.exe").arg("-d").arg(d.replace(';', r"\;")).spawn().is_err() {
        let _ = Command::new("powershell.exe")
            .args(["-NoExit", "-Command", &format!("Set-Location -LiteralPath '{}'", d.replace('\'', "''"))])
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn();
    }
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

/// A shell preview handler (what Explorer's preview pane uses; e.g. Office's previewers)
/// showing a file in a child window of ours.
pub struct Preview {
    host: windows::Win32::Foundation::HWND,
    handler: windows::Win32::UI::Shell::IPreviewHandler,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Whether a preview handler is registered for this file's type.
pub fn has_previewer(path: &str) -> bool {
    previewer_clsid(path).is_some()
}

fn previewer_clsid(path: &str) -> Option<String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION};
    let ext = std::path::Path::new(path).extension()?.to_string_lossy().into_owned();
    let ext = wide(&format!(".{ext}"));
    let iid = wide("{8895b1c6-b41f-4c1c-a562-0d564250836f}"); // IPreviewHandler
    let mut buf = [0u16; 64];
    let mut len = buf.len() as u32;
    let hr = unsafe {
        AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, PCWSTR(ext.as_ptr()), PCWSTR(iid.as_ptr()), Some(PWSTR(buf.as_mut_ptr())), &mut len)
    };
    if hr.is_err() {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]);
    let s = s.trim_end_matches('\0').to_string();
    (!s.is_empty()).then_some(s)
}

impl Preview {
    /// Show `path` with its registered preview handler, in a child of `parent` at the given
    /// physical-pixel rectangle. None if there is no handler or it fails.
    pub fn open(parent: isize, path: &str, rect: (i32, i32, i32, i32)) -> Result<Preview, String> {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::System::Com::{CLSIDFromString, CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, STGM_READ, STGM_SHARE_DENY_NONE};
        use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
        use windows::Win32::UI::Shell::{IPreviewHandler, SHCreateStreamOnFileEx};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_VISIBLE};
        let clsid = previewer_clsid(path).ok_or("no previewer registered")?;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let clsid_w = wide(&clsid);
            let guid = CLSIDFromString(PCWSTR(clsid_w.as_ptr())).map_err(|e| e.to_string())?;
            let handler: IPreviewHandler =
                CoCreateInstance(&guid, None, CLSCTX_LOCAL_SERVER | CLSCTX_INPROC_SERVER).map_err(|e| format!("starting it: {e}"))?;
            let pw = wide(path);
            let init = if let Ok(f) = handler.cast::<IInitializeWithFile>() {
                f.Initialize(PCWSTR(pw.as_ptr()), STGM_READ.0)
            } else if let Ok(s) = handler.cast::<IInitializeWithStream>() {
                let stream = SHCreateStreamOnFileEx(PCWSTR(pw.as_ptr()), (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)
                    .map_err(|e| e.to_string())?;
                s.Initialize(&stream, STGM_READ.0)
            } else {
                return Err("it can't open files".into());
            };
            init.map_err(|e| format!("opening the file: {e}"))?;
            let class = wide("STATIC");
            let host = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
                rect.0,
                rect.1,
                rect.2,
                rect.3,
                Some(HWND(parent as *mut _)),
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;
            let r = RECT { left: 0, top: 0, right: rect.2, bottom: rect.3 };
            let shown = handler.SetWindow(host, &r).and_then(|_| handler.DoPreview());
            if let Err(e) = shown {
                let _ = handler.Unload();
                let _ = DestroyWindow(host);
                return Err(format!("previewing: {e}"));
            }
            let p = Preview { host, handler };
            p.set_rect(rect);
            Ok(p)
        }
    }

    /// Move it (physical pixels, relative to the parent window).
    pub fn set_rect(&self, rect: (i32, i32, i32, i32)) {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW};
        unsafe {
            let _ = SetWindowPos(self.host, Some(HWND_TOP), rect.0, rect.1, rect.2.max(1), rect.3.max(1), SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let r = RECT { left: 0, top: 0, right: rect.2.max(1), bottom: rect.3.max(1) };
            let _ = self.handler.SetRect(&r);
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        unsafe {
            let _ = self.handler.Unload();
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.host);
        }
    }
}

/// The native handle of a window, for `Preview::open`.
pub fn window_handle(w: &tao::window::Window) -> isize {
    use tao::platform::windows::WindowExtWindows;
    w.hwnd() as isize
}
