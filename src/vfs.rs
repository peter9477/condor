//! Zip files as folders: `C:\x\docs.zip\folder\file.md` names a file inside docs.zip.
//!
//! Listing reads the archive's index (cached by the zip's size and modification time).
//! A file inside is extracted on first use to a local cache that mirrors the archive's
//! layout (`<local data>\zip\<hash>\folder\file.md`), so everything that needs a real
//! file (rendering, PDF conversion, previewers, opening in another program, copying)
//! uses that copy. Nothing is ever written beside the zip.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SEP: char = std::path::MAIN_SEPARATOR;
/// Larger entries aren't extracted (Condor is a viewer).
const MAX_ENTRY: u64 = 512 << 20;

pub fn is_zip_name(name: &str) -> bool {
    name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".zip")
}

/// For a path inside a zip: (the zip file, the path inside it with '/' separators, which
/// is "" for the zip itself). None for ordinary paths. Only touches the disk for path
/// components named *.zip.
pub fn split(path: &str) -> Option<(String, String)> {
    let mut end = 0;
    for comp in path.split(SEP) {
        end += comp.len();
        if is_zip_name(comp) && Path::new(&path[..end]).is_file() {
            let inner = path[end..].trim_start_matches(SEP).replace(SEP, "/");
            return Some((path[..end].to_string(), inner.trim_end_matches('/').to_string()));
        }
        end += 1; // the separator
    }
    None
}

/// The zip file a path is inside of, if any (for watching it for changes).
pub fn container(path: &str) -> Option<String> {
    split(path).filter(|(_, inner)| !inner.is_empty()).map(|(z, _)| z)
}

#[derive(Clone, PartialEq)]
struct Sig(u64, Option<std::time::SystemTime>);

fn sig(zip: &str) -> Option<Sig> {
    let m = std::fs::metadata(zip).ok()?;
    Some(Sig(m.len(), m.modified().ok()))
}

/// Entry names (files, '/'-separated) of each zip, while it is unchanged.
static INDEX: Mutex<Option<HashMap<String, (Sig, Vec<(String, u64)>)>>> = Mutex::new(None);

fn index(zip: &str) -> std::io::Result<Vec<(String, u64)>> {
    let s = sig(zip).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "zip file not found"))?;
    if let Some((old, names)) = INDEX.lock().unwrap().get_or_insert_with(HashMap::new).get(zip) {
        if *old == s {
            return Ok(names.clone());
        }
    }
    let f = std::fs::File::open(zip)?;
    let mut a = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(std::io::Error::other)?;
    let mut names = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        if let Ok(e) = a.by_index_raw(i) {
            let n = e.name().replace('\\', "/");
            // Ignore unsafe names (absolute, or climbing out with ..).
            if n.starts_with('/') || n.split('/').any(|c| c == "..") {
                continue;
            }
            names.push((n, e.size()));
        }
    }
    let mut g = INDEX.lock().unwrap();
    let map = g.get_or_insert_with(HashMap::new);
    if map.len() > 50 {
        map.clear();
    }
    map.insert(zip.to_string(), (s, names.clone()));
    Ok(names)
}

/// What a zip folder (`inner` "" for the top) contains: (is folder, name).
pub fn list(zip: &str, inner: &str) -> std::io::Result<Vec<(bool, String)>> {
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    let mut seen = std::collections::BTreeMap::new();
    for (n, _) in index(zip)? {
        let Some(rest) = n.strip_prefix(&prefix) else { continue };
        let (first, more) = match rest.split_once('/') {
            Some((f, m)) => (f, !m.is_empty() || n.ends_with('/')),
            None => (rest, false),
        };
        if first.is_empty() {
            continue;
        }
        let dir = more || n.ends_with('/');
        let e = seen.entry(first.to_string()).or_insert(dir);
        *e |= dir;
    }
    if seen.is_empty() && !inner.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such folder in the zip file"));
    }
    Ok(seen.into_iter().map(|(n, d)| (d, n)).collect())
}

/// Whether a path is a folder: an ordinary one, a zip file, or a folder inside a zip.
pub fn is_dir(path: &str) -> bool {
    match split(path) {
        Some((_, inner)) if inner.is_empty() => true,
        Some((zip, inner)) => index(&zip).map_or(false, |ns| {
            let p = format!("{inner}/");
            ns.iter().any(|(n, _)| n.starts_with(&p))
        }),
        None => Path::new(path).is_dir(),
    }
}

fn cache_dir(zip: &str, s: &Sig) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::platform::path_key(zip).hash(&mut h);
    s.0.hash(&mut h);
    s.1.hash(&mut h);
    crate::platform::local_data_dir().join("zip").join(format!("{:016x}", h.finish()))
}

/// A real file for `path`: itself, or for a file inside a zip, its extracted copy (made
/// now if needed; may take a while for a big entry or a slow share).
pub fn local(path: &str) -> std::io::Result<PathBuf> {
    let Some((zip, inner)) = split(path) else { return Ok(PathBuf::from(path)) };
    if inner.is_empty() {
        return Ok(PathBuf::from(zip));
    }
    let s = sig(&zip).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "zip file not found"))?;
    let names = index(&zip)?;
    let Some((name, size)) = names.iter().find(|(n, _)| *n == inner) else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not in the zip file"));
    };
    if *size > MAX_ENTRY {
        return Err(std::io::Error::other("too large to extract"));
    }
    let dir = cache_dir(&zip, &s);
    let out = inner.split('/').fold(dir.clone(), |p, c| p.join(c));
    if out.is_file() {
        return Ok(out);
    }
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if out.is_file() {
        return Ok(out);
    }
    prune(&dir);
    let f = std::fs::File::open(&zip)?;
    let mut a = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(std::io::Error::other)?;
    let mut e = a.by_name(name).map_err(std::io::Error::other)?;
    let mut buf = Vec::with_capacity(*size as usize);
    e.by_ref().take(MAX_ENTRY).read_to_end(&mut buf)?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = out.with_extension("condor-part");
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, &out)?;
    Ok(out)
}

/// Keep the extraction cache small: drop other versions' folders beyond the newest 20.
fn prune(keep: &Path) {
    let Some(root) = keep.parent() else { return };
    let Ok(rd) = std::fs::read_dir(root) else { return };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
        .filter(|(_, p)| p != keep)
        .collect();
    if dirs.len() <= 20 {
        return;
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, d) in dirs.into_iter().skip(20) {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_zip_name("a.ZIP"));
        assert!(!is_zip_name(".zip"));
        assert!(!is_zip_name("zip"));
    }
}
