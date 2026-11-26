//! Per-environment saved state: one JSON file per environment under
//! `<config dir>/environments/<name>.json` (on Windows, %APPDATA%\Condor\environments).

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
    /// Text selection per recently viewed file, as recorded by ui/content.js.
    pub selection: HashMap<String, serde_json::Value>,
    pub sidebar_width: f64,
    pub window: Option<WinGeom>,
    /// Hide files and folders ignored by git (.gitignore).
    pub hide_ignored: bool,
    /// Show dot files and files with the Windows hidden attribute.
    pub show_hidden: bool,
    /// Show only files with a viewable extension (folders are always shown).
    pub viewable_only: bool,
    /// Exclude patterns (gitignore syntax, relative to each root).
    pub excludes: Vec<String>,
    /// Files that reload automatically when they change on disk.
    pub auto_reload: BTreeSet<String>,
    /// Files last viewed as source rather than rendered (opening them uses that view).
    pub source_view: BTreeSet<String>,
    /// The main window is split into two panes; the second shows `split_file`.
    pub split: bool,
    pub split_file: Option<String>,
    /// Side by side (true) or stacked; the first pane's share of the space.
    pub split_side_by_side: bool,
    pub split_ratio: f64,
    /// Files popped out into their own windows (reopened on start).
    pub popouts: Vec<Popout>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Popout {
    pub file: Option<String>,
    pub window: Option<WinGeom>,
}

impl Default for EnvState {
    fn default() -> Self {
        EnvState {
            roots: Vec::new(),
            expanded: BTreeSet::new(),
            open_file: None,
            scroll: HashMap::new(),
            scroll_order: Vec::new(),
            selection: HashMap::new(),
            sidebar_width: 300.0,
            window: None,
            hide_ignored: false,
            auto_reload: BTreeSet::new(),
            show_hidden: false,
            viewable_only: false,
            excludes: Vec::new(),
            source_view: BTreeSet::new(),
            split: false,
            split_file: None,
            split_side_by_side: true,
            split_ratio: 0.5,
            popouts: Vec::new(),
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
            self.selection.remove(&old);
        }
    }
}

pub use crate::platform::{config_dir, local_data_dir};

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

/// Settings shared by every Condor window and environment (`settings.json` beside the
/// environments folder). Windows notice changes within a couple of seconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Extensions shown by the "only viewable file types" filter (without the dot).
    pub viewable_exts: Vec<String>,
    /// How documents use the pane's width: "narrow", "wide" (tables and code) or "full".
    pub doc_width: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            viewable_exts: crate::filters::DEFAULT_VIEWABLE.iter().map(|s| s.to_string()).collect(),
            doc_width: "wide".to_string(),
        }
    }
}

impl Settings {
    /// Tidy user input: extensions lower-case without dots, no blanks or repeats.
    pub fn normalised(mut self) -> Settings {
        let mut seen = std::collections::HashSet::new();
        self.viewable_exts = self
            .viewable_exts
            .iter()
            .flat_map(|s| s.split([',', ' ', '\n', ';']))
            .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty() && seen.insert(e.clone()))
            .collect();
        if !matches!(self.doc_width.as_str(), "narrow" | "wide" | "full") {
            self.doc_width = "wide".into();
        }
        self
    }
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .unwrap_or_default()
        .normalised()
}

pub fn save_settings(s: &Settings) -> std::io::Result<()> {
    let path = settings_file();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}
