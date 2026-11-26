//! Everything that differs between operating systems lives here, so the rest of Condor
//! stays portable. Each platform file provides the same set of items:
//!
//! - `SEP`, `CASE_INSENSITIVE`: path conventions.
//! - `normalise_path`: tidy a user-supplied folder path (absolute, no trailing separator).
//! - `path_to_url_path` / `url_path_to_path`: map paths to and from content-URL paths.
//! - `scheme_origin`: the origin a webview uses for one of our custom protocols.
//! - `config_dir` / `local_data_dir` / `WEBVIEW_DATA`: where settings and caches go.
//! - `is_hidden`: whether a directory entry counts as hidden.
//! - `launch`: hand a URL to the system's default handler.
//! - `open_path` / `open_with` / `CAN_OPEN_WITH` / `is_executable`: open files in other apps.
//! - `pick_folder`: a native "choose folder" dialog.
//! - `confirm`: a native OK / Cancel question.
//! - `tune_ui_webview`: platform-specific webview builder settings.
//! - `history_go` / `history_state`: back and forward in a webview's history.
//! - `on_focus`: be told when a webview gets keyboard focus.
//!
//! Add new OS-specific behaviour as a function here rather than with `cfg` at the call site.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::*;

/// Last component of a path string.
pub fn file_name(path: &str) -> &str {
    let t = path.trim_end_matches(SEP);
    t.rsplit(SEP).next().filter(|s| !s.is_empty()).unwrap_or(path)
}

/// A key for comparing paths (case-folded where the file system ignores case).
pub fn path_key(path: &str) -> String {
    if CASE_INSENSITIVE { path.to_lowercase() } else { path.to_string() }
}

/// True if `path` is `root` or lies inside it.
pub fn is_under(path: &str, root: &str) -> bool {
    let (p, r) = (path_key(path), path_key(root));
    let r = r.trim_end_matches(SEP);
    p == r || p.strip_prefix(r).map_or(false, |rest| rest.starts_with(SEP))
}

/// Open an external link in the default browser or mail client. Only http, https and
/// mailto are passed on: documents must not be able to launch arbitrary handlers.
pub fn open_external(url: &str) {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        launch(url);
    }
}
