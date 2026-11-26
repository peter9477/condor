//! Watches the open file for changes by polling its size and modification time.
//!
//! Polling (rather than OS change notifications) works the same on local disks, WSL
//! (\\wsl$) and network shares, where notifications are unreliable. Files are never held
//! open: they are read in one go when served, so other programs can edit or delete them.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const INTERVAL: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sig {
    Present(Option<SystemTime>, u64),
    Missing,
}

pub fn signature(path: &str) -> Sig {
    match std::fs::metadata(path) {
        Ok(m) => Sig::Present(m.modified().ok(), m.len()),
        Err(_) => Sig::Missing,
    }
}

#[derive(Default)]
struct Target {
    path: Option<String>,
    /// Signature when last loaded; None means "take it on the next poll".
    base: Option<Sig>,
}

#[derive(Clone)]
pub struct Watcher {
    target: Arc<Mutex<Target>>,
}

impl Watcher {
    /// Start polling; `on_change(path, deleted)` is called from the watcher thread.
    pub fn start(on_change: impl Fn(String, bool) + Send + 'static) -> Watcher {
        let target = Arc::new(Mutex::new(Target::default()));
        let t = target.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(INTERVAL);
            if Arc::strong_count(&t) == 1 {
                break; // every Watcher handle is gone (its pane was closed)
            }
            let (path, base) = {
                let g = t.lock().unwrap();
                match &g.path {
                    Some(p) => (p.clone(), g.base),
                    None => continue,
                }
            };
            let sig = signature(&path); // may be slow on a share; done without the lock
            let mut g = t.lock().unwrap();
            if g.path.as_deref() != Some(path.as_str()) || g.base != base {
                continue; // target changed meanwhile
            }
            match base {
                None => g.base = Some(sig),
                Some(b) if b != sig => {
                    g.base = Some(sig);
                    drop(g);
                    on_change(path, sig == Sig::Missing);
                }
                _ => {}
            }
        });
        Watcher { target }
    }

    /// Watch `path` (or nothing), taking its current state as the loaded version.
    pub fn set(&self, path: Option<String>) {
        let mut g = self.target.lock().unwrap();
        if g.path != path {
            g.path = path;
            g.base = None;
        }
    }

    /// The content server is about to serve `path` with this signature: if it is the watched
    /// file, that is the version on screen.
    pub fn served(&self, path: &str, sig: Sig) {
        let mut g = self.target.lock().unwrap();
        if g.path.as_deref() == Some(path) {
            g.base = Some(sig);
        }
    }

    /// The file was (re)loaded: its current state is now the baseline.
    pub fn reset(&self) {
        self.target.lock().unwrap().base = None;
    }
}
