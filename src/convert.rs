//! Office and similar documents converted to PDF with LibreOffice, when it is installed.
//!
//! Conversions run headless with a profile of Condor's own (so an open LibreOffice isn't
//! disturbed), one at a time, and are cached by path, size and modification time.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// File types LibreOffice can turn into a PDF worth reading.
pub const EXTS: &[&str] = &[
    "doc", "docx", "docm", "dot", "dotx", "odt", "rtf", "wpd", "xls", "xlsx", "xlsm", "xlsb", "xlt", "xltx", "ods", "csv",
    "ppt", "pptx", "pptm", "pps", "ppsx", "odp", "vsd", "vsdx", "vsdm", "vdx", "odg", "pub",
];
const TIMEOUT: Duration = Duration::from_secs(180);
const CACHE_FILES: usize = 300;

pub fn convertible(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    EXTS.contains(&ext.as_str())
}

/// LibreOffice's soffice, if installed.
pub fn soffice() -> Option<&'static Path> {
    static S: OnceLock<Option<PathBuf>> = OnceLock::new();
    S.get_or_init(|| {
        let mut c: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            for v in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                if let Some(d) = std::env::var_os(v) {
                    c.push(Path::new(&d).join("LibreOffice\\program\\soffice.exe"));
                }
            }
        } else if cfg!(target_os = "macos") {
            c.push("/Applications/LibreOffice.app/Contents/MacOS/soffice".into());
        }
        if let Some(path) = std::env::var_os("PATH") {
            for d in std::env::split_paths(&path) {
                for n in ["soffice", "libreoffice", "soffice.exe"] {
                    c.push(d.join(n));
                }
            }
        }
        c.into_iter().find(|p| p.is_file())
    })
    .as_deref()
}

pub fn available() -> bool {
    soffice().is_some()
}

fn cache_dir() -> PathBuf {
    crate::platform::local_data_dir().join("converted")
}

/// Where the PDF of this version of `path` is (or would be) cached.
fn cache_path(path: &str) -> Option<PathBuf> {
    let m = std::fs::metadata(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    m.len().hash(&mut h);
    m.modified().ok().hash(&mut h);
    Some(cache_dir().join(format!("{:016x}.pdf", h.finish())))
}

/// The PDF for `path`, converting it first if needed (can take seconds; call off the UI thread).
pub fn to_pdf(path: &str) -> Result<Vec<u8>, String> {
    let out = cache_path(path).ok_or_else(|| format!("{path}: can't read it"))?;
    if let Ok(b) = std::fs::read(&out) {
        return Ok(b);
    }
    let soffice = soffice().ok_or("LibreOffice isn't installed")?;
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(b) = std::fs::read(&out) {
        return Ok(b); // converted while we waited
    }
    let base = crate::platform::local_data_dir();
    let work = base.join("convert-work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let profile = format!("file:///{}", base.join("libreoffice-profile").to_string_lossy().replace('\\', "/").trim_start_matches('/'));
    let mut cmd = std::process::Command::new(soffice);
    cmd.arg("--headless")
        .arg("--norestore")
        .arg("--nologo")
        .arg(format!("-env:UserInstallation={profile}"))
        .args(["--convert-to", "pdf", "--outdir"])
        .arg(&work)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start LibreOffice: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                return Err("LibreOffice took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let pdf = std::fs::read_dir(&work)
        .ok()
        .and_then(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "pdf")))
        .ok_or("LibreOffice couldn't convert it")?;
    let _ = std::fs::create_dir_all(cache_dir());
    std::fs::rename(&pdf, &out).or_else(|_| std::fs::copy(&pdf, &out).map(|_| ())).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&work);
    prune();
    std::fs::read(&out).map_err(|e| e.to_string())
}

/// Keep the cache to the most recent conversions.
fn prune() {
    let Ok(rd) = std::fs::read_dir(cache_dir()) else { return };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= CACHE_FILES {
        return;
    }
    files.sort();
    for (_, p) in &files[..files.len() - CACHE_FILES] {
        let _ = std::fs::remove_file(p);
    }
}
