//! File location through `kpsewhich`, shared by every caller.
//!
//! A `kpsewhich` process costs 0.1 to 1 s of wall time, nearly all of it
//! reading the `ls-R` databases, and a document asks for tens of files (fonts,
//! hyphenation patterns, the glyph list, packages). A LaTeX package's own
//! `\RequirePackage` list is known only once the package has been found and
//! read, so those asks come one after another. Four mechanisms remove the cost:
//!
//! * **One process.** `kpsewhich -interactive` is started once and asked every
//!   name in turn on its standard input, so the databases are read once per run
//!   instead of once per ask. Its standard output is a pseudo-terminal, which is
//!   what makes it line-buffered; through a pipe it would hold every answer until
//!   it exited. Where that cannot be set up, each ask spawns its own process.
//! * **Memo.** A name is resolved once per process; concurrent askers for the
//!   same name wait for the one resolution instead of asking again.
//! * **Batch.** [`warm`] resolves many names in one go.
//! * **Disk cache.** Resolved paths are appended to
//!   `<cache dir>/texrs/kpse-<env hash>.tsv`. The file name carries a hash of
//!   the variables that steer the search, so changing `TEXINPUTS` and friends
//!   selects a different file. The file opens with a stamp of the TeX tree --
//!   size and modification time of every `ls-R` database and every `texmf.cnf`
//!   kpathsea reads -- and is discarded when the stamp no longer matches, which
//!   is what installing or updating a package does (`mktexlsr` rewrites
//!   `ls-R`). A recorded path is taken only if the file still exists; misses are
//!   never recorded.
//!
//! The answer for a name is the same whichever mechanism produced it, so output
//! does not depend on timing or thread count.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
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
    /// Some thread is resolving this name.
    Pending,
    Done(Option<PathBuf>),
}

struct State {
    slots: Mutex<HashMap<String, Slot>>,
    resolved: Condvar,
    /// Paths recorded by earlier runs, consulted before asking `kpsewhich`.
    disk: BTreeMap<String, PathBuf>,
    disk_file: Option<PathBuf>,
    /// Whether the disk file has to be started over, under a fresh stamp,
    /// before anything is appended: it is missing, unstamped, or was stamped
    /// against a TeX tree that has since changed. Also serialises writers.
    restart: Mutex<bool>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| {
        let disk_file = disk_cache_path();
        let disk = disk_file.as_deref().and_then(read_disk);
        State {
            slots: Mutex::new(HashMap::new()),
            resolved: Condvar::new(),
            restart: Mutex::new(disk.is_none()),
            disk: disk.unwrap_or_default(),
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

/// Resolve every name not yet known, in one go.
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
                    slots.insert(n.clone(), Slot::Done(Some(p)));
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

/// Start resolving [`STARTUP_FILES`] on a background thread, which also starts
/// the one `kpsewhich` process; and, when the disk cache has to be started
/// over, stamp the TeX tree on another.
///
/// Returns at once. A caller that needs one of the files before the thread has
/// resolved it waits on that name (see [`locate`]); it never asks twice.
pub fn prefetch_startup() {
    let st = state();
    if st.disk_file.is_some() && st.restart.lock().is_ok_and(|r| *r) {
        std::thread::spawn(|| {
            tree_stamp();
        });
    }
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
        append_disk(st, file, names, found);
    }
}

/// Ask `kpsewhich` for all `names`; one answer per name, in order.
///
/// An answer is the first path printed whose last component is the name asked
/// and which is a file. That is the rule a single `kpsewhich a b` process needs
/// -- it prints one line per file it found and nothing for one it did not, so
/// lines cannot be matched to questions by position -- and keeping it for the
/// interactive process too keeps the answers the same either way.
pub fn resolve_many(names: &[String]) -> Vec<Option<PathBuf>> {
    if let Ok(mut server) = server().lock() {
        if let Some(s) = server.as_mut() {
            let mut out = Vec::with_capacity(names.len());
            for n in names {
                let answer = s.ask(n);
                if s.dead {
                    // It stopped answering: this name and the rest are asked
                    // the old way, and so is everything after this.
                    *server = None;
                    out.extend(names[out.len()..].iter().map(|n| spawn_one(n)));
                    break;
                }
                out.push(match answer {
                    Ok(lines) => pick(n, &lines),
                    Err(Unaskable) => spawn_one(n),
                });
            }
            return out;
        }
    }
    spawn_batch(names)
}

fn pick<S: AsRef<str>>(name: &str, lines: &[S]) -> Option<PathBuf> {
    lines
        .iter()
        .map(|l| l.as_ref().trim())
        .find(|l| l.rsplit('/').next() == Some(name) && Path::new(l).is_file())
        .map(PathBuf::from)
}

fn spawn_one(name: &str) -> Option<PathBuf> {
    spawn_batch(std::slice::from_ref(&name.to_string()))
        .into_iter()
        .next()
        .flatten()
}

/// One `kpsewhich NAME...` process.
fn spawn_batch(names: &[String]) -> Vec<Option<PathBuf>> {
    let Ok(out) = Command::new("kpsewhich").args(names).output() else {
        return vec![None; names.len()];
    };
    // Non-zero when ANY name is missing, so the lines are read, not the status.
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    names.iter().map(|n| pick(n, &lines)).collect()
}

/// A name the interactive process cannot be asked: it reads a line per name and
/// stops at `q` or `quit`, so those, and anything a line cannot carry, are
/// asked of a process of their own.
struct Unaskable;

/// Printed after every name, so the end of an answer -- including the empty
/// answer for a name that is not there -- is known. `kpsewhich` answers an
/// existing absolute path with itself.
const SENTINEL: &str = "/dev/null";

struct Server {
    input: ChildStdin,
    output: BufReader<File>,
    _child: Child,
    dead: bool,
    /// What is asked after every name, and what ends its answer: [`SENTINEL`]
    /// for the default search, and a file of the format's own suffix for a
    /// formatted one (see [`find_as`]).
    sentinel: String,
}

impl Server {
    fn start() -> Option<Server> {
        Server::start_with(&[], SENTINEL)
    }

    /// A process started with `options` before `-interactive`, ending every
    /// answer with `sentinel`.
    #[cfg(unix)]
    fn start_with(options: &[String], sentinel: &str) -> Option<Server> {
        let (master, slave) = pty()?;
        let mut child = Command::new("kpsewhich")
            .args(options)
            .args(["-interactive", sentinel])
            .stdin(Stdio::piped())
            .stdout(Stdio::from(slave))
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let input = child.stdin.take()?;
        let mut s = Server {
            input,
            output: BufReader::new(master),
            _child: child,
            dead: false,
            sentinel: sentinel.to_string(),
        };
        // The answer to the name on the command line says it is ready, and
        // that it understood `-interactive`.
        let mut line = String::new();
        match s.output.read_line(&mut line) {
            Ok(n) if n > 0 && line.trim() == s.sentinel => Some(s),
            _ => None,
        }
    }

    #[cfg(not(unix))]
    fn start_with(_options: &[String], _sentinel: &str) -> Option<Server> {
        None
    }

    /// The lines `kpsewhich` printed for `name`. Sets `dead`, with no lines,
    /// when the process stopped answering.
    fn ask(&mut self, name: &str) -> Result<Vec<String>, Unaskable> {
        if name.is_empty()
            || name.trim() != name
            || name.contains(['\n', '\r'])
            || matches!(name, "q" | "quit")
        {
            return Err(Unaskable);
        }
        if writeln!(self.input, "{name}\n{}", self.sentinel)
            .and_then(|()| self.input.flush())
            .is_err()
        {
            self.dead = true;
            return Ok(Vec::new());
        }
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            match self.output.read_line(&mut line) {
                Ok(n) if n > 0 => {}
                _ => {
                    self.dead = true;
                    return Ok(Vec::new());
                }
            }
            if line.trim() == self.sentinel {
                return Ok(lines);
            }
            lines.push(line);
        }
    }
}

/// The one interactive `kpsewhich`, started on first use; `None` when it could
/// not be, which sends every ask to a process of its own.
fn server() -> &'static Mutex<Option<Server>> {
    static SERVER: OnceLock<Mutex<Option<Server>>> = OnceLock::new();
    SERVER.get_or_init(|| Mutex::new(Server::start()))
}

/// A pseudo-terminal, master and slave, both close-on-exec so no other child
/// of this process holds the slave open. Output post-processing is off, so a
/// line arrives as written rather than with `\r\n`.
#[cfg(unix)]
fn pty() -> Option<(File, File)> {
    use std::os::fd::{FromRawFd, RawFd};
    fn cloexec(fd: RawFd) {
        // SAFETY: fcntl on a descriptor this function owns.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    // SAFETY: each descriptor is checked before use and handed to a File,
    // which closes it; `ptsname` is read at once, before any other call.
    unsafe {
        let m = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if m < 0 {
            return None;
        }
        let master = File::from_raw_fd(m);
        cloexec(m);
        if libc::grantpt(m) != 0 || libc::unlockpt(m) != 0 {
            return None;
        }
        let name = libc::ptsname(m);
        if name.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(name).to_owned();
        let s = libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if s < 0 {
            return None;
        }
        let slave = File::from_raw_fd(s);
        cloexec(s);
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(s, &mut t) == 0 {
            t.c_oflag &= !libc::OPOST;
            t.c_lflag &= !(libc::ECHO | libc::ICANON);
            libc::tcsetattr(s, libc::TCSANOW, &t);
        }
        Some((master, slave))
    }
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
        "TEXMFDBS",
        "TEXMFVAR",
        "TEXMFCONFIG",
    ] {
        var.hash(&mut h);
        std::env::var_os(var).hash(&mut h);
    }
    let dir = crate::script_cache::default_cache_path();
    Some(dir.parent()?.join(format!("kpse-{:016x}.tsv", h.finish())))
}

/// The files whose change means the TeX tree changed: `ls-R` in every tree
/// kpathsea keeps a database for and `texmf.cnf` in every directory it reads
/// configuration from, as `kpsewhich` itself reports them. Asked of a process
/// of its own, once, and only on a run that starts the disk cache over.
fn tree_files() -> Vec<PathBuf> {
    let Ok(out) = Command::new("kpsewhich")
        .args(["-show-path=ls-R", "-var-value=TEXMFCNF"])
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let (Some(dbs), Some(cnfs)) = (lines.next(), lines.next()) else {
        return Vec::new();
    };
    // The first is a search path, `:`-separated; the second the variable, which
    // TeX Live writes as a `{a,b,...}` brace list.
    let dirs = |line: &str, file: &'static str| -> Vec<PathBuf> {
        line.trim_start_matches('{')
            .trim_end_matches('}')
            .split([':', ','])
            .map(|d| d.trim_start_matches('!'))
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join(file))
            .collect()
    };
    let mut files = dirs(dbs, "ls-R");
    files.extend(dirs(cnfs, "texmf.cnf"));
    files
}

/// `#tree` header lines naming each of `files` and its size and modification
/// time, or `-` for one that is not there (yet).
fn stamp_of(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|f| format!("#tree\t{}\t{}\n", f.display(), file_stamp(f)))
        .collect()
}

fn file_stamp(f: &Path) -> String {
    let Ok(meta) = std::fs::metadata(f) else {
        return "-".to_string();
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{}:{mtime}", meta.len())
}

/// The header a restarted disk file opens with; `None` when the tree cannot be
/// stamped, and then nothing is recorded, since nothing could be validated.
fn tree_stamp() -> Option<&'static str> {
    static STAMP: OnceLock<Option<String>> = OnceLock::new();
    STAMP
        .get_or_init(|| {
            let files = tree_files();
            (!files.is_empty()).then(|| stamp_of(&files))
        })
        .as_deref()
}

/// A file found through the working directory is not a property of the TeX tree.
fn under_cwd(p: &Path) -> bool {
    std::env::current_dir().is_ok_and(|d| p.starts_with(d))
}

/// The recorded paths, or `None` when the file is missing, carries no stamp, or
/// its stamp no longer matches the tree.
fn read_disk(file: &Path) -> Option<BTreeMap<String, PathBuf>> {
    let text = std::fs::read_to_string(file).ok()?;
    parse_disk(&text)
}

fn parse_disk(text: &str) -> Option<BTreeMap<String, PathBuf>> {
    let (header, body): (Vec<&str>, Vec<&str>) = text.lines().partition(|l| l.starts_with('#'));
    let mut stamped = false;
    for line in header {
        let mut fields = line.strip_prefix("#tree\t")?.splitn(2, '\t');
        let (Some(path), Some(stamp)) = (fields.next(), fields.next()) else {
            return None;
        };
        if file_stamp(Path::new(path)) != stamp {
            return None;
        }
        stamped = true;
    }
    if !stamped {
        return None;
    }
    // Later lines win: a stale path re-resolved is appended, not rewritten.
    Some(
        body.into_iter()
            .filter_map(|l| l.split_once('\t'))
            .map(|(n, p)| (n.to_string(), PathBuf::from(p)))
            .collect(),
    )
}

fn append_disk(st: &State, file: &Path, names: &[String], found: &[Option<PathBuf>]) {
    let mut text = String::new();
    for (n, f) in names.iter().zip(found) {
        if let Some(p) = f.as_ref().filter(|p| p.is_absolute() && !under_cwd(p)) {
            text.push_str(&format!("{n}\t{}\n", p.display()));
        }
    }
    if text.is_empty() {
        return;
    }
    let Ok(mut restart) = st.restart.lock() else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if *restart {
        let Some(stamp) = tree_stamp() else {
            return;
        };
        // A whole new file, renamed into place, so a concurrent reader sees the
        // old file or the new one and never a header without its paths.
        let tmp = file.with_extension(format!("tsv.{}", std::process::id()));
        if std::fs::write(&tmp, format!("{stamp}{text}")).is_ok()
            && std::fs::rename(&tmp, file).is_ok()
        {
            *restart = false;
        }
        return;
    }
    // One write call per batch: appends of this size do not interleave.
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(file) {
        let _ = f.write_all(text.as_bytes());
    }
}

// ── formatted lookups, for the Lua `kpse` library ──────────────────────

/// Where kpathsea finds `name` as a file of `format` for program `progname`:
/// `kpsewhich -progname=P -format=F [-must-exist] NAME`, which is what
/// LuaTeX's `kpse.find_file(name, format, mustexist)` is (lkpselib.c,
/// `find_file` calling `kpse_find_file`).
///
/// [`locate`] cannot answer it: it asks the default search, which guesses the
/// format from a name's suffix, so `luaotfload-main` (the `lua` format adds
/// `.lua`) and `lmroman10-regular` (`opentype fonts` adds `.otf`) are not
/// found at all, and a `.tex` name is looked up under `kpsewhich`'s own
/// program name rather than `lualatex`'s, which is a different TEXINPUTS.
///
/// One interactive process per (program, format, must-exist), each answering
/// on a pseudo-terminal as [`locate`]'s does. The answer to a formatted
/// question has no fixed last line -- `/dev/null` is not a file of most
/// formats and comes back as nothing -- so each process is given a SENTINEL
/// FILE of its own format: an empty file named with the format's first suffix
/// (`kpsewhich -help-formats`), asked by absolute path, which kpathsea answers
/// with itself. Memoised per process; a name with no answer is remembered too,
/// because a package that probes for an optional file asks for it repeatedly.
pub fn find_as(name: &str, format: &str, progname: &str, must_exist: bool) -> Option<PathBuf> {
    type Key = (String, String, bool);
    type Memo = HashMap<(Key, String), Option<PathBuf>>;
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    static SERVERS: OnceLock<Mutex<HashMap<Key, Option<Server>>>> = OnceLock::new();
    let key: Key = (progname.to_string(), format.to_string(), must_exist);
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = memo.lock().ok()?.get(&(key.clone(), name.to_string())) {
        return hit.clone();
    }
    let mut options = vec![format!("-progname={progname}"), format!("-format={format}")];
    if must_exist {
        options.push("-must-exist".to_string());
    }
    let ask_once = |options: &[String]| -> Option<PathBuf> {
        let out = Command::new("kpsewhich")
            .args(options)
            .arg(name)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().next().map(|l| PathBuf::from(l.trim()))
    };
    let found = {
        let mut servers = SERVERS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .ok()?;
        let slot = servers
            .entry(key.clone())
            .or_insert_with(|| sentinel_for(format).and_then(|s| Server::start_with(&options, &s)));
        match slot.as_mut().map(|s| s.ask(name)) {
            Some(Ok(lines)) if !slot.as_ref().is_some_and(|s| s.dead) => lines
                .first()
                .map(|l| PathBuf::from(l.trim()))
                .filter(|p| !p.as_os_str().is_empty()),
            _ => {
                if slot.as_ref().is_some_and(|s| s.dead) {
                    *slot = None;
                }
                ask_once(&options)
            }
        }
    };
    if let Ok(mut m) = memo.lock() {
        m.insert((key, name.to_string()), found.clone());
    }
    found
}

/// The sentinel file [`find_as`] asks after every name for `format`: an empty
/// file, in this process's own temporary directory, carrying the format's
/// first suffix, so kpathsea takes it as a file of that format.
fn sentinel_for(format: &str) -> Option<String> {
    let suffix = format_suffixes()
        .get(format)
        .and_then(|s| s.first().cloned())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("texrs-kpse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join(format!("sentinel{suffix}"));
    if !file.exists() {
        std::fs::write(&file, "").ok()?;
    }
    Some(file.to_string_lossy().into_owned())
}

/// Every format name (and abbreviation) `kpsewhich -help-formats` lists, with
/// the suffixes it gives it, in its order. A line reads
/// `lua: .lua .luatex .luc .luctex .texlua .texluc .tlu  [variables: ...]`, or
/// `bitmap font (bitmapfont):  [variables: ...]` for a format without one.
fn format_suffixes() -> &'static HashMap<String, Vec<String>> {
    static TABLE: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let Ok(out) = Command::new("kpsewhich").arg("-help-formats").output() else {
            return HashMap::new();
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut table = HashMap::new();
        for line in text.lines().filter(|l| !l.starts_with(' ')) {
            let Some((names, rest)) = line.split_once(':') else {
                continue;
            };
            let rest = rest.split("[variables").next().unwrap_or_default();
            let suffixes: Vec<String> = rest
                .split_whitespace()
                .filter(|s| s.starts_with('.'))
                .map(str::to_string)
                .collect();
            let (name, abbrev) = match names.split_once(" (") {
                Some((n, a)) => (n.trim(), Some(a.trim_end_matches(')').trim())),
                None => (names.trim(), None),
            };
            table.insert(name.to_string(), suffixes.clone());
            if let Some(a) = abbrev {
                table.insert(a.to_string(), suffixes);
            }
        }
        table
    })
}

/// `kpsewhich -progname=P OPTION=VALUE`, its first line of output: what
/// `kpse.var_value`, `kpse.expand_var`, `kpse.expand_path`,
/// `kpse.expand_braces` and `kpse.show_path` are, each one a call into
/// kpathsea's variable expansion with nothing to search. `None` when kpsewhich
/// printed nothing, which is its answer for an unset variable. Memoised, since
/// a variable cannot change under a running process.
pub fn query(progname: &str, option: &str, value: &str) -> Option<String> {
    type Memo = HashMap<(String, String, String), Option<String>>;
    static MEMO: OnceLock<Mutex<Memo>> = OnceLock::new();
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (progname.to_string(), option.to_string(), value.to_string());
    if let Some(hit) = memo.lock().ok()?.get(&key) {
        return hit.clone();
    }
    let answer = Command::new("kpsewhich")
        .arg(format!("-progname={progname}"))
        .arg(format!("{option}={value}"))
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|out| {
            let text = String::from_utf8_lossy(&out.stdout);
            let line = text.lines().next().map(str::to_string);
            // An unset variable is a failing exit and an empty line, which is
            // kpathsea's NULL and Lua's nil.
            match (out.status.success(), line) {
                (false, Some(l)) if l.is_empty() => None,
                (_, line) => line,
            }
        });
    if let Ok(mut m) = memo.lock() {
        m.insert(key, answer.clone());
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_stamped_against_a_tree_that_changed_is_discarded() {
        // What a TeX Live update does: a package moves to a new path, the old
        // file stays behind, and `mktexlsr` rewrites `ls-R`. The recorded path
        // still exists, so only the stamp can say it is stale.
        let dir = std::env::temp_dir().join(format!("texrs-kpse-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("ls-R");
        let old = dir.join("old.sty");
        std::fs::write(&db, "% ls-R -- filename database\n./:\nold.sty\n").unwrap();
        std::fs::write(&old, "").unwrap();
        let cnf = dir.join("absent").join("texmf.cnf");

        let cache = format!(
            "{}x.sty\t{}\n",
            stamp_of(&[db.clone(), cnf.clone()]),
            old.display()
        );
        let read = parse_disk(&cache).expect("an unchanged tree keeps the cache");
        assert_eq!(read.get("x.sty"), Some(&old));

        // Same size, new time: a rewrite that happens to be as long counts.
        std::fs::write(&db, "% ls-R -- filename database\n./:\nnew.sty\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        File::options()
            .write(true)
            .open(&db)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(parse_disk(&cache).is_none(), "a rewritten ls-R discards it");

        // A database or configuration file that appears where there was none
        // changes the search as much as one that changes.
        let cache = format!(
            "{}x.sty\t{}\n",
            stamp_of(std::slice::from_ref(&cnf)),
            old.display()
        );
        assert!(parse_disk(&cache).is_some());
        std::fs::create_dir_all(cnf.parent().unwrap()).unwrap();
        std::fs::write(&cnf, "TEXINPUTS = .\n").unwrap();
        assert!(parse_disk(&cache).is_none(), "a new texmf.cnf discards it");

        // A file from before stamps existed is never trusted.
        assert!(parse_disk(&format!("x.sty\t{}\n", old.display())).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_interactive_process_answers_as_a_process_per_name_does() {
        // A found name, a missing one, and a name the interactive process must
        // not be sent (`q` would stop it). Skipped without a TeX installation.
        let names: Vec<String> = ["article.cls", "texrs-no-such-file.sty", "q", "cmr10.tfm"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let batch = spawn_batch(&names);
        if batch[0].is_none() {
            eprintln!("skipping: kpsewhich cannot find article.cls");
            return;
        }
        let Some(mut s) = Server::start() else {
            panic!("kpsewhich is installed but -interactive could not be started");
        };
        let asked: Vec<Option<PathBuf>> = names
            .iter()
            .map(|n| match s.ask(n) {
                Ok(lines) => pick(n, &lines),
                Err(Unaskable) => spawn_one(n),
            })
            .collect();
        assert!(!s.dead);
        assert_eq!(asked, batch);
        assert!(asked[1].is_none());
    }
}
