//! Optional hiding of git-ignored entries (off by default: Condor shows everything).
//!
//! For a listed folder, finds the enclosing repository (nearest ancestor with a `.git`),
//! then applies `.git/info/exclude` and every `.gitignore` from the repository root down to
//! the folder, deepest first, as git does. Matchers are cached briefly so that expanding
//! many folders over a slow share doesn't re-read the same files.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10);

type Cache = Mutex<HashMap<PathBuf, (Instant, Arc<Option<Gitignore>>)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn clear_cache() {
    cache().lock().unwrap().clear();
}

/// Matcher for one directory's own ignore file(s); `is_root` adds `.git/info/exclude`.
fn matcher_for(dir: &Path, is_root: bool) -> Arc<Option<Gitignore>> {
    if let Some((t, m)) = cache().lock().unwrap().get(dir) {
        if t.elapsed() < TTL {
            return m.clone();
        }
    }
    let mut b = GitignoreBuilder::new(dir);
    let mut any = false;
    let gi = dir.join(".gitignore");
    if gi.is_file() {
        any |= b.add(&gi).is_none();
    }
    if is_root {
        let ex = dir.join(".git").join("info").join("exclude");
        if ex.is_file() {
            any |= b.add(&ex).is_none();
        }
    }
    let m = Arc::new(if any { b.build().ok() } else { None });
    cache().lock().unwrap().insert(dir.to_path_buf(), (Instant::now(), m.clone()));
    m
}

/// A filter for the entries of `dir`.
pub struct Filter {
    /// Matchers from `dir` up to the repository root (deepest first).
    levels: Vec<Arc<Option<Gitignore>>>,
}

impl Filter {
    /// None if `dir` isn't inside a git repository.
    pub fn new(dir: &Path) -> Option<Filter> {
        let mut chain = Vec::new();
        let mut cur = Some(dir);
        let mut root = None;
        while let Some(d) = cur {
            chain.push(d);
            if d.join(".git").exists() {
                root = Some(d);
                break;
            }
            cur = d.parent();
        }
        let root = root?;
        let levels = chain.iter().map(|d| matcher_for(d, *d == root)).collect();
        Some(Filter { levels })
    }

    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        if is_dir && path.file_name().map_or(false, |n| n == ".git") {
            return true;
        }
        for m in self.levels.iter().filter_map(|m| m.as_ref().as_ref()) {
            match m.matched(path, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }
}
