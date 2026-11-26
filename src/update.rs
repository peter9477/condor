//! Notices when a new build replaces our exe on disk (e.g. via tools/deploy.sh, which
//! renames the running exe aside and copies the new one into its place), so the UI can
//! offer to restart into it.

use crate::watch::{signature, Sig};
use std::path::PathBuf;
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(5);

/// Path we were started from (taken at startup, before any rename).
pub fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Calls `on_new` once, when the file at `exe` differs from how it was at startup and
/// has been stable for one interval (so a half-copied file isn't offered).
pub fn watch(exe: PathBuf, on_new: impl Fn() + Send + 'static) {
    let Some(path) = exe.to_str().map(str::to_string) else { return };
    let start = signature(&path);
    std::thread::spawn(move || {
        let mut prev = start;
        loop {
            std::thread::sleep(INTERVAL);
            let sig = signature(&path);
            if sig != start && sig == prev && matches!(sig, Sig::Present(..)) {
                on_new();
                return;
            }
            prev = sig;
        }
    });
}
