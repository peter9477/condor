#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Condor: a lightweight folder-tree file viewer.
//!
//! Each window ("shell") has one UI webview underneath everything, and one or more panes on
//! top of it, each a content webview:
//!   * the UI webview is our own trusted page: the folder tree (main window only), the bar
//!     above each pane, banners and splitters. It is the only one with IPC.
//!   * content webviews show documents: untrusted, no IPC at all.
//!
//! The main window has the tree and one or two panes (split side by side or stacked).
//! Pop-out windows have a single pane and no tree.

mod convert;
mod envs;
mod filters;
mod paths;
mod platform;
mod gitfilter;
mod highlight;
mod render;
mod search;
mod state;
mod update;
mod watch;

use serde::Deserialize;
use serde_json::json;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::http::{header, Response};
use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

// Layout sizes, in the UI webview's CSS pixels.
/// Splitter between the tree and the panes.
const SPLITTER: f64 = 5.0;
/// Bar above each pane: file name and the file's controls.
const PANE_BAR: f64 = 32.0;
/// "File changed" / "can't show" banner under a pane's bar.
const BANNER: f64 = 34.0;
/// Line above the document, so pages read as separate from the UI.
const FRAME: f64 = 1.0;
/// Divider between two panes.
const DIVIDER: f64 = 5.0;

const SAVE_DELAY: Duration = Duration::from_millis(800);
const SCROLL_POLL: Duration = Duration::from_millis(1500);
/// How often open folders are checked for added or removed entries.
const DIR_CHECK: Duration = Duration::from_secs(3);

const UI_HTML: &str = include_str!("../ui/index.html");
const PICKER_HTML: &str = include_str!("../ui/picker.html");
/// A document slower than this to arrive shows "loading" in its pane's bar.
const LOADING_SHOWN_AFTER: Duration = Duration::from_millis(600);
/// An environment slower than this to load (a slow share) shows the "waiting" window.
const WAIT_SHOWN_AFTER: Duration = Duration::from_millis(700);

type PaneId = u32;

#[derive(Debug)]
enum UserEvent {
    /// Message from a window's UI webview (or the selector).
    Ipc(WindowId, String),
    /// An environment's state was loaded (or its folder is missing): (request number, result).
    Opened(u64, envs::Opened),
    /// The selector's check of whether an environment's folder is reachable.
    Checked(String, bool),
    /// Another Condor was started for this environment: come to the front.
    Activate,
    /// A short note to show in a window (in a pane's bar, or the tree's window).
    Flash(WindowId, Option<PaneId>, String),
    /// A directory listing finished (JSON message for the main UI).
    Listed(String),
    /// A pane is navigating to this URL.
    ContentNav(PaneId, String),
    /// A pane finished loading this URL.
    PageLoaded(PaneId, String),
    /// Polled view state of a pane: (url, scroll y, selection).
    Scroll(PaneId, String, f64, serde_json::Value),
    /// A document asked to open a local file in a new window ("Open link in new window",
    /// Ctrl+click, target=_blank): open it in a pop-out.
    OpenWindow(String),
    /// The file shown can't be previewed (binary or too large); offer to open it elsewhere.
    NoPreview(PaneId, String),
    /// The file in a pane changed on disk: (path, deleted).
    FileChanged(PaneId, String, bool),
    /// Reload a pane, first recording this scroll position for its file.
    ReloadAt(PaneId, String, f64),
    /// Load this URL in a pane (a file:// link, redirected to Condor's own URL for the file).
    LoadContent(PaneId, String),
    /// A pane's document got keyboard focus.
    Focus(PaneId),
    /// A new build of condor.exe has been installed where we were started from.
    UpdateAvailable,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum Cmd {
    Ready,
    List { path: String },
    /// Open a file from the tree, in the main window's focused pane.
    Open { path: String },
    Expand { path: String, open: bool },
    /// The UI's device pixel ratio, and (main window) the tree's width.
    Sidebar { width: f64, dpr: Option<f64> },
    AddRoot,
    RemoveRoot { path: String },
    MoveRoot { path: String, to: usize },
    Restart,
    SetSettings { settings: state::Settings },
    OpenExternal { path: String, with: bool },
    /// Clipboard: a file or folder itself (as Explorer copies it), or its path as text.
    CopyFile { path: String },
    CopyPath { path: String },
    /// Clipboard: the pane's selection, else the whole document.
    CopyPane { pane: Option<PaneId> },
    OpenTerminal { path: String },
    SetFilters { show_hidden: bool, hide_ignored: bool, viewable_only: bool, excludes: Vec<String> },
    AutoReload { path: String, on: bool },
    // Pane commands: `pane` defaults to the window's focused pane.
    Reload { pane: Option<PaneId> },
    Dismiss { pane: Option<PaneId> },
    Back { pane: Option<PaneId> },
    Forward { pane: Option<PaneId> },
    ToggleSource { pane: Option<PaneId> },
    Focus { pane: PaneId },
    /// Split the main window (or, if split, swap side-by-side and stacked).
    Split { pane: Option<PaneId> },
    ClosePane { pane: PaneId },
    /// Open a file, or a pane's file, in a new window (a pane in a split moves there).
    PopOut { pane: Option<PaneId>, path: Option<String> },
    /// Open a file in the main window's other pane, splitting if needed.
    OpenOther { path: String },
    /// The first pane's share of a split.
    PaneRatio { ratio: f64 },
    /// Make room above a pane's document for a popup (the tag editor); 0 to give it back.
    PaneInset { pane: PaneId, px: f64 },
    /// Search the roots; the reply carries `id` so the UI can drop stale results.
    Search { query: String, id: u64 },
    /// Show the pane's Office file with the Windows previewer or LibreOffice.
    OfficeView { pane: Option<PaneId>, mode: String },
    /// Replace the tags on a file or folder (an empty list removes them).
    SetTags { path: String, dir: bool, tags: Vec<String> },
    /// Filter the tree to items with any of these tags (empty: no filter).
    TagFilter { tags: Vec<String> },
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum Kind {
    Main,
    Popout,
}

/// A top-level window: its UI webview and its panes.
struct Shell {
    kind: Kind,
    // Field order matters: the webview is dropped before the window that hosts it.
    ui: WebView,
    window: Window,
    /// Device pixel ratio of the UI webview, as it reports it.
    ui_dpr: Option<f64>,
    panes: Vec<PaneId>,
    focused: PaneId,
}

/// A document view.
struct Pane {
    shell: WindowId,
    content: WebView,
    watcher: watch::Watcher,
    /// File shown, and the full URL (its query says rendered or source view).
    current: Option<String>,
    url: String,
    /// A saved scroll position is still being applied; don't record positions meanwhile.
    restoring: bool,
    /// The banner showing under the pane's bar, if any (the message sent to the UI).
    banner: Option<serde_json::Value>,
    /// A Windows preview handler showing the file in place of the content view.
    preview: Option<platform::Preview>,
    /// Extra space above the document (CSS px) while the UI shows a popup there.
    inset: f64,
    /// Where the document goes, in physical pixels (x, y, w, h).
    rect: std::cell::Cell<(i32, i32, i32, i32)>,
    /// A document is on its way: when to say so, and whether the bar says so now.
    loading: Option<Instant>,
    loading_shown: bool,
}

struct App {
    env: envs::Env,
    /// Writes the state file off the UI thread (it may be on a slow share).
    saver: envs::Saver,
    st: state::EnvState,
    proxy: EventLoopProxy<UserEvent>,
    ctx: WebContext,
    main: WindowId,
    shells: HashMap<WindowId, Shell>,
    /// Pop-out windows in the order they were opened (saved in that order).
    popout_order: Vec<WindowId>,
    panes: HashMap<PaneId, Pane>,
    next_pane: PaneId,
    save_at: Option<Instant>,
    next_poll: Instant,
    last_saved: Option<state::EnvState>,
    filters: Arc<filters::Filters>,
    /// The exe we were started from, and whether a newer build has replaced it.
    exe: Option<std::path::PathBuf>,
    update_ready: bool,
    /// Restart once the panes have reported their scroll positions.
    restart_at: Option<Instant>,
    /// Global settings, and the settings file's signature when last read or written.
    settings: state::Settings,
    settings_sig: watch::Sig,
    /// Open folders' modification times (they change when entries come or go), checked on a
    /// worker thread; one check at a time.
    dir_times: Arc<std::sync::Mutex<HashMap<String, std::time::SystemTime>>>,
    dir_check: Arc<std::sync::atomic::AtomicBool>,
    next_dir_check: Instant,
}

fn log(msg: &str) {
    use std::io::Write;
    let dir = state::local_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("condor.log")) {
        let _ = writeln!(f, "{msg}");
    }
}

/// Extra tracing, when the CONDOR_DEBUG environment variable is set.
fn debug(msg: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("CONDOR_DEBUG").is_some()) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() % 100_000);
        log(&format!("{t:>5} {}", msg()));
    }
}

/// Read a directory on a worker thread and report back.
fn list_dir(path: String, filters: Arc<filters::Filters>, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let msg = match std::fs::read_dir(&path) {
            Ok(rd) => {
                let keep = filters.for_dir(&path);
                let mut entries: Vec<(bool, String, String)> = rd
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let ft = e.file_type().ok();
                        let mut is_dir = ft.map(|t| t.is_dir()).unwrap_or(false);
                        if ft.map(|t| t.is_symlink()).unwrap_or(false) {
                            is_dir = e.path().is_dir();
                        }
                        let name = e.file_name().to_string_lossy().into_owned();
                        let p = e.path();
                        if !keep.keep(&p, &name, is_dir, platform::is_hidden(&e, &name)) {
                            return None;
                        }
                        Some((is_dir, name, p.to_string_lossy().into_owned()))
                    })
                    .collect();
                entries.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
                        .then_with(|| a.1.cmp(&b.1))
                });
                let list: Vec<_> = entries
                    .into_iter()
                    .map(|(d, n, p)| json!({"name": n, "path": p, "dir": d}))
                    .collect();
                json!({"ev": "list", "path": path, "entries": list})
            }
            Err(e) => json!({"ev": "list", "path": path, "error": e.to_string()}),
        };
        let _ = proxy.send_event(UserEvent::Listed(msg.to_string()));
    });
}

/// Build a top-level window, restoring its geometry if it is still on a connected monitor.
fn build_window(target: &EventLoopWindowTarget<UserEvent>, geom: Option<state::WinGeom>, default: LogicalSize<f64>) -> Window {
    let mut wb = WindowBuilder::new().with_title("Condor").with_inner_size(default).with_window_icon(platform::app_icon());
    if let Some(g) = geom {
        let on_screen = target.available_monitors().any(|m| {
            let (mp, ms) = (m.position(), m.size());
            g.x + 50 < mp.x + ms.width as i32 && g.x + g.w as i32 > mp.x + 50 && g.y >= mp.y - 20 && g.y < mp.y + ms.height as i32 - 50
        });
        wb = wb.with_inner_size(PhysicalSize::new(g.w.max(300), g.h.max(200)));
        if on_screen {
            wb = wb.with_position(PhysicalPosition::new(g.x, g.y));
        }
        wb = wb.with_maximized(g.maximized);
    }
    wb.build(target).expect("create window")
}

/// The UI webview for a window: our own page, the only one with IPC.
fn build_ui(ctx: &mut WebContext, window: &Window, proxy: &EventLoopProxy<UserEvent>, kind: Kind) -> WebView {
    let p_ipc = proxy.clone();
    let wid = window.id();
    let ui_origin = format!("{}/", platform::scheme_origin(paths::UI_SCHEME));
    let mode = if kind == Kind::Main { "main" } else { "popout" };
    platform::tune_ui_webview(WebViewBuilder::new_with_web_context(ctx))
        .with_url(format!("{}://localhost/?mode={mode}", paths::UI_SCHEME))
        .with_custom_protocol(paths::UI_SCHEME.into(), |_id, _req| {
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(Cow::Borrowed(UI_HTML.as_bytes()))
                .unwrap()
        })
        .with_navigation_handler(move |url| {
            if url.starts_with(&ui_origin) {
                true
            } else {
                platform::open_external(&url);
                false
            }
        })
        .with_ipc_handler(move |req| {
            let _ = p_ipc.send_event(UserEvent::Ipc(wid, req.into_body()));
        })
        .with_devtools(std::env::var_os("CONDOR_DEVTOOLS").is_some())
        // Built first, so the panes sit on top of it.
        .build_as_child(window)
        .expect("create UI webview")
}

/// A content webview for pane `id`: untrusted documents, no IPC.
fn build_content(ctx: &mut WebContext, window: &Window, proxy: &EventLoopProxy<UserEvent>, id: PaneId, url: &str) -> (WebView, watch::Watcher) {
    let p_watch = proxy.clone();
    let watcher = watch::Watcher::start(move |path, deleted| {
        let _ = p_watch.send_event(UserEvent::FileChanged(id, path, deleted));
    });
    let w_serve = watcher.clone();
    let w_nav = watcher.clone();
    let (p_nav, p_load, p_new, p_focus) = (proxy.clone(), proxy.clone(), proxy.clone(), proxy.clone());
    let p_redirect = proxy.clone();
    let file_origin = paths::file_origin();
    let fo2 = file_origin.clone();
    let start = format!("{}://localhost{}", paths::FILE_SCHEME, url.strip_prefix(&file_origin).unwrap_or("/"));
    let content = WebViewBuilder::new_with_web_context(ctx)
        .with_url(start)
        .with_asynchronous_custom_protocol(paths::FILE_SCHEME.into(), move |_id, req, responder| {
            let uri = req.uri().clone();
            let w = w_serve.clone();
            std::thread::spawn(move || {
                if let Some(p) = paths::url_path_to_path(uri.path()) {
                    w.served(&p, watch::signature(&p));
                }
                responder.respond(render::serve(uri.path(), uri.query()));
            });
        })
        .with_navigation_handler(move |url| {
            if url.starts_with(&file_origin) {
                w_nav.set(paths::url_to_path(&url));
                let _ = p_nav.send_event(UserEvent::ContentNav(id, url));
                true
            } else if url.starts_with("about:") || url.starts_with("data:") || url.starts_with("blob:") {
                true
            } else if let Some(p) = paths::file_url_to_path(&url) {
                // file:// links open here, like any other link to a local file.
                let _ = p_redirect.send_event(UserEvent::LoadContent(id, paths::path_to_url(&p)));
                false
            } else {
                platform::open_external(&url);
                false
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            if url.starts_with(&fo2) {
                let _ = p_new.send_event(UserEvent::OpenWindow(url));
            } else if let Some(p) = paths::file_url_to_path(&url) {
                let _ = p_new.send_event(UserEvent::OpenWindow(paths::path_to_url(&p)));
            } else {
                platform::open_external(&url);
            }
            wry::NewWindowResponse::Deny
        })
        .with_initialization_script(include_str!("../ui/content.js"))
        .with_on_page_load_handler(move |ev, url| {
            if let PageLoadEvent::Finished = ev {
                let _ = p_load.send_event(UserEvent::PageLoaded(id, url));
            }
        })
        .with_devtools(std::env::var_os("CONDOR_DEVTOOLS").is_some())
        .build_as_child(window)
        .expect("create content webview");
    platform::on_focus(&content, move || {
        let _ = p_focus.send_event(UserEvent::Focus(id));
    });
    (content, watcher)
}

/// How an Office file is shown under these settings: the choice made for its type, else
/// the global setting ("auto": the previewer, but LibreOffice for unreliable previewers).
fn office_mode(s: &state::Settings, path: &str) -> &'static str {
    match s.office_by_type.get(&convert::ext_of(path)).map(String::as_str) {
        Some("previewer") => return "previewer",
        Some("libreoffice") => return "libreoffice",
        _ => {}
    }
    match s.office_view.as_str() {
        "previewer" => "previewer",
        "libreoffice" => "libreoffice",
        "off" => "off",
        _ if convert::previewer_unreliable(path) && convert::available() => "libreoffice",
        _ => "previewer",
    }
}

fn geometry_of(w: &Window, previous: Option<state::WinGeom>) -> Option<state::WinGeom> {
    if w.is_minimized() {
        return previous;
    }
    if w.is_maximized() {
        // Keep the restored geometry; just record the flag.
        return previous.map(|g| state::WinGeom { maximized: true, ..g });
    }
    let pos = w.outer_position().ok()?;
    let size = w.inner_size();
    Some(state::WinGeom { x: pos.x, y: pos.y, w: size.width, h: size.height, maximized: false })
}

impl App {
    // ----- plumbing -------------------------------------------------------------------

    fn send_ui(&self, wid: WindowId, msg: &serde_json::Value) {
        if let Some(sh) = self.shells.get(&wid) {
            let _ = sh.ui.evaluate_script(&format!("window.condor && condor.recv({msg})"));
        }
    }

    fn send_main(&self, msg: &serde_json::Value) {
        self.send_ui(self.main, msg);
    }

    fn mark_dirty(&mut self) {
        self.save_at = Some(Instant::now() + SAVE_DELAY);
    }

    fn save_now(&mut self) {
        self.capture_layout();
        if self.last_saved.as_ref() != Some(&self.st) {
            self.saver.save(&self.env.state_file, &self.st);
            self.last_saved = Some(self.st.clone());
        }
        self.save_at = None;
    }

    /// Record window geometry, the split and the pop-outs in the state.
    fn capture_layout(&mut self) {
        let Some(main) = self.shells.get(&self.main) else { return };
        self.st.window = geometry_of(&main.window, self.st.window);
        let files: Vec<Option<String>> = main.panes.iter().map(|p| self.panes.get(p).and_then(|p| p.current.clone())).collect();
        if let Some(Some(f)) = files.first() {
            self.st.open_file = Some(f.clone());
        }
        self.st.split = files.len() > 1;
        if let Some(f) = files.get(1) {
            self.st.split_file = f.clone();
        }
        let old = std::mem::take(&mut self.st.popouts);
        self.st.popouts = self
            .popout_order
            .iter()
            .enumerate()
            .filter_map(|(i, wid)| {
                let sh = self.shells.get(wid)?;
                let file = sh.panes.first().and_then(|p| self.panes.get(p)).and_then(|p| p.current.clone());
                Some(state::Popout { file, window: geometry_of(&sh.window, old.get(i).and_then(|o| o.window)) })
            })
            .collect();
    }

    fn filter_state(&self) -> serde_json::Value {
        json!({
            "showHidden": self.st.show_hidden,
            "hideIgnored": self.st.hide_ignored,
            "viewableOnly": self.st.viewable_only,
            "excludes": self.st.excludes,
        })
    }

    fn rebuild_filters(&mut self) {
        self.filters = Arc::new(filters::Filters::from_state(&self.st, &self.settings));
        render::set_roots(&self.st.roots);
    }

    /// URL for opening `path`, in the view (rendered or source) last chosen for it.
    fn url_for(&self, path: &str) -> String {
        let url = paths::path_to_url(path);
        if self.st.source_view.contains(path) && render::has_source_view(path) {
            format!("{url}?source")
        } else {
            url
        }
    }

    fn pane_of(&self, wid: WindowId, pane: Option<PaneId>) -> Option<PaneId> {
        let id = pane.or_else(|| self.shells.get(&wid).map(|s| s.focused))?;
        self.panes.contains_key(&id).then_some(id)
    }

    // ----- windows and panes ----------------------------------------------------------

    fn add_pane(&mut self, wid: WindowId, file: Option<&str>) -> PaneId {
        let url = file.map(|f| self.url_for(f));
        self.add_pane_url(wid, url)
    }

    fn add_pane_url(&mut self, wid: WindowId, url: Option<String>) -> PaneId {
        let id = self.next_pane;
        self.next_pane += 1;
        let url = url.unwrap_or_else(|| format!("{}/", paths::file_origin()));
        let window = &self.shells[&wid].window;
        let (content, watcher) = build_content(&mut self.ctx, window, &self.proxy, id, &url);
        self.panes.insert(
            id,
            Pane {
                shell: wid,
                content,
                watcher,
                current: None,
                url: String::new(),
                restoring: false,
                banner: None,
                preview: None,
                inset: 0.0,
                rect: Default::default(),
                loading: None,
                loading_shown: false,
            },
        );
        if let Some(sh) = self.shells.get_mut(&wid) {
            sh.panes.push(id);
        }
        id
    }

    fn create_shell(&mut self, target: &EventLoopWindowTarget<UserEvent>, kind: Kind, geom: Option<state::WinGeom>) -> WindowId {
        let default = if kind == Kind::Main { LogicalSize::new(1200.0, 800.0) } else { LogicalSize::new(900.0, 800.0) };
        let window = build_window(target, geom, default);
        let ui = build_ui(&mut self.ctx, &window, &self.proxy, kind);
        let wid = window.id();
        self.shells.insert(wid, Shell { kind, ui, window, ui_dpr: None, panes: Vec::new(), focused: 0 });
        wid
    }

    /// A new pop-out window showing `file`.
    fn pop_out(&mut self, target: &EventLoopWindowTarget<UserEvent>, file: Option<&str>, geom: Option<state::WinGeom>) {
        let url = file.map(|f| self.url_for(f));
        self.pop_out_url(target, url, geom);
    }

    /// A new pop-out window showing this content URL (it may carry a query or #fragment).
    fn pop_out_url(&mut self, target: &EventLoopWindowTarget<UserEvent>, url: Option<String>, geom: Option<state::WinGeom>) {
        let geom = geom.or_else(|| {
            // Cascade from the main window.
            let w = &self.shells.get(&self.main)?.window;
            let pos = w.outer_position().ok()?;
            let scale = w.scale_factor();
            Some(state::WinGeom {
                x: pos.x + (60.0 * scale) as i32,
                y: pos.y + (60.0 * scale) as i32,
                w: (900.0 * scale) as u32,
                h: (800.0 * scale) as u32,
                maximized: false,
            })
        });
        let wid = self.create_shell(target, Kind::Popout, geom);
        let id = self.add_pane_url(wid, url);
        if let Some(sh) = self.shells.get_mut(&wid) {
            sh.focused = id;
        }
        self.popout_order.push(wid);
        self.layout(wid);
        self.mark_dirty();
    }

    fn close_pane(&mut self, id: PaneId) {
        let Some(pane) = self.panes.remove(&id) else { return };
        pane.watcher.set(None);
        let wid = pane.shell;
        drop(pane); // destroys the webview
        if let Some(sh) = self.shells.get_mut(&wid) {
            sh.panes.retain(|p| *p != id);
            if sh.focused == id {
                sh.focused = sh.panes.first().copied().unwrap_or(0);
            }
        }
        self.after_focus_change(wid);
        self.layout(wid);
        self.mark_dirty();
    }

    fn close_popout(&mut self, wid: WindowId) {
        let Some(sh) = self.shells.remove(&wid) else { return };
        for id in &sh.panes {
            if let Some(p) = self.panes.remove(id) {
                p.watcher.set(None);
            }
        }
        self.popout_order.retain(|w| *w != wid);
        drop(sh);
        self.mark_dirty();
    }

    fn set_focus(&mut self, id: PaneId) {
        let Some(wid) = self.panes.get(&id).map(|p| p.shell) else { return };
        let Some(sh) = self.shells.get_mut(&wid) else { return };
        if sh.focused != id {
            sh.focused = id;
            self.after_focus_change(wid);
            self.layout(wid);
        }
    }

    /// The window title and (main window) the tree selection follow the focused pane.
    fn after_focus_change(&mut self, wid: WindowId) {
        let Some(sh) = self.shells.get(&wid) else { return };
        let file = self.panes.get(&sh.focused).and_then(|p| p.current.clone());
        let env = if self.env.label.is_empty() { String::new() } else { format!(" ({})", self.env.label) };
        let title = match &file {
            Some(p) => format!("{} - Condor{env}", platform::file_name(p)),
            None => format!("Condor{env}"),
        };
        sh.window.set_title(&title);
        if sh.kind == Kind::Main {
            self.send_main(&json!({"ev": "selected", "path": file}));
        }
    }

    /// Position the UI and the panes of a window, and tell its UI where things are.
    fn layout(&self, wid: WindowId) {
        let Some(sh) = self.shells.get(&wid) else { return };
        let size = sh.window.inner_size();
        let dpr = sh.ui_dpr.unwrap_or_else(|| sh.window.scale_factor());
        let _ = sh.ui.set_bounds(Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(size.width, size.height).into(),
        });
        let (ww, wh) = (size.width as f64 / dpr, size.height as f64 / dpr);
        let x0 = if sh.kind == Kind::Main { self.st.sidebar_width + SPLITTER } else { 0.0 };
        let area = (x0, 0.0, (ww - x0).max(0.0), wh);
        let mut rects = Vec::new();
        let mut divider = serde_json::Value::Null;
        if sh.panes.len() >= 2 {
            let r = self.st.split_ratio.clamp(0.1, 0.9);
            if self.st.split_side_by_side {
                let w1 = ((area.2 - DIVIDER) * r).round();
                rects.push((area.0, area.1, w1, area.3));
                rects.push((area.0 + w1 + DIVIDER, area.1, area.2 - w1 - DIVIDER, area.3));
                divider = json!({"x": area.0 + w1, "y": area.1, "w": DIVIDER, "h": area.3});
            } else {
                let h1 = ((area.3 - DIVIDER) * r).round();
                rects.push((area.0, area.1, area.2, h1));
                rects.push((area.0, area.1 + h1 + DIVIDER, area.2, area.3 - h1 - DIVIDER));
                divider = json!({"x": area.0, "y": area.1 + h1, "w": area.2, "h": DIVIDER});
            }
        } else {
            rects.push(area);
        }
        let phys = |v: f64| (v * dpr).round().max(0.0);
        let mut panes = Vec::new();
        for (id, r) in sh.panes.iter().zip(rects) {
            let Some(p) = self.panes.get(id) else { continue };
            let top = PANE_BAR + if p.banner.is_some() { BANNER } else { 0.0 } + p.inset + FRAME;
            let (x, y) = (phys(r.0), phys(r.1 + top));
            let (w, h) = (phys(r.0 + r.2) - x, (phys(r.1 + r.3) - y).max(0.0));
            let _ = p.content.set_bounds(Rect {
                position: PhysicalPosition::new(x as i32, y as i32).into(),
                size: PhysicalSize::new(w.max(0.0) as u32, h as u32).into(),
            });
            p.rect.set((x as i32, y as i32, w.max(0.0) as i32, h as i32));
            if let Some(pv) = &p.preview {
                pv.set_rect(p.rect.get());
            }
            panes.push(json!({"id": id, "x": r.0, "y": r.1, "w": r.2, "h": r.3, "focused": *id == sh.focused}));
        }
        self.send_ui(
            wid,
            &json!({"ev": "layout", "panes": panes, "divider": divider, "area": {"x": area.0, "y": area.1, "w": area.2, "h": area.3},
                    "sideBySide": self.st.split_side_by_side, "split": sh.panes.len() >= 2}),
        );
    }

    /// Tell a pane's UI about its file and the file's settings.
    fn send_pane_state(&self, id: PaneId) {
        let Some(p) = self.panes.get(&id) else { return };
        let can = p.current.as_deref().map_or(false, render::has_source_view);
        let auto = p.current.as_ref().map_or(false, |f| self.st.auto_reload.contains(f));
        // Office files: which viewer shows it, and whether the other one is available.
        let office = match &p.current {
            Some(f) if p.preview.is_some() && convert::convertible(f) && convert::available() => Some("previewer"),
            Some(f) if render::is_converted_url(&p.url) && platform::has_previewer(f) => Some("libreoffice"),
            _ => None,
        };
        self.send_ui(
            p.shell,
            &json!({"ev": "pane", "id": id, "file": p.current, "canSource": can,
                    "source": can && render::is_source_url(&p.url), "auto": auto, "office": office,
                    "loading": p.loading_shown}),
        );
        if let Some((back, fwd)) = platform::history_state(&p.content) {
            self.send_ui(p.shell, &json!({"ev": "history", "id": id, "back": back, "forward": fwd}));
        }
    }

    /// Every window shows tags (the tree's badges, the panes' star buttons).
    fn send_tags(&self) {
        let msg = json!({"ev": "tags", "tags": self.st.tags, "filter": self.st.tag_filter});
        for w in self.shells.keys() {
            self.send_ui(*w, &msg);
        }
    }

    /// How an Office file is shown unless the reader chose otherwise for it: the global
    /// setting, where "auto" means the Windows previewer, except for types whose previewer
    /// often shows nothing (Visio's, without Visio installed), which go to LibreOffice.
    fn office_default(&self, path: &str) -> &'static str {
        office_mode(&self.settings, path)
    }

    /// File types the tree shows undimmed besides the viewable list: those with highlighting,
    /// and Office documents when a previewer or LibreOffice can show them.
    fn showable(&self) -> Vec<String> {
        let mut v = highlight::extensions();
        if self.settings.office_view != "off" && (cfg!(windows) || convert::available()) {
            v.extend(convert::EXTS.iter().map(|s| s.to_string()));
        }
        v
    }

    fn init_message(&self, wid: WindowId) -> serde_json::Value {
        let kind = self.shells.get(&wid).map_or(Kind::Main, |s| s.kind);
        let selected = self.shells.get(&wid).and_then(|s| self.panes.get(&s.focused)).and_then(|p| p.current.clone());
        json!({
            "ev": "init",
            "mode": if kind == Kind::Main { "main" } else { "popout" },
            "env": self.env.label,
            "update": self.update_ready,
            "roots": self.st.roots,
            "openWith": platform::CAN_OPEN_WITH,
            "settings": self.settings,
            "expanded": self.st.expanded,
            "selected": selected.or_else(|| self.st.open_file.clone()),
            "sidebar": self.st.sidebar_width,
            "filters": self.filter_state(),
            "tags": self.st.tags,
            "tagFilter": self.st.tag_filter,
            "star": state::STAR,
            // Shown in full in the tree; other files are dimmed (Condor can't preview them).
            "showable": self.showable(),
        })
    }

    // ----- commands from the UI -------------------------------------------------------

    fn handle_cmd(&mut self, target: &EventLoopWindowTarget<UserEvent>, wid: WindowId, cmd: Cmd) {
        let is_main = wid == self.main;
        match cmd {
            Cmd::Ready => {
                self.send_ui(wid, &self.init_message(wid));
                self.layout(wid);
                let panes = self.shells.get(&wid).map(|s| s.panes.clone()).unwrap_or_default();
                for id in panes {
                    self.send_pane_state(id);
                    if let Some(b) = self.panes.get(&id).and_then(|p| p.banner.clone()) {
                        self.send_ui(wid, &b);
                    }
                }
            }
            Cmd::List { path } => list_dir(path, self.filters.clone(), self.proxy.clone()),
            Cmd::Open { path } => {
                let focused = self.shells.get(&self.main).map(|s| s.focused);
                if let Some(p) = focused.and_then(|f| self.panes.get(&f)) {
                    debug(|| format!("open {path}"));
                    let _ = p.content.load_url(&self.url_for(&path));
                }
            }
            Cmd::Expand { path, open } => {
                let changed = if open { self.st.expanded.insert(path) } else { self.st.expanded.remove(&path) };
                if changed {
                    self.mark_dirty();
                }
            }
            Cmd::Sidebar { width, dpr } => {
                if let Some(sh) = self.shells.get_mut(&wid) {
                    if let Some(d) = dpr.filter(|d| d.is_finite() && *d > 0.2 && *d < 10.0) {
                        sh.ui_dpr = Some(d);
                    }
                }
                if is_main && width.is_finite() {
                    let w = width.clamp(120.0, 2000.0);
                    if w != self.st.sidebar_width {
                        self.st.sidebar_width = w;
                        self.mark_dirty();
                    }
                }
                self.layout(wid);
            }
            Cmd::AddRoot => {
                // Blocks this thread while the dialog is open, like any modal dialog.
                if let Some(p) = platform::pick_folder("Add folder to Condor") {
                    self.add_root(&p.to_string_lossy());
                }
            }
            Cmd::RemoveRoot { path } => {
                self.st.roots.retain(|r| r != &path);
                self.rebuild_filters();
                // Forget saved expansion under that root, so re-adding starts fresh.
                self.st.expanded.retain(|e| !platform::is_under(e, &path));
                self.send_main(&json!({"ev": "roots", "roots": self.st.roots}));
                self.mark_dirty();
            }
            Cmd::MoveRoot { path, to } => {
                if let Some(i) = self.st.roots.iter().position(|r| r == &path) {
                    let r = self.st.roots.remove(i);
                    let to = to.min(self.st.roots.len());
                    self.st.roots.insert(to, r);
                    self.send_main(&json!({"ev": "roots", "roots": self.st.roots}));
                    self.mark_dirty();
                }
            }
            Cmd::Restart => {
                // Collect scroll positions first; the replies arrive as Scroll events.
                self.poll_scroll();
                self.restart_at = Some(Instant::now() + Duration::from_millis(400));
            }
            Cmd::SetFilters { show_hidden, hide_ignored, viewable_only, excludes } => {
                self.st.show_hidden = show_hidden;
                self.st.hide_ignored = hide_ignored;
                self.st.viewable_only = viewable_only;
                self.st.excludes = excludes.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                gitfilter::clear_cache();
                self.rebuild_filters();
                self.mark_dirty();
                self.send_main(&json!({"ev": "filter", "filters": self.filter_state()}));
            }
            Cmd::OpenExternal { path, with } => {
                // On a worker thread: checking the file may be slow on a share.
                std::thread::spawn(move || {
                    if !std::path::Path::new(&path).is_file() {
                        return;
                    }
                    if with {
                        platform::open_with(&path);
                    } else if !platform::is_executable(&path)
                        || platform::confirm(
                            "Run program?",
                            &format!("{} is a program or script. Opening it will run it.\n\nRun it?", platform::file_name(&path)),
                        )
                    {
                        platform::open_path(&path);
                    }
                });
            }
            Cmd::SetSettings { settings } => {
                let s = settings.normalised();
                if let Err(e) = state::save_settings(&s) {
                    log(&format!("saving settings failed: {e}"));
                }
                self.settings_sig = watch::signature(&state::settings_file().to_string_lossy());
                self.apply_settings(s);
            }
            Cmd::CopyFile { path } => {
                let ok = platform::clipboard_files(&[path.clone()]);
                self.flash(wid, None, if ok { format!("Copied {}", platform::file_name(&path)) } else { "Couldn't copy".into() });
            }
            Cmd::CopyPath { path } => {
                let ok = platform::clipboard_text(&path);
                self.flash(wid, None, if ok { "Copied the path".into() } else { "Couldn't copy".into() });
            }
            Cmd::OpenTerminal { path } => {
                std::thread::spawn(move || platform::open_terminal(&path));
            }
            Cmd::CopyPane { pane } => {
                let Some(id) = self.pane_of(wid, pane) else { return };
                let Some(file) = self.panes[&id].current.clone() else { return };
                let proxy = self.proxy.clone();
                let selected = move |sel: String, proxy: EventLoopProxy<UserEvent>, file: String| {
                    // On a worker thread: reading the file may be slow.
                    std::thread::spawn(move || {
                        let (ok, what) = if !sel.is_empty() {
                            (platform::clipboard_text(&sel), "Copied the selection")
                        } else if let Some(text) = text_of_file(&file) {
                            (platform::clipboard_text(&text), "Copied the text")
                        } else {
                            (platform::clipboard_files(&[file.clone()]), "Copied the file")
                        };
                        let msg = if ok { what } else { "Couldn't copy" };
                        let _ = proxy.send_event(UserEvent::Flash(wid, Some(id), msg.to_string()));
                    });
                };
                let p = &self.panes[&id];
                if p.preview.is_some() {
                    selected(String::new(), proxy, file);
                } else {
                    let _ = p.content.evaluate_script_with_callback("String(window.getSelection ? getSelection() : '')", move |r| {
                        let sel: String = serde_json::from_str(&r).unwrap_or_default();
                        selected(sel, proxy.clone(), file.clone());
                    });
                }
            }
            Cmd::AutoReload { path, on } => {
                let changed = if on { self.st.auto_reload.insert(path.clone()) } else { self.st.auto_reload.remove(&path) };
                if changed {
                    self.mark_dirty();
                }
                let showing: Vec<PaneId> =
                    self.panes.iter().filter(|(_, p)| p.current.as_deref() == Some(path.as_str())).map(|(id, _)| *id).collect();
                for id in showing {
                    let changed_banner = self.panes[&id].banner.as_ref().map_or(false, |b| b["kind"] == "changed");
                    if on && changed_banner {
                        self.reload_keep_scroll(id);
                    }
                    self.send_pane_state(id);
                }
            }
            Cmd::Reload { pane } => {
                if let Some(id) = self.pane_of(wid, pane) {
                    self.reload_keep_scroll(id);
                }
            }
            Cmd::Dismiss { pane } => {
                if let Some(id) = self.pane_of(wid, pane) {
                    self.set_banner(id, None);
                }
            }
            Cmd::Back { pane } => {
                if let Some(id) = self.pane_of(wid, pane) {
                    debug(|| format!("back {id}"));
                    platform::history_go(&self.panes[&id].content, true);
                }
            }
            Cmd::Forward { pane } => {
                if let Some(id) = self.pane_of(wid, pane) {
                    platform::history_go(&self.panes[&id].content, false);
                }
            }
            Cmd::ToggleSource { pane } => {
                let Some(id) = self.pane_of(wid, pane) else { return };
                let p = &self.panes[&id];
                let Some(file) = p.current.clone().filter(|f| render::has_source_view(f)) else { return };
                let to_source = !render::is_source_url(&p.url);
                if to_source {
                    self.st.source_view.insert(file.clone());
                } else {
                    self.st.source_view.remove(&file);
                }
                self.mark_dirty();
                let _ = self.panes[&id].content.load_url(&self.url_for(&file));
            }
            Cmd::Focus { pane } => self.set_focus(pane),
            Cmd::Split { pane } => {
                if !is_main {
                    return;
                }
                let n = self.shells[&self.main].panes.len();
                if n >= 2 {
                    self.st.split_side_by_side = !self.st.split_side_by_side;
                } else {
                    let src = self.pane_of(wid, pane);
                    let file = src.and_then(|p| self.panes[&p].current.clone());
                    let id = self.add_pane(self.main, file.as_deref());
                    self.set_focus(id);
                }
                self.layout(self.main);
                self.mark_dirty();
            }
            Cmd::ClosePane { pane } => {
                let in_split = self.panes.get(&pane).map_or(false, |p| p.shell == self.main) && self.shells[&self.main].panes.len() > 1;
                if in_split {
                    self.close_pane(pane);
                }
            }
            Cmd::PopOut { pane, path } => {
                let file = path.or_else(|| self.pane_of(wid, pane).and_then(|p| self.panes[&p].current.clone()));
                self.pop_out(target, file.as_deref(), None);
                // A pane in a split moves to the new window.
                if let Some(id) = pane {
                    if is_main && self.shells[&self.main].panes.len() > 1 {
                        self.close_pane(id);
                    }
                }
            }
            Cmd::OpenOther { path } => {
                let main = &self.shells[&self.main];
                let other = main.panes.iter().copied().find(|p| *p != main.focused);
                match other {
                    Some(id) => {
                        let _ = self.panes[&id].content.load_url(&self.url_for(&path));
                    }
                    None => {
                        let focused = main.focused;
                        self.add_pane(self.main, Some(&path));
                        self.layout(self.main);
                        self.set_focus(focused);
                        self.mark_dirty();
                    }
                }
            }
            Cmd::SetTags { path, dir, tags } => {
                let tags: std::collections::BTreeSet<String> = tags.iter().filter_map(|t| state::clean_tag(t)).collect();
                if tags.is_empty() {
                    self.st.tags.remove(&path);
                } else {
                    self.st.tags.insert(path, state::Tagged { dir, tags });
                }
                // Drop filter entries for tags nothing has any more.
                let used = |t: &String| self.st.tags.values().any(|v| v.tags.contains(t));
                let filter: Vec<String> = self.st.tag_filter.iter().filter(|t| used(t)).cloned().collect();
                self.st.tag_filter = filter;
                self.mark_dirty();
                self.send_tags();
            }
            Cmd::TagFilter { tags } => {
                self.st.tag_filter = tags;
                self.mark_dirty();
                self.send_tags();
            }
            Cmd::PaneInset { pane, px } => {
                if let Some(p) = self.panes.get_mut(&pane) {
                    let px = if px.is_finite() { px.clamp(0.0, 600.0) } else { 0.0 };
                    if p.inset != px {
                        p.inset = px;
                        let wid = p.shell;
                        self.layout(wid);
                    }
                }
            }
            Cmd::Search { query, id } => {
                let (roots, filters, proxy) = (self.st.roots.clone(), self.filters.clone(), self.proxy.clone());
                std::thread::spawn(move || {
                    let t = Instant::now();
                    let hits = search::search(&roots, &filters, &query);
                    let ms = t.elapsed().as_millis() as u64;
                    let msg = json!({"ev": "results", "id": id, "query": query, "hits": hits, "ms": ms});
                    let _ = proxy.send_event(UserEvent::Listed(msg.to_string()));
                });
            }
            Cmd::OfficeView { pane, mode } => {
                let Some(id) = self.pane_of(wid, pane) else { return };
                let Some(file) = self.panes[&id].current.clone() else { return };
                if !matches!(mode.as_str(), "previewer" | "libreoffice") {
                    return;
                }
                // The choice is for the file type, shared by all windows (settings.json).
                let ext = convert::ext_of(&file);
                let mut s = self.settings.clone();
                s.office_by_type.remove(&ext);
                let natural = office_mode(&s, &file);
                if mode != natural {
                    s.office_by_type.insert(ext.clone(), mode);
                }
                if let Err(e) = state::save_settings(&s) {
                    log(&format!("saving settings failed: {e}"));
                }
                self.settings_sig = watch::signature(&state::settings_file().to_string_lossy());
                self.apply_settings(s);
            }
            Cmd::PaneRatio { ratio } => {
                if ratio.is_finite() {
                    self.st.split_ratio = ratio.clamp(0.1, 0.9);
                    self.layout(self.main);
                    self.mark_dirty();
                }
            }
        }
    }

    fn add_root(&mut self, p: &str) {
        let p = platform::normalise_path(p);
        if !self.st.roots.iter().any(|r| r.eq_ignore_ascii_case(&p)) {
            self.st.roots.push(p.clone());
            self.st.expanded.insert(p);
            self.rebuild_filters();
            self.send_main(&json!({"ev": "roots", "roots": self.st.roots, "expanded": self.st.expanded}));
            self.mark_dirty();
        }
    }

    // ----- pane events ----------------------------------------------------------------

    fn on_content_nav(&mut self, id: PaneId, url: &str) {
        debug(|| format!("nav {id} {url}"));
        let path = paths::url_to_path(url);
        let Some(p) = self.panes.get_mut(&id) else { return };
        if p.preview.take().is_some() {
            let _ = p.content.set_visible(true);
        }
        let moved = path != p.current;
        let page = |u: &str| u.split('#').next().unwrap_or("").to_string();
        if page(url) != page(&p.url) {
            p.loading = Some(Instant::now() + LOADING_SHOWN_AFTER);
            p.loading_shown = false;
        }
        p.current = path.clone();
        p.url = url.to_string();
        if moved {
            p.restoring = path.as_ref().map_or(false, |f| self.st.scroll.contains_key(f));
            self.set_banner(id, None);
        }
        let wid = self.panes[&id].shell;
        if self.shells.get(&wid).map_or(false, |s| s.focused == id) {
            self.after_focus_change(wid);
        }
        self.send_pane_state(id);
        self.mark_dirty();
    }

    fn on_no_preview(&mut self, id: PaneId, path: &str) {
        let Some(p) = self.panes.get(&id) else { return };
        if p.current.as_deref() != Some(path) || p.banner.is_some() {
            return; // moved on, or a change banner is already showing
        }
        let mode = self.office_default(path).to_string();
        if matches!(mode.as_str(), "auto" | "previewer") {
            let hwnd = platform::window_handle(&self.shells[&p.shell].window);
            match platform::Preview::open(hwnd, path, p.rect.get()) {
                Ok(pv) => {
                    debug(|| format!("previewer for {path}"));
                    let p = self.panes.get_mut(&id).unwrap();
                    let _ = p.content.set_visible(false);
                    p.preview = Some(pv);
                    self.send_pane_state(id);
                    return;
                }
                Err(e) => debug(|| format!("previewer for {path}: {e}")),
            }
        }
        let p = &self.panes[&id];
        // LibreOffice if chosen, or when the previewer failed (unless the setting says previewer only).
        let lo = mode == "libreoffice" || (mode != "off" && self.settings.office_view != "previewer");
        if lo && convert::convertible(path) && convert::available() {
            // Replace (not add to) the history entry, so Back skips the placeholder.
            let url = format!("{}?converted", paths::path_to_url(path));
            let _ = p.content.evaluate_script(&format!("location.replace({})", json!(url)));
            let msg = json!({"ev": "banner", "pane": id, "show": true, "kind": "converting", "path": path});
            self.set_banner(id, Some(msg));
            return;
        }
        let msg = json!({"ev": "banner", "pane": id, "show": true, "kind": "open", "path": path, "openWith": platform::CAN_OPEN_WITH});
        self.set_banner(id, Some(msg));
    }

    fn set_banner(&mut self, id: PaneId, msg: Option<serde_json::Value>) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        if p.banner.is_none() && msg.is_none() {
            return;
        }
        p.banner = msg.clone();
        let wid = p.shell;
        self.send_ui(wid, &msg.unwrap_or_else(|| json!({"ev": "banner", "pane": id, "show": false})));
        self.layout(wid);
    }

    /// Reload a pane, keeping the reader's scroll position.
    fn reload_keep_scroll(&mut self, id: PaneId) {
        self.set_banner(id, None);
        let Some(p) = self.panes.get(&id) else { return };
        p.watcher.reset();
        let proxy = self.proxy.clone();
        let fallback = self.proxy.clone();
        let r = p.content.evaluate_script_with_callback("[location.href, window.scrollY]", move |res| {
            let v: serde_json::Value = serde_json::from_str(&res).unwrap_or_default();
            let url = v.get(0).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let y = v.get(1).and_then(|x| x.as_f64()).unwrap_or(-1.0);
            let _ = proxy.send_event(UserEvent::ReloadAt(id, url, y));
        });
        if r.is_err() {
            let _ = fallback.send_event(UserEvent::ReloadAt(id, String::new(), -1.0));
        }
    }

    fn on_reload_at(&mut self, id: PaneId, url: &str, y: f64) {
        let Some(p) = self.panes.get_mut(&id) else { return };
        if let (Some(f), true) = (paths::url_to_path(url), y >= 0.0 && y.is_finite()) {
            if Some(&f) == p.current.as_ref() && !render::is_source_url(url) {
                p.restoring = true;
                self.st.set_scroll(&f, y.round());
            }
        }
        let _ = self.panes[&id].content.reload();
    }

    fn on_file_changed(&mut self, id: PaneId, path: &str, deleted: bool) {
        if self.panes.get(&id).and_then(|p| p.current.as_deref()) != Some(path) {
            return;
        }
        if !deleted && self.st.auto_reload.contains(path) {
            self.reload_keep_scroll(id);
            return;
        }
        let msg = json!({"ev": "banner", "pane": id, "show": true, "kind": "changed", "path": path, "deleted": deleted});
        self.set_banner(id, Some(msg));
    }

    fn on_page_loaded(&mut self, id: PaneId, url: &str) {
        debug(|| format!("loaded {id} {url}"));
        if let Some(p) = self.panes.get_mut(&id) {
            p.loading = None;
            p.loading_shown = false;
        }
        self.send_pane_state(id);
        let Some(p) = self.panes.get_mut(&id) else { return };
        let path = paths::url_to_path(url);
        if render::is_converted_url(url) {
            let converting = p.banner.as_ref().map_or(false, |b| b["kind"] == "converting");
            if converting {
                self.set_banner(id, None);
            }
            return;
        }
        if let Some(f) = path.clone() {
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                if !render::previewable(&f) {
                    let _ = proxy.send_event(UserEvent::NoPreview(id, f));
                }
            });
        }
        if path.is_none() || path != p.current {
            return;
        }
        let f = path.unwrap();
        // ui/content.js restores from this session's memory if it has one, else from the
        // saved state. Pages it doesn't run in (e.g. PDFs) get a plain scroll.
        let saved = if render::is_source_url(url) {
            json!({"y": 0})
        } else {
            json!({"y": self.st.scroll.get(&f).copied().unwrap_or(0.0), "sel": self.st.selection.get(&f)})
        };
        let js = format!(
            "(function(st){{if(window.__condor){{window.__condor.restore(st);return}}\
             var n=0;function f(){{window.scrollTo(0,st.y);\
             if(Math.abs(window.scrollY-st.y)>2&&n++<30)setTimeout(f,100)}}if(st.y>0)f()}})({saved})"
        );
        let _ = p.content.evaluate_script(&js);
        p.restoring = false;
        // Give the restore a moment before polling, so we don't record a pre-restore 0.
        self.next_poll = Instant::now() + Duration::from_millis(3500);
    }

    /// Pick up settings changed by another window.
    fn check_settings(&mut self) {
        let sig = watch::signature(&state::settings_file().to_string_lossy());
        if sig != self.settings_sig {
            self.settings_sig = sig;
            self.apply_settings(state::load_settings());
        }
    }

    fn apply_settings(&mut self, s: state::Settings) {
        if s == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, s);
        if old.viewable_exts != self.settings.viewable_exts {
            self.rebuild_filters();
            self.send_main(&json!({"ev": "filter", "filters": self.filter_state()}));
        }
        if old.office_view != self.settings.office_view || old.office_by_type != self.settings.office_by_type {
            // Show Office files again with the viewer now chosen for them.
            let ids: Vec<PaneId> = self
                .panes
                .iter()
                .filter(|(_, p)| p.current.as_deref().map_or(false, convert::convertible))
                .map(|(id, _)| *id)
                .collect();
            for id in ids {
                let p = &self.panes[&id];
                let _ = p.content.load_url(&paths::path_to_url(p.current.as_deref().unwrap_or_default()));
            }
        }
        if old.doc_width != self.settings.doc_width {
            render::set_width(&self.settings.doc_width);
            let ids: Vec<PaneId> = self.panes.keys().copied().collect();
            for id in ids {
                self.reload_keep_scroll(id);
            }
        }
        let wids: Vec<WindowId> = self.shells.keys().copied().collect();
        for w in wids {
            self.send_ui(w, &json!({"ev": "settings", "settings": self.settings}));
        }
    }

    /// Folders showing their contents in the tree: roots and expanded folders whose
    /// ancestors (up to their root) are all expanded.
    fn open_dirs(&self) -> Vec<String> {
        let ex = &self.st.expanded;
        let mut out = Vec::new();
        for d in ex {
            let Some(root) = self.st.roots.iter().find(|r| platform::is_under(d, r)) else { continue };
            let mut p = std::path::Path::new(d);
            let mut shown = true;
            while platform::path_key(&p.to_string_lossy()) != platform::path_key(root) {
                match p.parent() {
                    Some(q) => p = q,
                    None => break,
                }
                if !ex.contains(&*p.to_string_lossy()) {
                    shown = false;
                    break;
                }
            }
            if shown {
                out.push(d.clone());
            }
        }
        out
    }

    /// Re-list open folders whose contents changed on disk (by their modification time).
    fn check_dirs(&mut self) {
        use std::sync::atomic::Ordering;
        if self.dir_check.swap(true, Ordering::SeqCst) {
            return; // the last check is still going (a slow share)
        }
        let dirs = self.open_dirs();
        let (times, busy, filters, proxy) = (self.dir_times.clone(), self.dir_check.clone(), self.filters.clone(), self.proxy.clone());
        std::thread::spawn(move || {
            let mut seen = HashMap::new();
            for d in dirs {
                let Ok(t) = std::fs::metadata(&d).and_then(|m| m.modified()) else { continue };
                let before = times.lock().unwrap().get(&d).copied();
                if before.map_or(false, |b| b != t) {
                    list_dir(d.clone(), filters.clone(), proxy.clone());
                }
                seen.insert(d, t);
            }
            *times.lock().unwrap() = seen;
            busy.store(false, Ordering::SeqCst);
        });
    }

    fn poll_scroll(&mut self) {
        self.next_poll = Instant::now() + SCROLL_POLL;
        for (id, p) in &self.panes {
            if p.restoring || p.current.is_none() {
                continue;
            }
            let (proxy, id) = (self.proxy.clone(), *id);
            let _ = p.content.evaluate_script_with_callback(
                "[location.href, window.scrollY, window.__condor ? window.__condor.state().sel : null]",
                move |res| {
                    if let Ok(serde_json::Value::Array(v)) = serde_json::from_str::<serde_json::Value>(&res) {
                        if let (Some(u), Some(y)) = (v.first().and_then(|x| x.as_str()), v.get(1).and_then(|x| x.as_f64())) {
                            let sel = v.get(2).cloned().unwrap_or_default();
                            let _ = proxy.send_event(UserEvent::Scroll(id, u.to_string(), y, sel));
                        }
                    }
                },
            );
        }
    }

    /// Save everything and start the new build with the same environment; true if it started.
    fn restart(&mut self) -> bool {
        let Some(exe) = self.exe.clone() else { return false };
        // Stop answering for the environment first, or the new process would just bring
        // this window to the front and quit.
        self.finish();
        match std::process::Command::new(exe).arg("--state").arg(&self.env.state_file).spawn() {
            Ok(_) => true,
            Err(e) => {
                log(&format!("restart failed: {e}"));
                false
            }
        }
    }

    /// The file a pane's polled URL refers to, if it is still the pane's current rendered view.
    fn polled_file(&self, id: PaneId, url: &str) -> Option<String> {
        let p = self.panes.get(&id)?;
        // Saved positions are for the rendered view (the source view's last only a session).
        let f = paths::url_to_path(url).filter(|_| !render::is_source_url(url))?;
        (Some(&f) == p.current.as_ref() && !p.restoring).then_some(f)
    }

    fn on_scroll(&mut self, id: PaneId, url: &str, y: f64, sel: serde_json::Value) {
        let Some(f) = self.polled_file(id, url) else { return };
        if y.is_finite() {
            let y = y.max(0.0).round();
            let old = self.st.scroll.get(&f).copied();
            if old != Some(y) && !(old.is_none() && y == 0.0) {
                self.st.set_scroll(&f, y);
                self.mark_dirty();
            }
        }
        let changed = if sel.is_null() {
            self.st.selection.remove(&f).is_some()
        } else if self.st.selection.get(&f) != Some(&sel) {
            self.st.selection.insert(f.clone(), sel);
            true
        } else {
            false
        };
        if changed {
            if !self.st.scroll.contains_key(&f) {
                self.st.set_scroll(&f, 0.0); // keeps the file in the capped recent list
            }
            self.mark_dirty();
        }
    }
}

impl App {
    /// Open the main window (and pop-outs) for a loaded environment.
    fn start(target: &EventLoopWindowTarget<UserEvent>, ctx: WebContext, proxy: EventLoopProxy<UserEvent>, env: envs::Env, st: state::EnvState) -> App {
        render::set_roots(&st.roots);
        let settings = state::load_settings();
        render::set_width(&settings.doc_width);

        // The main window comes first so its id is known; the shell map gets it right after.
        let main_window = build_window(target, st.window, LogicalSize::new(1200.0, 800.0));
        let main = main_window.id();
        let mut app = App {
            env,
            saver: envs::Saver::start(),
            proxy,
            ctx,
            main,
            shells: HashMap::new(),
            popout_order: Vec::new(),
            panes: HashMap::new(),
            next_pane: 1,
            save_at: None,
            next_poll: Instant::now() + SCROLL_POLL,
            last_saved: None,
            filters: Arc::new(filters::Filters::from_state(&st, &settings)),
            exe: update::exe_path(),
            update_ready: false,
            restart_at: None,
            settings_sig: watch::signature(&state::settings_file().to_string_lossy()),
            dir_times: Default::default(),
            dir_check: Default::default(),
            next_dir_check: Instant::now() + DIR_CHECK,
            settings,
            st,
        };
        let ui = build_ui(&mut app.ctx, &main_window, &app.proxy, Kind::Main);
        app.shells.insert(main, Shell { kind: Kind::Main, ui, window: main_window, ui_dpr: None, panes: Vec::new(), focused: 0 });
        let first = app.st.open_file.clone();
        let id = app.add_pane(main, first.as_deref());
        app.shells.get_mut(&main).unwrap().focused = id;
        if app.st.split {
            let second = app.st.split_file.clone();
            app.add_pane(main, second.as_deref());
        }
        app.layout(main);
        for po in app.st.popouts.clone() {
            app.pop_out(target, po.file.as_deref(), po.window);
        }
        app.save_now();
        if let Some(exe) = app.exe.clone() {
            let p = app.proxy.clone();
            update::watch(exe, move || {
                let _ = p.send_event(UserEvent::UpdateAvailable);
            });
        }
        let p = app.proxy.clone();
        envs::listen(&app.env, move || {
            let _ = p.send_event(UserEvent::Activate);
        });
        app
    }

    fn on_event(&mut self, event: Event<'_, UserEvent>, target: &EventLoopWindowTarget<UserEvent>, control_flow: &mut ControlFlow) {
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {}
            Event::UserEvent(ev) => match ev {
                UserEvent::Ipc(wid, s) => match serde_json::from_str::<Cmd>(&s) {
                    Ok(cmd) => self.handle_cmd(target, wid, cmd),
                    Err(e) => log(&format!("bad ipc {s}: {e}")),
                },
                UserEvent::Listed(msg) => {
                    if let Some(sh) = self.shells.get(&self.main) {
                        let _ = sh.ui.evaluate_script(&format!("window.condor && condor.recv({msg})"));
                    }
                }
                UserEvent::ContentNav(id, url) => self.on_content_nav(id, &url),
                UserEvent::PageLoaded(id, url) => self.on_page_loaded(id, &url),
                UserEvent::Scroll(id, url, y, sel) => self.on_scroll(id, &url, y, sel),
                UserEvent::FileChanged(id, p, deleted) => self.on_file_changed(id, &p, deleted),
                UserEvent::ReloadAt(id, url, y) => self.on_reload_at(id, &url, y),
                UserEvent::NoPreview(id, path) => self.on_no_preview(id, &path),
                UserEvent::OpenWindow(url) => self.pop_out_url(target, Some(url), None),
                UserEvent::LoadContent(id, url) => {
                    if let Some(p) = self.panes.get(&id) {
                        let _ = p.content.load_url(&url);
                    }
                }
                UserEvent::Focus(id) => self.set_focus(id),
                UserEvent::Activate => self.activate(),
                UserEvent::Flash(wid, pane, text) => self.flash(wid, pane, text),
                UserEvent::Opened(..) | UserEvent::Checked(..) => {}
                UserEvent::UpdateAvailable => {
                    self.update_ready = true;
                    let wids: Vec<WindowId> = self.shells.keys().copied().collect();
                    for w in wids {
                        self.send_ui(w, &json!({"ev": "update"}));
                    }
                }
            },
            Event::WindowEvent { window_id, event, .. } => match event {
                WindowEvent::CloseRequested => {
                    if window_id == self.main {
                        self.poll_scroll();
                        self.finish();
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                    self.close_popout(window_id);
                }
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    self.layout(window_id);
                    self.mark_dirty();
                }
                WindowEvent::Moved(_) => self.mark_dirty(),
                _ => {}
            },
            Event::LoopDestroyed => {
                self.save_now();
                self.saver.flush(Duration::from_secs(5));
            }
            _ => {}
        }

        let now = Instant::now();
        if self.restart_at.map_or(false, |t| now >= t) {
            self.restart_at = None;
            if self.restart() {
                *control_flow = ControlFlow::Exit;
                return;
            }
        }
        if now >= self.next_poll {
            self.poll_scroll();
            self.check_settings();
        }
        if now >= self.next_dir_check {
            self.next_dir_check = now + DIR_CHECK;
            self.check_dirs();
        }
        if self.save_at.map_or(false, |t| now >= t) {
            self.save_now();
        }
        self.show_loading(now);
        let mut wake = self.next_poll.min(self.next_dir_check);
        for p in self.panes.values() {
            if let (Some(t), false) = (p.loading, p.loading_shown) {
                wake = wake.min(t);
            }
        }
        for t in [self.save_at, self.restart_at].into_iter().flatten() {
            wake = wake.min(t);
        }
        *control_flow = ControlFlow::WaitUntil(wake);
    }

    /// Say "loading" in the bar of panes whose document is slow to arrive.
    fn show_loading(&mut self, now: Instant) {
        let due: Vec<PaneId> = self
            .panes
            .iter()
            .filter(|(_, p)| !p.loading_shown && p.loading.map_or(false, |t| now >= t))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            if let Some(p) = self.panes.get_mut(&id) {
                p.loading_shown = true;
            }
            self.send_pane_state(id);
        }
    }

    /// A short note ("Copied the path") in the window, by the pane's bar or the tree.
    fn flash(&self, wid: WindowId, pane: Option<PaneId>, text: String) {
        self.send_ui(wid, &json!({"ev": "flash", "pane": pane, "text": text}));
    }

    /// Another start of this environment: bring the main window to the front.
    fn activate(&mut self) {
        if let Some(sh) = self.shells.get(&self.main) {
            sh.window.set_visible(true);
            sh.window.set_minimized(false);
            sh.window.set_focus();
        }
    }

    /// Save, wait for the write, and give up the environment (on exit and restart).
    fn finish(&mut self) {
        self.save_now();
        self.saver.flush(Duration::from_secs(5));
        envs::touch_recent(&self.env);
        envs::unregister(&self.env);
    }
}

/// Before an environment is open: the selector window, and loading the chosen state on a
/// worker thread (a folder on a slow share shows "waiting", with a way back to the list).
struct Starter {
    picker: Option<Picker>,
    /// The folder a new environment can be made in (the current folder, if a real one).
    here: Option<String>,
    note: Option<String>,
    gen: u64,
    /// The environment being loaded: request number, which, when to show "waiting".
    pending: Option<(u64, envs::Env, Instant)>,
}

struct Picker {
    // Field order: the webview is dropped before its window.
    web: WebView,
    #[allow(dead_code)]
    window: Window,
}

enum Phase {
    Start(Starter, WebContext),
    Run(App),
    Switching,
}

/// A file's text, if it is text (UTF-8 without NULs, at most 8 MB), for copying.
fn text_of_file(path: &str) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    if m.len() > 8 << 20 {
        return None;
    }
    let b = std::fs::read(path).ok()?;
    if b.contains(&0) {
        return None;
    }
    String::from_utf8(b).ok()
}

fn waiting_msg(e: &envs::Env) -> serde_json::Value {
    let place = e.folder.clone().unwrap_or_else(|| e.state_file.to_string_lossy().into_owned());
    json!({"ev": "waiting", "name": envs::display_name(e), "place": place})
}

impl Starter {
    fn show(&mut self, target: &EventLoopWindowTarget<UserEvent>, ctx: &mut WebContext, proxy: &EventLoopProxy<UserEvent>) {
        if self.picker.is_some() {
            return;
        }
        let window = WindowBuilder::new()
            .with_title("Condor")
            .with_window_icon(platform::app_icon())
            .with_inner_size(LogicalSize::new(640.0, 500.0))
            .build(target)
            .expect("create window");
        if let Some(m) = window.current_monitor() {
            let (mp, ms, ws) = (m.position(), m.size(), window.outer_size());
            window.set_outer_position(PhysicalPosition::new(
                mp.x + (ms.width as i32 - ws.width as i32) / 2,
                mp.y + (ms.height as i32 - ws.height as i32) / 3,
            ));
        }
        let waiting = self.pending.as_ref().map(|(_, e, _)| waiting_msg(e));
        let init = json!({"items": envs::known(), "here": self.here, "note": self.note, "waiting": waiting});
        let (wid, p) = (window.id(), proxy.clone());
        let web = platform::tune_ui_webview(WebViewBuilder::new_with_web_context(ctx))
            .with_html(PICKER_HTML)
            .with_initialization_script(format!("window.PICKER_INIT = {init};"))
            .with_navigation_handler(|url| url.starts_with("about:") || url.starts_with("data:"))
            .with_ipc_handler(move |req| {
                let _ = p.send_event(UserEvent::Ipc(wid, req.into_body()));
            })
            .build(&window)
            .expect("create selector");
        self.picker = Some(Picker { web, window });
    }

    fn send(&self, msg: &serde_json::Value) {
        if let Some(pk) = &self.picker {
            let _ = pk.web.evaluate_script(&format!("window.picker && picker.recv({msg})"));
        }
    }

    /// Start loading `env`; false if it is already open in another Condor (now in front).
    fn begin(&mut self, env: envs::Env, proxy: &EventLoopProxy<UserEvent>) -> bool {
        if envs::activate_existing(&env) {
            return false;
        }
        self.gen += 1;
        let g = self.gen;
        self.send(&waiting_msg(&env));
        self.pending = Some((g, env.clone(), Instant::now() + WAIT_SHOWN_AFTER));
        let p = proxy.clone();
        std::thread::spawn(move || {
            let _ = p.send_event(UserEvent::Opened(g, envs::open(env)));
        });
        true
    }

    /// A message from the selector page; false to quit.
    fn on_ipc(&mut self, s: &str, proxy: &EventLoopProxy<UserEvent>) -> bool {
        let v: serde_json::Value = serde_json::from_str(s).unwrap_or_default();
        let state = v["state"].as_str().unwrap_or_default().to_string();
        match v["cmd"].as_str().unwrap_or_default() {
            "ready" => {
                // Mark environments whose folder can't be reached (each checked separately,
                // so one slow share doesn't hold up the rest).
                for k in envs::known() {
                    let Some(f) = k.folder else { continue };
                    let p = proxy.clone();
                    std::thread::spawn(move || {
                        let ok = std::path::Path::new(&f).is_dir();
                        let _ = p.send_event(UserEvent::Checked(k.state, ok));
                    });
                }
            }
            "open" if !state.is_empty() => return self.begin(envs::from_state_path(&state), proxy),
            "new" => {
                if let Some(h) = self.here.clone() {
                    return self.begin(envs::folder_env(&h), proxy);
                }
            }
            "browse" => {
                if let Some(f) = platform::pick_folder("Folder for the new Condor environment") {
                    return self.begin(envs::folder_env(&f.to_string_lossy()), proxy);
                }
            }
            "forget" => envs::forget(&state),
            "cancel" => {
                self.pending = None;
                self.send(&json!({"ev": "back"}));
            }
            "quit" => return false,
            _ => {}
        }
        true
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
    let args = envs::parse_args();
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut ctx = WebContext::new(Some(state::local_data_dir().join(platform::WEBVIEW_DATA)));
    let mut starter = Starter { picker: None, here: None, note: None, gen: 0, pending: None };
    match envs::choose(&args) {
        envs::Choice::Env(e) => {
            if !starter.begin(e, &proxy) {
                return; // already open in another window, now in front
            }
        }
        envs::Choice::Select { here } => {
            starter.here = here;
            starter.show(&event_loop, &mut ctx, &proxy);
        }
    }

    let mut phase = Phase::Start(starter, ctx);
    event_loop.run(move |event, target, control_flow| {
        if let Phase::Run(app) = &mut phase {
            app.on_event(event, target, control_flow);
            return;
        }
        let Phase::Start(st, ctx) = &mut phase else { return };
        let mut quit = false;
        let mut ready = None;
        match event {
            Event::UserEvent(UserEvent::Ipc(_, s)) => quit = !st.on_ipc(&s, &proxy),
            Event::UserEvent(UserEvent::Checked(state, ok)) => st.send(&json!({"ev": "check", "state": state, "ok": ok})),
            Event::UserEvent(UserEvent::Opened(g, r)) if st.pending.as_ref().map_or(false, |p| p.0 == g) => {
                st.pending = None;
                match r {
                    envs::Opened::Ready(env, state) => ready = Some((env, state)),
                    envs::Opened::Gone(env) => {
                        let f = env.folder.clone().unwrap_or_default();
                        st.note = Some(format!("The folder for \u{201c}{}\u{201d} isn't available: {f}", envs::display_name(&env)));
                        if st.picker.is_some() {
                            st.send(&json!({"ev": "back", "note": st.note}));
                        } else {
                            st.show(target, ctx, &proxy);
                        }
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => quit = true,
            _ => {}
        }
        if quit {
            *control_flow = ControlFlow::Exit;
            return;
        }
        if let Some((env, state)) = ready {
            // A file argument may have turned out to belong to an environment already open.
            if envs::activate_existing(&env) {
                *control_flow = ControlFlow::Exit;
                return;
            }
            let Phase::Start(_, ctx) = std::mem::replace(&mut phase, Phase::Switching) else { unreachable!() };
            // The selector (if any) is dropped with the start phase.
            phase = Phase::Run(App::start(target, ctx, proxy.clone(), env, state));
            *control_flow = ControlFlow::WaitUntil(Instant::now() + SCROLL_POLL);
            return;
        }
        // Still loading after a moment: show the waiting window (with a way out).
        let show_at = st.pending.as_ref().filter(|_| st.picker.is_none()).map(|p| p.2);
        match show_at {
            Some(t) if Instant::now() >= t => {
                st.show(target, ctx, &proxy);
                *control_flow = ControlFlow::Wait;
            }
            Some(t) => *control_flow = ControlFlow::WaitUntil(t),
            None => *control_flow = ControlFlow::Wait,
        }
    });
}
