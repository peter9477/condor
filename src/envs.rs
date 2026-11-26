//! Which saved state ("environment") a Condor window uses, and keeping track of them.
//!
//! * `condor FOLDER` (e.g. `condor .`) keeps its state in `FOLDER\.condor\state.json`,
//!   creating it if needed, with a `.gitignore` of `*` beside it so git ignores the folder.
//!   `condor FILE` opens the file in the nearest folder above it that has an environment.
//! * With no arguments, the current folder's environment opens if it has one; otherwise
//!   (including from the Start menu) the selector lists the known environments, most
//!   recently used first (`recent.json` in the config folder), and offers a new one here.
//! * `--env NAME` uses a named environment in the config folder; `--state FILE` any file.
//! * One window per environment: starting one that is already open brings that window to
//!   the front instead (each running window listens on a local port, recorded under the
//!   local data folder).
//!
//! Test instances (CONDOR_SETTINGS set) don't change the recent list.

use crate::platform;
use crate::state::{self, EnvState};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where an environment's state lives and how to show its name.
#[derive(Clone, Debug)]
pub struct Env {
    pub state_file: PathBuf,
    /// Shown in the window title ("" for the default environment).
    pub label: String,
    /// The folder a folder environment belongs to.
    pub folder: Option<String>,
    /// Open this file (from the command line), and add this root, once loaded.
    pub open_file: Option<String>,
    pub add_root: Option<String>,
}

pub enum Choice {
    Env(Env),
    /// Show the selector; `here` is a folder a new environment could be made in.
    Select { here: Option<String> },
}

/// What the command line asks for.
pub struct Args {
    pub env: Option<String>,
    pub state: Option<String>,
    pub folder: Option<String>,
}

pub fn parse_args() -> Args {
    let mut a = Args { env: None, state: None, folder: None };
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--env" | "-e" => a.env = it.next(),
            "--state" => a.state = it.next(),
            _ if x.starts_with("--env=") => a.env = Some(x["--env=".len()..].to_string()),
            _ if x.starts_with("--state=") => a.state = Some(x["--state=".len()..].to_string()),
            _ => a.folder = Some(x),
        }
    }
    a
}

pub fn named(name: &str) -> Env {
    Env {
        state_file: state::env_file(name),
        label: if name == "default" { String::new() } else { name.to_string() },
        folder: None,
        open_file: None,
        add_root: None,
    }
}

pub fn folder_env(folder: &str) -> Env {
    let folder = platform::normalise_path(folder);
    Env {
        state_file: Path::new(&folder).join(".condor").join("state.json"),
        label: platform::file_name(&folder).to_string(),
        folder: Some(folder),
        open_file: None,
        add_root: None,
    }
}

fn file_env(file: &str) -> Env {
    let p = PathBuf::from(platform::normalise_path(file));
    // A folder's .condor\state.json is that folder's environment.
    let folder = p.parent().filter(|d| d.file_name().map_or(false, |n| n == ".condor")).and_then(|d| d.parent());
    match folder {
        Some(f) => folder_env(&f.to_string_lossy()),
        None => {
            // A named environment (in the config folder) keeps its name; "default" shows none.
            let named = p.parent().map_or(false, |d| platform::path_key(&d.to_string_lossy()) == platform::path_key(&state::config_dir().join("environments").to_string_lossy()));
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let label = if named && stem == "default" { String::new() } else { stem };
            Env { label, state_file: p, folder: None, open_file: None, add_root: None }
        }
    }
}

pub fn from_state_path(file: &str) -> Env {
    file_env(file)
}

/// The name to list an environment under.
pub fn display_name(env: &Env) -> String {
    if env.label.is_empty() { "default".into() } else { env.label.clone() }
}

fn has_env(dir: &Path) -> bool {
    dir.join(".condor").join("state.json").is_file()
}

/// Folders Condor is started in that aren't a project: the Start menu and shortcuts start
/// programs in the home folder, System32 or the program's own folder.
fn incidental(dir: &str) -> bool {
    let d = platform::path_key(dir);
    let mut not: Vec<String> = ["USERPROFILE", "HOME", "SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|p| platform::path_key(&p))
        .collect();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_string_lossy().into_owned())) {
        not.push(platform::path_key(&exe_dir));
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).map(|p| platform::path_key(&p)).ok();
    not.iter().any(|n| {
        // System folders count with everything under them; the home folder only itself.
        d == *n || (Some(n) != home.as_ref() && platform::is_under(&d, n))
    }) || Path::new(dir).parent().is_none()
}

/// Which environment to open. Touches only the named path and its parents.
pub fn choose(a: &Args) -> Choice {
    let explicit = a.state.as_deref().map(file_env).or_else(|| a.env.as_deref().map(named));
    if let Some(mut e) = explicit {
        // `--env NAME FOLDER`: the folder becomes a root of that environment.
        e.add_root = a.folder.as_deref().map(platform::normalise_path);
        return Choice::Env(e);
    }
    if let Some(f) = &a.folder {
        // Whether it is a file is found out by `open`, off the UI thread (it may be slow).
        return Choice::Env(folder_env(f));
    }
    let cwd = std::env::current_dir().ok().map(|c| c.to_string_lossy().into_owned());
    if let Some(c) = &cwd {
        if has_env(Path::new(c)) {
            return Choice::Env(folder_env(c));
        }
    }
    Choice::Select { here: cwd.filter(|c| !incidental(c)) }
}

/// An environment the selector lists.
#[derive(Serialize, Clone)]
pub struct Known {
    pub state: String,
    pub name: String,
    /// Where it is: its folder, or the state file for other kinds.
    pub place: String,
    /// The folder to check is reachable (folder environments).
    #[serde(skip)]
    pub folder: Option<String>,
}

/// Known environments, most recently used first, then named ones never recorded as used.
pub fn known() -> Vec<Known> {
    let mut out: Vec<Known> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |e: Env, out: &mut Vec<Known>| {
        if seen.insert(platform::path_key(&e.state_file.to_string_lossy())) {
            out.push(Known {
                state: e.state_file.to_string_lossy().into_owned(),
                name: display_name(&e),
                place: e.folder.clone().unwrap_or_else(|| e.state_file.to_string_lossy().into_owned()),
                folder: e.folder.clone(),
            });
        }
    };
    for r in load_recent().envs {
        add(file_env(&r.state), &mut out);
    }
    let mut named: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(state::config_dir().join("environments"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    named.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in named {
        add(file_env(&p.to_string_lossy()), &mut out);
    }
    out
}

/// Drop an environment from the recent list (its state file is left alone).
pub fn forget(state: &str) {
    let mut r = load_recent();
    r.envs.retain(|e| platform::path_key(&e.state) != platform::path_key(state));
    let _ = write_atomic(&recent_file(), &serde_json::to_vec_pretty(&r).unwrap_or_default());
}

#[derive(Debug)]
pub enum Opened {
    Ready(Env, EnvState),
    /// The environment's folder doesn't exist (any more).
    Gone(Env),
}

/// Load an environment's state; may be slow (network folders), so off the UI thread.
pub fn open(mut env: Env) -> Opened {
    if let Some(f) = env.folder.clone() {
        let p = Path::new(&f);
        if p.is_file() {
            // A file: the nearest enclosing environment, else its own folder's.
            let dir = p.ancestors().skip(1).find(|d| has_env(d)).or_else(|| p.parent());
            env = folder_env(&dir.map_or(f.clone(), |d| d.to_string_lossy().into_owned()));
            env.open_file = Some(f);
        } else if !p.is_dir() {
            return Opened::Gone(env);
        }
    }
    let mut st = match std::fs::read(&env.state_file) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
        Err(_) => {
            // New: a folder environment starts with its folder as the root.
            let mut st = EnvState::default();
            if let Some(f) = &env.folder {
                st.roots.push(f.clone());
                st.expanded.insert(f.clone());
            }
            st
        }
    };
    if env.folder.is_some() {
        prepare_dir(&env.state_file);
    }
    if let Some(r) = &env.add_root {
        if !st.roots.iter().any(|x| platform::path_key(x) == platform::path_key(r)) {
            st.roots.push(r.clone());
            st.expanded.insert(r.clone());
        }
    }
    if let Some(f) = &env.open_file {
        st.open_file = Some(f.clone());
    }
    touch_recent(&env);
    Opened::Ready(env, st)
}

/// Create `.condor\` with a .gitignore that ignores the folder itself.
fn prepare_dir(state_file: &Path) {
    if let Some(dir) = state_file.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            let gi = dir.join(".gitignore");
            if !gi.exists() {
                let _ = std::fs::write(gi, "# Condor's own state; not part of the project.\n*\n");
            }
        }
    }
}

// ----- saving off the UI thread -------------------------------------------------------

enum Job {
    Write(PathBuf, Vec<u8>),
    Flush(std::sync::mpsc::Sender<()>),
}

/// Writes state files on a worker thread, so a slow folder never stalls the window.
pub struct Saver(std::sync::mpsc::Sender<Job>);

impl Saver {
    pub fn start() -> Saver {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Write(path, bytes) => {
                        // Only the latest of several queued writes to a file matters.
                        if let Err(e) = write_atomic(&path, &bytes) {
                            crate::log(&format!("saving {}: {e}", path.display()));
                        }
                    }
                    Job::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Saver(tx)
    }

    pub fn save(&self, path: &Path, st: &EnvState) {
        let bytes = serde_json::to_vec_pretty(st).unwrap_or_default();
        let _ = self.0.send(Job::Write(path.to_path_buf(), bytes));
    }

    /// Wait (a while) for queued writes, before exiting.
    pub fn flush(&self, wait: Duration) {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.0.send(Job::Flush(tx)).is_ok() {
            let _ = rx.recv_timeout(wait);
        }
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

// ----- the recent list ------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recent {
    /// Most recently used first.
    pub envs: Vec<RecentEnv>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct RecentEnv {
    pub state: String,
    pub label: String,
}

const RECENT_MAX: usize = 30;

fn recent_file() -> PathBuf {
    state::config_dir().join("recent.json")
}

pub fn load_recent() -> Recent {
    std::fs::read(recent_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Test instances leave the list alone.
pub fn is_test_instance() -> bool {
    std::env::var_os("CONDOR_SETTINGS").map_or(false, |v| !v.is_empty())
}

/// Put `env` first in the recent list (when it opens and when it closes).
pub fn touch_recent(env: &Env) {
    if is_test_instance() {
        return;
    }
    let mut r = load_recent();
    let key = env.state_file.to_string_lossy().into_owned();
    r.envs.retain(|e| platform::path_key(&e.state) != platform::path_key(&key));
    r.envs.insert(0, RecentEnv { state: key, label: env.label.clone() });
    r.envs.truncate(RECENT_MAX);
    let _ = write_atomic(&recent_file(), &serde_json::to_vec_pretty(&r).unwrap_or_default());
}

// ----- one window per environment -------------------------------------------------------

fn running_file(env: &Env) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    platform::path_key(&env.state_file.to_string_lossy()).hash(&mut h);
    state::local_data_dir().join("running").join(format!("{:016x}.json", h.finish()))
}

#[derive(Serialize, Deserialize)]
struct Running {
    pid: u32,
    port: u16,
}

/// If `env` is open in another Condor, ask it to come to the front; true if it did.
pub fn activate_existing(env: &Env) -> bool {
    let Ok(b) = std::fs::read(running_file(env)) else { return false };
    let Ok(r) = serde_json::from_slice::<Running>(&b) else { return false };
    if r.pid == std::process::id() {
        return false;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], r.port));
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    platform::allow_foreground(r.pid);
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(b"activate\n").is_err() {
        return false;
    }
    let mut line = String::new();
    std::io::BufReader::new(s).read_line(&mut line).is_ok() && line.trim() == "ok"
}

/// Listen for other Condors starting this environment; `on_activate` runs for each.
pub fn listen(env: &Env, on_activate: impl Fn() + Send + 'static) {
    let Ok(l) = std::net::TcpListener::bind(("127.0.0.1", 0)) else { return };
    let Ok(port) = l.local_addr().map(|a| a.port()) else { return };
    let rec = Running { pid: std::process::id(), port };
    let _ = write_atomic(&running_file(env), &serde_json::to_vec(&rec).unwrap_or_default());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            let mut r = std::io::BufReader::new(&s);
            if r.read_line(&mut line).is_ok() && line.trim() == "activate" {
                on_activate();
                let _ = (&s).write_all(b"ok\n");
            }
        }
    });
}

/// Stop answering for `env` (on exit, or before a restart starts the new build).
pub fn unregister(env: &Env) {
    let f = running_file(env);
    let mine = std::fs::read(&f)
        .ok()
        .and_then(|b| serde_json::from_slice::<Running>(&b).ok())
        .map_or(false, |r| r.pid == std::process::id());
    if mine {
        let _ = std::fs::remove_file(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_state_file_round_trip() {
        let e = file_env(&format!("{}/proj/.condor/state.json", if cfg!(windows) { "C:" } else { "" }));
        assert!(e.folder.as_deref().map_or(false, |f| f.ends_with("proj")));
        assert_eq!(e.label, "proj");
    }
}
