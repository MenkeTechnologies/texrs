//! File location through `kpsewhich`, shared by every caller.
//!
//! A `kpsewhich` process costs 0.3 to 1 s of wall time and a document asks for
//! tens of files (fonts, hyphenation patterns, the glyph list, packages), each
//! of which used to be its own blocking spawn. Three mechanisms remove that:
//!
//! * **Memo.** A name is resolved once per process; concurrent askers for the
//!   same name wait for the one resolution instead of spawning their own.
//! * **Batch.** [`warm`] resolves many names with a single process.
//! * **Disk cache.** Resolved paths are appended to
//!   `<cache dir>/texrs/kpse-<env hash>.tsv`. A later run takes a recorded path
//!   only if the file still exists; misses are never recorded, and the file name
//!   carries a hash of the variables that steer the search, so changing
//!   `TEXINPUTS` and friends selects a different file.
//!
//! The answer for a name is the same whichever mechanism produced it, so output
//! does not depend on timing or thread count.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};

/// Files every typeset run reads: the Computer Modern metrics and outlines the
/// math fonts use, the hyphenation patterns and the glyph list.
pub const STARTUP_FILES: &[&str] = &[
    "cmr10.tfm",
    "cmr7.tfm",
    "cmr5.tfm",
    "cmmi10.tfm",
    "cmmi7.tfm",
    "cmmi5.tfm",
    "cmsy10.tfm",
    "cmsy7.tfm",
    "cmsy5.tfm",
    "cmex10.tfm",
    "cmr10.pfb",
    "cmr10.afm",
    "hyphen.tex",
    "glyphlist.txt",
];

enum Slot {
    /// Some thread is running `kpsewhich` for this name.
    Pending,
    Done(Option<PathBuf>),
}

struct State {
    slots: Mutex<HashMap<String, Slot>>,
    resolved: Condvar,
    /// Paths recorded by earlier runs, consulted before spawning anything.
    disk: BTreeMap<String, PathBuf>,
    disk_file: Option<PathBuf>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let disk_file = disk_cache_path();
        let disk = disk_file.as_deref().map(read_disk).unwrap_or_default();
        State {
            slots: Mutex::new(HashMap::new()),
            resolved: Condvar::new(),
            disk,
            disk_file,
        }
    })
}

/// Where TeX keeps `name`, e.g. `cmr10.tfm` or `hyphen.tex`.
pub fn locate(name: &str) -> Option<PathBuf> {
    let st = state();
    let mut slots = st.slots.lock().ok()?;
    loop {
        match slots.get(name) {
            Some(Slot::Done(found)) => return found.clone(),
            Some(Slot::Pending) => slots = st.resolved.wait(slots).ok()?,
            None => break,
        }
    }
    if let Some(hit) = disk_hit(st, name) {
        let hit = hit.clone();
        slots.insert(name.to_string(), Slot::Done(Some(hit.clone())));
        return Some(hit);
    }
    slots.insert(name.to_string(), Slot::Pending);
    drop(slots);

    let found = resolve_many(&[name.to_string()])
        .into_iter()
        .next()
        .flatten();
    finish(st, &[name.to_string()], std::slice::from_ref(&found));
    found
}

/// Resolve every name not yet known with one `kpsewhich` process.
pub fn warm(names: &[String]) {
    let st = state();
    let mut todo: Vec<String> = Vec::new();
    {
        let Ok(mut slots) = st.slots.lock() else {
            return;
        };
        for n in names {
            if slots.contains_key(n) || todo.contains(n) {
                continue;
            }
            match disk_hit(st, n) {
                Some(p) => {
                    slots.insert(n.clone(), Slot::Done(Some(p.clone())));
                }
                None => {
                    slots.insert(n.clone(), Slot::Pending);
                    todo.push(n.clone());
                }
            }
        }
    }
    if todo.is_empty() {
        return;
    }
    let found = resolve_many(&todo);
    finish(st, &todo, &found);
}

/// Start resolving [`STARTUP_FILES`] on a background thread.
///
/// Returns at once. A caller that needs one of the files before the thread has
/// resolved it waits on that name (see [`locate`]); it never spawns a second
/// process for it.
pub fn prefetch_startup() {
    std::thread::spawn(|| {
        let names: Vec<String> = STARTUP_FILES.iter().map(|s| s.to_string()).collect();
        warm(&names);
    });
}

/// A path recorded by an earlier run, if it is still there and no file of the
/// same name in the working directory (which `kpsewhich` searches first) would
/// now shadow it.
fn disk_hit(st: &State, name: &str) -> Option<PathBuf> {
    let p = st.disk.get(name)?;
    (p.is_file() && !Path::new(name).exists()).then(|| p.clone())
}

fn finish(st: &State, names: &[String], found: &[Option<PathBuf>]) {
    if let Ok(mut slots) = st.slots.lock() {
        for (n, f) in names.iter().zip(found) {
            slots.insert(n.clone(), Slot::Done(f.clone()));
        }
    }
    st.resolved.notify_all();
    if let Some(file) = &st.disk_file {
        append_disk(file, names, found);
    }
}

/// One `kpsewhich` process for all `names`.
///
/// `kpsewhich a b` prints one line per file it found and nothing for one it did
/// not, so answers are matched to questions by the file name at the end of each
/// path rather than by position.
pub fn resolve_many(names: &[String]) -> Vec<Option<PathBuf>> {
    let Ok(out) = std::process::Command::new("kpsewhich").args(names).output() else {
        return vec![None; names.len()];
    };
    // Non-zero when ANY name is missing, so the lines are read, not the status.
    let text = String::from_utf8_lossy(&out.stdout);
    let mut by_name: BTreeMap<&str, PathBuf> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let path = PathBuf::from(line);
        if let Some(file) = line.rsplit('/').next() {
            if path.is_file() {
                by_name.entry(file).or_insert(path);
            }
        }
    }
    names
        .iter()
        .map(|n| by_name.get(n.as_str()).cloned())
        .collect()
}

/// Names whose search path is not the default one are not looked up here; the
/// hash covers the variables `kpsewhich` reads for the default path.
fn disk_cache_path() -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    if !crate::script_cache::cache_enabled() {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for var in [
        "PATH",
        "TEXINPUTS",
        "TFMFONTS",
        "TEXFONTS",
        "T1FONTS",
        "AFMFONTS",
        "TEXMFHOME",
        "TEXMFLOCAL",
        "TEXMFDIST",
        "TEXMF",
        "TEXMFCNF",
        "TEXMFVAR",
        "TEXMFCONFIG",
    ] {
        var.hash(&mut h);
        std::env::var_os(var).hash(&mut h);
    }
    let dir = crate::script_cache::default_cache_path();
    Some(dir.parent()?.join(format!("kpse-{:016x}.tsv", h.finish())))
}

/// A file found through the working directory is not a property of the TeX tree.
fn under_cwd(p: &Path) -> bool {
    std::env::current_dir().is_ok_and(|d| p.starts_with(d))
}

fn read_disk(file: &Path) -> BTreeMap<String, PathBuf> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    // Later lines win: a stale path re-resolved is appended, not rewritten.
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(n, p)| (n.to_string(), PathBuf::from(p)))
        .collect()
}

fn append_disk(file: &Path, names: &[String], found: &[Option<PathBuf>]) {
    use std::io::Write;
    let mut text = String::new();
    for (n, f) in names.iter().zip(found) {
        if let Some(p) = f.as_ref().filter(|p| p.is_absolute() && !under_cwd(p)) {
            text.push_str(&format!("{n}\t{}\n", p.display()));
        }
    }
    if text.is_empty() {
        return;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // One write call per batch: appends of this size do not interleave.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
    {
        let _ = f.write_all(text.as_bytes());
    }
}
