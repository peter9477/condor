//! Per-environment saved state (where it lives: see envs.rs), and the global settings.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WinGeom {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub maximized: bool,
}

/// A place in a document (ui/content.js's `locate()`), with what the reader said about it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Bookmark {
    pub id: u64,
    pub path: String,
    /// The view it was made in: "" (the file's usual view), "web" (built-in Word view),
    /// "pdf" (an Office file's PDF), "source".
    pub view: String,
    /// Where: scroll position, selection, page... as the view reported them.
    pub at: serde_json::Value,
    /// The text selected when it was made, or text near the place.
    pub text: String,
    pub desc: String,
    pub tags: Vec<String>,
    /// How far through the document (0..1), for ordering; and a short label ("p. 3", "42%").
    pub frac: f64,
    pub label: String,
    /// Unix seconds.
    pub created: u64,
    pub accessed: u64,
    /// The file when the bookmark was made: size and modification time, and a hash of its
    /// contents (FNV-1a 64, hex), to tell whether it has changed since.
    pub sig: String,
    pub hash: String,
}

/// A file's size and modification time, as a short string (empty if it can't be read).
pub fn file_sig(path: &str) -> String {
    match crate::watch::signature(path) {
        crate::watch::Sig::Present(t, len) => {
            let ns = t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
            format!("{len}-{ns}")
        }
        _ => String::new(),
    }
}

/// A hash of a file's contents (FNV-1a 64, hex; stable across builds), or None if unreadable.
pub fn file_hash(path: &str) -> Option<String> {
    use std::io::Read;
    let local = crate::vfs::local(path).ok()?;
    let mut f = std::fs::File::open(local).ok()?;
    let mut h: u64 = 0xcbf29ce484222325;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    Some(format!("{h:016x}"))
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
    /// Office files kept to one viewer ("web", "previewer", "libreoffice") whatever their
    /// type's choice in the settings.
    pub office_by_file: BTreeMap<String, String>,
    /// The main window is split into two panes; the second shows `split_file`.
    pub split: bool,
    pub split_file: Option<String>,
    /// Side by side (true) or stacked; the first pane's share of the space.
    pub split_side_by_side: bool,
    pub split_ratio: f64,
    /// Files popped out into their own windows (reopened on start).
    pub popouts: Vec<Popout>,
    /// Tags on files and folders, by path. A star is the tag `STAR`.
    pub tags: BTreeMap<String, Tagged>,
    /// Tags the tree is filtered to (shows items with any of them); empty shows everything.
    pub tag_filter: Vec<String>,
    /// Bookmarks: places in documents, with a description and tags of their own.
    pub bookmarks: Vec<Bookmark>,
}

/// The tag that shows as a star.
pub const STAR: &str = "\u{2605}";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Tagged {
    pub dir: bool,
    pub tags: BTreeSet<String>,
}

/// A tag as typed: trimmed, inner whitespace collapsed, at most 40 characters.
pub fn clean_tag(t: &str) -> Option<String> {
    let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
    (!t.is_empty()).then_some(t)
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
            office_by_file: BTreeMap::new(),
            show_hidden: false,
            viewable_only: false,
            excludes: Vec::new(),
            source_view: BTreeSet::new(),
            split: false,
            split_file: None,
            split_side_by_side: true,
            split_ratio: 0.5,
            popouts: Vec::new(),
            tags: BTreeMap::new(),
            tag_filter: Vec::new(),
            bookmarks: Vec::new(),
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


/// Settings shared by every Condor window and environment (`settings.json` beside the
/// environments folder). Windows notice changes within a couple of seconds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Extensions shown by the "only viewable file types" filter (without the dot).
    pub viewable_exts: Vec<String>,
    /// How documents use the pane's width: "narrow", "wide" (tables and code) or "full".
    pub doc_width: String,
    /// Office and similar files: "auto" (the Windows previewer if one is installed, else
    /// LibreOffice), "previewer", "libreoffice" or "off".
    pub office_view: String,
    /// File types (extension) shown the other way: "previewer" or "libreoffice".
    pub office_by_type: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            viewable_exts: crate::filters::DEFAULT_VIEWABLE.iter().map(|s| s.to_string()).collect(),
            doc_width: "wide".to_string(),
            office_view: "auto".to_string(),
            office_by_type: Default::default(),
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
        if !matches!(self.office_view.as_str(), "auto" | "previewer" | "libreoffice" | "off") {
            self.office_view = "auto".into();
        }
        self.office_by_type = std::mem::take(&mut self.office_by_type)
            .into_iter()
            .map(|(k, v)| (k.trim().trim_start_matches('.').to_ascii_lowercase(), v))
            .filter(|(k, v)| !k.is_empty() && matches!(v.as_str(), "previewer" | "libreoffice"))
            .collect();
        self
    }
}

/// `settings.json` beside the environments, or the file named by CONDOR_SETTINGS (so a test
/// instance can't change the settings of the windows someone is using).
pub fn settings_file() -> PathBuf {
    match std::env::var_os("CONDOR_SETTINGS") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => config_dir().join("settings.json"),
    }
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
