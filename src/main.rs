#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Condor: a lightweight folder-tree file viewer.
//!
//! One window holds two webviews:
//!   * the UI webview (left, full window underneath): the folder tree, trusted, talks to us via IPC;
//!   * the content webview (right): shows documents, untrusted, with no IPC at all.

mod paths;
mod render;
mod state;

use serde::Deserialize;
use serde_json::json;
use std::borrow::Cow;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::http::{header, Response};
use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

#[cfg(windows)]
use wry::WebViewBuilderExtWindows;

const SPLITTER: f64 = 5.0;
const SAVE_DELAY: Duration = Duration::from_millis(800);
const SCROLL_POLL: Duration = Duration::from_millis(1500);

const UI_HTML: &str = include_str!("../ui/index.html");

#[derive(Debug)]
enum UserEvent {
    /// Message from the UI webview.
    Ipc(String),
    /// A directory listing finished (JSON message for the UI).
    Listed(String),
    /// The content webview is navigating to this URL.
    ContentNav(String),
    /// The content webview finished loading this URL.
    PageLoaded(String),
    /// Scroll position report: (url, y).
    Scroll(String, f64),
    /// Load this URL in the content webview (e.g. a target=_blank link to a local file).
    LoadContent(String),
}

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum Cmd {
    Ready,
    List { path: String },
    Open { path: String },
    Expand { path: String, open: bool },
    Sidebar { width: f64 },
    AddRoot,
    RemoveRoot { path: String },
    MoveRoot { path: String, to: usize },
    Reload,
}

struct App {
    env: String,
    st: state::EnvState,
    window: Window,
    ui: WebView,
    content: WebView,
    proxy: EventLoopProxy<UserEvent>,
    save_at: Option<Instant>,
    next_poll: Instant,
    /// File currently shown in the content view, and whether its saved scroll is still to be applied.
    current: Option<String>,
    restoring: bool,
    last_saved: Option<state::EnvState>,
}

fn log(msg: &str) {
    use std::io::Write;
    let dir = state::local_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("condor.log")) {
        let _ = writeln!(f, "{msg}");
    }
}

fn open_external(url: &str) {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// Read a directory on a worker thread and report back.
fn list_dir(path: String, proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let msg = match std::fs::read_dir(&path) {
            Ok(rd) => {
                let mut entries: Vec<(bool, String, String)> = rd
                    .filter_map(|e| e.ok())
                    .map(|e| {
                        let ft = e.file_type().ok();
                        let mut is_dir = ft.map(|t| t.is_dir()).unwrap_or(false);
                        if ft.map(|t| t.is_symlink()).unwrap_or(false) {
                            is_dir = e.path().is_dir();
                        }
                        let p = e.path().to_string_lossy().into_owned();
                        (is_dir, e.file_name().to_string_lossy().into_owned(), p)
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

/// Normalise a folder path given by the user or the command line.
fn normalise_root(p: &str) -> String {
    let p = p.trim().trim_matches('"').replace('/', "\\");
    let abs = std::path::Path::new(&p);
    let p = if abs.is_absolute() || p.starts_with("\\\\") {
        p
    } else {
        std::env::current_dir()
            .map(|c| c.join(&p).to_string_lossy().into_owned())
            .unwrap_or(p)
    };
    let p = p.strip_prefix("\\\\?\\").map(|s| s.to_string()).unwrap_or(p);
    // Keep "C:\" but drop other trailing separators.
    if p.len() > 3 && p.ends_with('\\') { p.trim_end_matches('\\').to_string() } else { p }
}

fn file_title(path: &str) -> &str {
    path.trim_end_matches('\\').rsplit('\\').next().unwrap_or(path)
}

impl App {
    fn send_ui(&self, msg: &serde_json::Value) {
        let _ = self.ui.evaluate_script(&format!("window.condor && condor.recv({msg})"));
    }

    fn mark_dirty(&mut self) {
        self.save_at = Some(Instant::now() + SAVE_DELAY);
    }

    fn save_now(&mut self) {
        self.capture_geometry();
        if self.last_saved.as_ref() != Some(&self.st) {
            if let Err(e) = state::save(&self.env, &self.st) {
                log(&format!("save failed: {e}"));
            }
            self.last_saved = Some(self.st.clone());
        }
        self.save_at = None;
    }

    fn capture_geometry(&mut self) {
        let w = &self.window;
        if w.is_minimized() {
            return;
        }
        let maximized = w.is_maximized();
        if maximized {
            // Keep the restored geometry; just record the flag.
            if let Some(g) = &mut self.st.window {
                g.maximized = true;
            }
            return;
        }
        if let Ok(pos) = w.outer_position() {
            let size = w.inner_size();
            self.st.window = Some(state::WinGeom { x: pos.x, y: pos.y, w: size.width, h: size.height, maximized });
        }
    }

    fn layout(&self) {
        let scale = self.window.scale_factor();
        let size: LogicalSize<f64> = self.window.inner_size().to_logical(scale);
        let _ = self.ui.set_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width, size.height).into(),
        });
        let x = (self.st.sidebar_width + SPLITTER).min(size.width);
        let _ = self.content.set_bounds(Rect {
            position: LogicalPosition::new(x, 0.0).into(),
            size: LogicalSize::new((size.width - x).max(0.0), size.height).into(),
        });
    }

    fn open_file(&mut self, path: &str) {
        let url = paths::path_to_url(path);
        let _ = self.content.load_url(&url);
    }

    fn init_message(&self) -> serde_json::Value {
        json!({
            "ev": "init",
            "env": self.env,
            "roots": self.st.roots,
            "expanded": self.st.expanded,
            "selected": self.st.open_file,
            "sidebar": self.st.sidebar_width,
        })
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Ready => self.send_ui(&self.init_message()),
            Cmd::List { path } => list_dir(path, self.proxy.clone()),
            Cmd::Open { path } => self.open_file(&path),
            Cmd::Expand { path, open } => {
                let changed = if open { self.st.expanded.insert(path) } else { self.st.expanded.remove(&path) };
                if changed {
                    self.mark_dirty();
                }
            }
            Cmd::Sidebar { width } => {
                if width.is_finite() {
                    self.st.sidebar_width = width.clamp(120.0, 2000.0);
                    self.layout();
                    self.mark_dirty();
                }
            }
            Cmd::AddRoot => {
                #[cfg(windows)]
                let picked = rfd::FileDialog::new().set_title("Add folder to Condor").pick_folder();
                #[cfg(not(windows))]
                let picked: Option<std::path::PathBuf> = None;
                if let Some(p) = picked {
                    self.add_root(&p.to_string_lossy());
                }
            }
            Cmd::RemoveRoot { path } => {
                self.st.roots.retain(|r| r != &path);
                // Forget saved expansion under that root, so re-adding starts fresh.
                let prefix = format!("{}\\", path.trim_end_matches('\\'));
                self.st.expanded.retain(|e| e != &path && !e.starts_with(&prefix));
                self.send_ui(&json!({"ev": "roots", "roots": self.st.roots}));
                self.mark_dirty();
            }
            Cmd::MoveRoot { path, to } => {
                if let Some(i) = self.st.roots.iter().position(|r| r == &path) {
                    let r = self.st.roots.remove(i);
                    let to = to.min(self.st.roots.len());
                    self.st.roots.insert(to, r);
                    self.send_ui(&json!({"ev": "roots", "roots": self.st.roots}));
                    self.mark_dirty();
                }
            }
            Cmd::Reload => {
                let _ = self.content.reload();
            }
        }
    }

    fn add_root(&mut self, p: &str) {
        let p = normalise_root(p);
        if !self.st.roots.iter().any(|r| r.eq_ignore_ascii_case(&p)) {
            self.st.roots.push(p.clone());
            self.st.expanded.insert(p);
            self.send_ui(&json!({"ev": "roots", "roots": self.st.roots, "expanded": self.st.expanded}));
            self.mark_dirty();
        }
    }

    fn on_content_nav(&mut self, url: &str) {
        let path = paths::url_to_path(url);
        if path != self.current {
            self.current = path.clone();
            self.restoring = self.current.as_ref().map_or(false, |p| self.st.scroll.contains_key(p));
        }
        let title = match &path {
            Some(p) => format!("{} - Condor", file_title(p)),
            None => "Condor".to_string(),
        };
        self.window.set_title(&title);
        if path.is_some() && self.st.open_file != path {
            self.st.open_file = path.clone();
            self.mark_dirty();
        }
        self.send_ui(&json!({"ev": "selected", "path": path}));
    }

    fn on_page_loaded(&mut self, url: &str) {
        let path = paths::url_to_path(url);
        if path.is_none() || path != self.current {
            return;
        }
        let p = path.unwrap();
        if let Some(y) = self.st.scroll.get(&p).copied() {
            if y > 0.0 {
                // Retry for a while: images may still be loading and changing the page height.
                let js = format!(
                    "(function(y){{var n=0;function f(){{window.scrollTo(0,y);\
                     if(Math.abs(window.scrollY-y)>2&&n++<30)setTimeout(f,100)}}f()}})({y})"
                );
                let _ = self.content.evaluate_script(&js);
            }
        }
        self.restoring = false;
        // Give the restore a moment before polling, so we don't record a pre-restore 0.
        self.next_poll = Instant::now() + Duration::from_millis(3500);
    }

    fn poll_scroll(&mut self) {
        self.next_poll = Instant::now() + SCROLL_POLL;
        if self.restoring || self.current.is_none() {
            return;
        }
        let proxy = self.proxy.clone();
        let _ = self.content.evaluate_script_with_callback(
            "[location.href, window.scrollY]",
            move |res| {
                if let Ok(serde_json::Value::Array(v)) = serde_json::from_str::<serde_json::Value>(&res) {
                    if let (Some(u), Some(y)) = (v.first().and_then(|x| x.as_str()), v.get(1).and_then(|x| x.as_f64())) {
                        let _ = proxy.send_event(UserEvent::Scroll(u.to_string(), y));
                    }
                }
            },
        );
    }

    fn on_scroll(&mut self, url: &str, y: f64) {
        let Some(p) = paths::url_to_path(url) else { return };
        if Some(&p) != self.current.as_ref() || self.restoring || !y.is_finite() {
            return;
        }
        let y = y.max(0.0).round();
        let old = self.st.scroll.get(&p).copied();
        if old != Some(y) && !(old.is_none() && y == 0.0) {
            self.st.set_scroll(&p, y);
            self.mark_dirty();
        }
    }
}

fn parse_args() -> (String, Option<String>) {
    let mut env = "default".to_string();
    let mut path = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--env" || a == "-e" {
            if let Some(v) = args.next() {
                env = v;
            }
        } else if let Some(v) = a.strip_prefix("--env=") {
            env = v.to_string();
        } else {
            path = Some(a);
        }
    }
    (env, path)
}

fn main() {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
    let (env, start_path) = parse_args();
    let mut st = state::load(&env);
    if let Some(p) = start_path {
        let p = normalise_root(&p);
        if !st.roots.iter().any(|r| r.eq_ignore_ascii_case(&p)) {
            st.roots.push(p.clone());
            st.expanded.insert(p);
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let mut wb = WindowBuilder::new().with_title("Condor").with_inner_size(LogicalSize::new(1200.0, 800.0));
    if let Some(g) = st.window {
        // Only restore the position if it is still on a connected monitor.
        let on_screen = event_loop.available_monitors().any(|m| {
            let (mp, ms) = (m.position(), m.size());
            g.x + 50 < mp.x + ms.width as i32 && g.x + g.w as i32 > mp.x + 50 && g.y >= mp.y - 20 && g.y < mp.y + ms.height as i32 - 50
        });
        wb = wb.with_inner_size(PhysicalSize::new(g.w.max(300), g.h.max(200)));
        if on_screen {
            wb = wb.with_position(PhysicalPosition::new(g.x, g.y));
        }
        wb = wb.with_maximized(g.maximized);
    }
    let window = wb.build(&event_loop).expect("create window");

    let mut ctx = WebContext::new(Some(state::local_data_dir().join("WebView2")));

    // UI webview: our own trusted page, the only one with IPC.
    let p_ipc = proxy.clone();
    let ui_origin = format!("http://{}.localhost/", paths::UI_SCHEME);
    let ui_builder = WebViewBuilder::new_with_web_context(&mut ctx);
    #[cfg(windows)]
    let ui_builder = ui_builder.with_browser_accelerator_keys(false);
    let ui_builder = ui_builder
        .with_url(format!("{}://localhost/", paths::UI_SCHEME))
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
                open_external(&url);
                false
            }
        })
        .with_ipc_handler(move |req| {
            let _ = p_ipc.send_event(UserEvent::Ipc(req.into_body()));
        })
        .with_devtools(std::env::var_os("CONDOR_DEVTOOLS").is_some());
    // Built first so the content view sits on top of it.
    let ui = ui_builder.build_as_child(&window).expect("create UI webview");


    // Content webview: untrusted documents, no IPC.
    let p_nav = proxy.clone();
    let p_load = proxy.clone();
    let p_new = proxy.clone();
    let file_origin = paths::file_origin();
    let fo2 = file_origin.clone();
    let initial = st.open_file.as_deref().map(paths::path_to_url).unwrap_or_else(|| format!("{file_origin}/"));
    let content_builder = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(format!("{}://localhost{}", paths::FILE_SCHEME, initial.strip_prefix(&file_origin).unwrap_or("/")))
        .with_asynchronous_custom_protocol(paths::FILE_SCHEME.into(), |_id, req, responder| {
            let uri = req.uri().clone();
            std::thread::spawn(move || {
                responder.respond(render::serve(uri.path(), uri.query()));
            });
        })
        .with_navigation_handler(move |url| {
            if url.starts_with(&file_origin) {
                let _ = p_nav.send_event(UserEvent::ContentNav(url));
                true
            } else if url.starts_with("about:") || url.starts_with("data:") || url.starts_with("blob:") {
                true
            } else {
                open_external(&url);
                false
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            if url.starts_with(&fo2) {
                let _ = p_new.send_event(UserEvent::LoadContent(url));
            } else {
                open_external(&url);
            }
            wry::NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |ev, url| {
            if let PageLoadEvent::Finished = ev {
                let _ = p_load.send_event(UserEvent::PageLoaded(url));
            }
        })
        .with_devtools(std::env::var_os("CONDOR_DEVTOOLS").is_some());
    let content = content_builder.build_as_child(&window).expect("create content webview");


    let mut app = App {
        env,
        current: None,
        restoring: false,
        last_saved: None,
        save_at: None,
        next_poll: Instant::now() + SCROLL_POLL,
        st,
        window,
        ui,
        content,
        proxy,
    };
    app.layout();
    app.save_now();
    let _ctx = ctx; // keep the web context alive for the life of the app

    event_loop.run(move |event, _target, control_flow| {
        let _ = &_ctx;
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {}
            Event::UserEvent(ev) => match ev {
                UserEvent::Ipc(s) => match serde_json::from_str::<Cmd>(&s) {
                    Ok(cmd) => app.handle_cmd(cmd),
                    Err(e) => log(&format!("bad ipc {s}: {e}")),
                },
                UserEvent::Listed(msg) => {
                    let _ = app.ui.evaluate_script(&format!("window.condor && condor.recv({msg})"));
                }
                UserEvent::ContentNav(url) => app.on_content_nav(&url),
                UserEvent::PageLoaded(url) => app.on_page_loaded(&url),
                UserEvent::Scroll(url, y) => app.on_scroll(&url, y),
                UserEvent::LoadContent(url) => {
                    let _ = app.content.load_url(&url);
                }
            },
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    app.poll_scroll();
                    app.save_now();
                    *control_flow = ControlFlow::Exit;
                    return;
                }
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    app.layout();
                    app.mark_dirty();
                }
                WindowEvent::Moved(_) => app.mark_dirty(),
                _ => {}
            },
            Event::LoopDestroyed => app.save_now(),
            _ => {}
        }

        let now = Instant::now();
        if now >= app.next_poll {
            app.poll_scroll();
        }
        if app.save_at.map_or(false, |t| now >= t) {
            app.save_now();
        }
        let mut wake = app.next_poll;
        if let Some(t) = app.save_at {
            wake = wake.min(t);
        }
        *control_flow = ControlFlow::WaitUntil(wake);
    });
}
