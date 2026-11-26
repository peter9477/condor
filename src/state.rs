//! Per-environment saved state: one JSON file per environment under
//! %APPDATA%\Condor\environments\<name>.json.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WinGeom {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct EnvState {
    /// Root folders, in display order.
    pub roots: Vec<String>,
    /// Every folder the user has expanded, independent of whether its ancestors are
    /// currently expanded, so collapsing and re-expanding a parent restores the nesting.
    pub expanded: BTreeSet<String>,
    /// File shown in the viewing pane.
    pub open_file: Option<String>,
    /// Scroll position (CSS px) per recently viewed file.
    pub scroll: HashMap<String, f64>,
    /// Order of `scroll` keys, most recent last (to cap its size).
    pub scroll_order: Vec<String>,
    pub sidebar_width: f64,
    pub window: Option<WinGeom>,
    /// Hide files and folders ignored by git (.gitignore).
    pub hide_ignored: bool,
    /// Show dot files and files with the Windows hidden attribute.
    pub show_hidden: bool,
    /// Show only files with a viewable extension (folders are always shown).
    pub viewable_only: bool,
    pub viewable_exts: Vec<String>,
    /// Exclude patterns (gitignore syntax, relative to each root).
    pub excludes: Vec<String>,
    /// Files that reload automatically when they change on disk.
    pub auto_reload: BTreeSet<String>,
}

impl Default for EnvState {
    fn default() -> Self {
        EnvState {
            roots: Vec::new(),
            expanded: BTreeSet::new(),
            open_file: None,
            scroll: HashMap::new(),
            scroll_order: Vec::new(),
            sidebar_width: 300.0,
            window: None,
            hide_ignored: false,
            auto_reload: BTreeSet::new(),
            show_hidden: false,
            viewable_only: false,
            viewable_exts: crate::filters::DEFAULT_VIEWABLE.iter().map(|s| s.to_string()).collect(),
            excludes: Vec::new(),
        }
    }
}

const SCROLL_MEMORY: usize = 200;

impl EnvState {
    pub fn set_scroll(&mut self, file: &str, y: f64) {
        self.scroll_order.retain(|f| f != file);
        self.scroll_order.push(file.to_string());
        self.scroll.insert(file.to_string(), y);
        while self.scroll_order.len() > SCROLL_MEMORY {
            let old = self.scroll_order.remove(0);
            self.scroll.remove(&old);
        }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Condor")
}

pub fn local_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("Condor"))
        .unwrap_or_else(|| config_dir())
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_ .".contains(c) { c } else { '_' })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "default".into() } else { s }
}

pub fn env_file(name: &str) -> PathBuf {
    config_dir().join("environments").join(format!("{}.json", sanitize(name)))
}

pub fn load(name: &str) -> EnvState {
    std::fs::read(env_file(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(name: &str, st: &EnvState) -> std::io::Result<()> {
    let path = env_file(name);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(st).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}
