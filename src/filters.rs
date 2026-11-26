//! Which tree entries to show: hidden (dot) files, git-ignored entries, file types, and
//! user-defined exclude patterns. Directories are never hidden by the file-type filter.

use crate::gitfilter;
use crate::state::EnvState;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_VIEWABLE: &[&str] = &[
    "md", "markdown", "mdown", "mkd", "txt", "text", "log", "html", "htm", "xhtml", "pdf", "png", "jpg",
    "jpeg", "gif", "svg", "webp", "bmp", "ico", "avif", "csv", "tsv", "json", "yaml", "yml", "toml",
    "xml", "rst", "adoc", "org",
];

pub struct Filters {
    show_hidden: bool,
    hide_ignored: bool,
    viewable_only: bool,
    exts: HashSet<String>,
    /// Exclude patterns compiled per root (gitignore syntax, relative to each root).
    excludes: Vec<(String, Gitignore)>,
}

impl Filters {
    pub fn from_state(st: &EnvState) -> Filters {
        let mut excludes = Vec::new();
        let pats: Vec<&str> = st
            .excludes
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
            .collect();
        if !pats.is_empty() {
            for root in &st.roots {
                let mut b = GitignoreBuilder::new(root);
                for p in &pats {
                    let _ = b.add_line(None, p);
                }
                if let Ok(g) = b.build() {
                    excludes.push((root.to_ascii_lowercase(), g));
                }
            }
        }
        Filters {
            show_hidden: st.show_hidden,
            hide_ignored: st.hide_ignored,
            viewable_only: st.viewable_only,
            exts: st.viewable_exts.iter().map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).collect(),
            excludes,
        }
    }

    fn exclude_for(&self, dir: &str) -> Option<&Gitignore> {
        let d = dir.to_ascii_lowercase();
        self.excludes
            .iter()
            .filter(|(r, _)| d == *r || d.starts_with(&format!("{}\\", r.trim_end_matches('\\'))))
            .max_by_key(|(r, _)| r.len())
            .map(|(_, g)| g)
    }

    /// A per-directory predicate for the entries of `dir` (call on a worker thread: it may
    /// read .gitignore files).
    pub fn for_dir<'a>(&'a self, dir: &str) -> DirFilter<'a> {
        DirFilter {
            f: self,
            git: if self.hide_ignored { gitfilter::Filter::new(Path::new(dir)) } else { None },
            exclude: self.exclude_for(dir),
        }
    }
}

pub struct DirFilter<'a> {
    f: &'a Filters,
    git: Option<gitfilter::Filter>,
    exclude: Option<&'a Gitignore>,
}

impl DirFilter<'_> {
    pub fn keep(&self, path: &Path, name: &str, is_dir: bool, hidden_attr: bool) -> bool {
        if !self.f.show_hidden && (name.starts_with('.') || hidden_attr) {
            return false;
        }
        if self.f.viewable_only && !is_dir {
            // Files without an extension (README, LICENSE, ...) are usually text: keep them.
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            if ext.map_or(false, |e| !self.f.exts.contains(&e)) {
                return false;
            }
        }
        if let Some(g) = &self.git {
            if g.is_ignored(path, is_dir) {
                return false;
            }
        }
        if let Some(x) = self.exclude {
            if x.matched(path, is_dir).is_ignore() {
                return false;
            }
        }
        true
    }
}
