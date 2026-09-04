//! Finding and loading the real `.sty` and `.cls` files a document names.
//!
//! `\usepackage{geometry}` used to be CONSUMED: the arguments were read, the
//! options kept where the typesetter could use them, and the package itself was
//! never opened. That is a defensible reading for an engine with no stomach and
//! an indefensible one for an engine that has a mouth and an expander, because
//! most of what a package does IS mouth and expander work -- it defines macros.
//!
//! What happens instead:
//!
//!  1. the file is looked for with `kpsewhich`, which is how every TeX program
//!     on the machine finds it;
//!  2. it is LOADED -- read through the engine's own `\input`, with `@` a letter
//!     the way `\usepackage` makes it, against the kernel in
//!     `src/latex/kernel.tex`;
//!  3. a package that will not load is reported by name, with the control
//!     sequence that stopped it: `texrs: package tikz needs \pgfutil@packagewarning`.
//!
//! Step 3 is the point. The failure mode this replaces is not "the package did
//! not load", it is "the package did not load and nothing said so", which is
//! how a document quietly produced the wrong thing. A package that cannot load
//! still leaves the document readable -- the stand-ins in `prelude.tex` answer
//! its commands as they did before -- but the run says which package it was and
//! what it wanted.
//!
//! The load is ATTEMPTED before it is committed. A `.sty` is read into a
//! throwaway `Lowerer` first, and only a file that got all the way through is
//! put in the real preamble; a file that stopped halfway would otherwise leave
//! the run with half a package's definitions, which is worse than none.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// What a document asked to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// `article`, `geometry`, ... -- the name in the braces.
    pub name: String,
    /// `cls` for `\documentclass`, `sty` for the other three.
    pub extension: &'static str,
    /// The `[...]` the document wrote, kept because the option code a package
    /// declares with `\DeclareOption` is run against it.
    pub options: String,
}

/// The outcome of trying to load one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The file was found and read all the way through. The path is what goes
    /// into the preamble as an `\input`.
    Loaded(PathBuf),
    /// The file was found and stopped part way. The string is the engine's own
    /// message, which names the control sequence it wanted.
    Refused { path: PathBuf, reason: String },
    /// `kpsewhich` could not find it, or there is no `kpsewhich` on this
    /// machine.
    NotFound,
}

/// The requests in `src`, in the order the document wrote them.
///
/// A scan of the source rather than an expansion, for the same reason
/// `counters.rs` scans: this has to happen BEFORE the run, since what it
/// produces is part of what the run reads. A `\usepackage` whose argument is
/// built by a macro is not seen -- and is consumed exactly as it was before
/// this module existed, so nothing is lost by missing it.
pub fn requests(src: &str) -> Vec<Request> {
    let mut out = Vec::new();
    for (command, extension) in [
        ("\\documentclass", "cls"),
        ("\\LoadClass", "cls"),
        ("\\usepackage", "sty"),
        ("\\RequirePackage", "sty"),
    ] {
        for (options, names) in calls(src, command) {
            for name in names.split(',') {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                out.push(Request {
                    name: name.to_string(),
                    extension,
                    options: options.clone(),
                });
            }
        }
    }
    out
}

/// Every `[options]{names}` pair following `command` in `src`.
///
/// The comment character is honoured, because a preamble is full of commented
/// out `\usepackage` lines and loading one of those would be loading a package
/// the document deliberately did not ask for.
fn calls(src: &str, command: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line_start in comment_free(src) {
        let mut rest = line_start.as_str();
        while let Some(at) = rest.find(command) {
            let after = &rest[at + command.len()..];
            rest = after;
            if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let after = after.trim_start();
            let (options, after) = match after.strip_prefix('[') {
                Some(body) => match body.find(']') {
                    Some(end) => (body[..end].to_string(), body[end + 1..].trim_start()),
                    None => continue,
                },
                None => (String::new(), after),
            };
            let Some(body) = after.strip_prefix('{') else {
                continue;
            };
            let Some(end) = body.find('}') else { continue };
            out.push((options, body[..end].to_string()));
        }
    }
    out
}

/// `src` with everything after an unescaped `%` on each line removed.
///
/// Returned a line at a time so a `\usepackage` cannot be assembled out of two
/// lines that a comment separated.
fn comment_free(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let mut kept = String::with_capacity(line.len());
        let mut escaped = false;
        for ch in line.chars() {
            if escaped {
                kept.push(ch);
                escaped = false;
                continue;
            }
            match ch {
                '\\' => {
                    kept.push(ch);
                    escaped = true;
                }
                '%' => break,
                _ => kept.push(ch),
            }
        }
        out.push(kept);
    }
    out
}

/// Where `kpsewhich` says `NAME.EXT` is.
///
/// `None` when there is no `kpsewhich` on this machine, which is not an error:
/// a texrs built into a container with no TeX installation loads no packages
/// and says so, rather than failing to start.
pub fn resolve(name: &str, extension: &str) -> Option<PathBuf> {
    let key = format!("{name}.{extension}");
    if let Some(hit) = resolutions().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let found = kpsewhich(&key);
    if let Ok(mut c) = resolutions().lock() {
        c.insert(key, found.clone());
    }
    found
}

fn kpsewhich(file: &str) -> Option<PathBuf> {
    resolve_many(std::slice::from_ref(&file.to_string()))
        .into_iter()
        .next()
        .flatten()
}

/// Ask `kpsewhich` for several files at once, and fill the cache with all of
/// them.
///
/// One process rather than one per package, because the process is the cost:
/// a corpus preamble names thirty packages, `kpsewhich` takes roughly half a
/// second to start, and asking separately turned a two-second document into a
/// seventeen-second one. `kpsewhich a.sty b.sty` prints one line per file it
/// found, IN ORDER, and prints nothing for one it did not -- so the answers
/// cannot be matched to the questions by position. They are matched by the file
/// name at the end of each path, which is what kpsewhich was asked for.
pub fn warm(names: &[String]) {
    let missing: Vec<String> = {
        let cache = resolutions();
        let Ok(cache) = cache.lock() else { return };
        names
            .iter()
            .filter(|n| !cache.contains_key(*n))
            .cloned()
            .collect()
    };
    if missing.is_empty() {
        return;
    }
    let found = resolve_many(&missing);
    let Ok(mut cache) = resolutions().lock() else {
        return;
    };
    for (name, path) in missing.iter().zip(found) {
        cache.insert(name.clone(), path);
    }
}

fn resolutions() -> &'static Mutex<BTreeMap<String, Option<PathBuf>>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn resolve_many(names: &[String]) -> Vec<Option<PathBuf>> {
    let empty = vec![None; names.len()];
    let Ok(out) = std::process::Command::new("kpsewhich").args(names).output() else {
        return empty;
    };
    // kpsewhich exits non-zero when ANY of the names was not found, so the
    // status says nothing useful about the ones that were; the lines do.
    let text = String::from_utf8_lossy(&out.stdout);
    let mut by_name: BTreeMap<&str, PathBuf> = BTreeMap::new();
    for line in text.lines() {
        let path = PathBuf::from(line.trim());
        let Some(file) = line.trim().rsplit('/').next() else {
            continue;
        };
        if path.is_file() {
            by_name.insert(file, path);
        }
    }
    names
        .iter()
        .map(|n| by_name.get(n.as_str()).cloned())
        .collect()
}

/// Read one package or class through the engine, and say what happened.
///
/// The answer is memoised per process: a sweep over a corpus asks for
/// `xcolor.sty` once per document, and the answer cannot change between them.
pub fn attempt(request: &Request) -> Outcome {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Outcome>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let key = format!("{}.{}", request.name, request.extension);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let outcome = match resolve(&request.name, request.extension) {
        None => Outcome::NotFound,
        Some(path) => read_through(&path),
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(key, outcome.clone());
    }
    outcome
}

/// The load itself, into a Lowerer nothing else will use.
///
/// `\makeatletter` is what `\usepackage` does before it opens a file
/// (latex.ltx, `\@pushfilename` sets `\catcode`\@=11`), and every `.sty` is
/// written expecting it.
fn read_through(path: &std::path::Path) -> Outcome {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let source = format!("\\makeatletter\\input {}\\makeatother\n", path.display());
    let mut lowerer = crate::lower::Lowerer::new();
    if let Err(e) = lowerer.preload(&super::support(&text)) {
        return Outcome::Refused {
            path: path.to_path_buf(),
            reason: e.0,
        };
    }
    match lowerer.preload(&source) {
        Ok(()) => Outcome::Loaded(path.to_path_buf()),
        Err(e) => Outcome::Refused {
            path: path.to_path_buf(),
            reason: e.0,
        },
    }
}

/// The TeX that loads what `src` asked for, the diagnostics for what it did
/// not, and the TEXT of everything that loaded.
///
/// The three come back together because they are one decision made once: a
/// package is either in the preamble or in the report, never both and never
/// neither. The text is there so the caller can scan the files that really
/// loaded for the declarations Rust makes on their behalf -- a counter
/// register has to be allocated across the document AND its packages at once,
/// or the two hand out the same register twice.
pub fn preamble(src: &str) -> (String, Vec<String>, String) {
    let mut tex = String::new();
    let mut said = Vec::new();
    let mut loaded = String::new();
    let requests = requests(src);
    // One `kpsewhich` for the whole preamble rather than one per package: see
    // `warm`. A corpus book names thirty of them.
    warm(
        &requests
            .iter()
            .map(|r| format!("{}.{}", r.name, r.extension))
            .collect::<Vec<_>>(),
    );
    for request in requests {
        match attempt(&request) {
            Outcome::Loaded(path) => {
                // The options reach the package the way \ProcessOptions reads
                // them: \@curroptions is the list \DeclareOption code is run
                // against. \@currname and \@currext are what \ProvidesPackage
                // records the version under.
                // The \newif switches and the counter registers the file itself
                // declares go in ahead of it, for the reason
                // `src/latex/switches.rs` and `src/latex/counters.rs` give: a
                // .cls uses its own switches a few lines below the \newif that
                // declared them, and the engine can build neither name.
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                loaded.push_str(&text);
                loaded.push('\n');
                tex.push_str(&format!(
                    "\\def\\@currname{{{}}}\\def\\@currext{{{}}}\\def\\@curroptions{{{}}}\n\
                     \\makeatletter\n{}\\input {}\\makeatother\n",
                    request.name,
                    request.extension,
                    request.options,
                    crate::latex::switches::definitions(&text),
                    path.display(),
                ));
            }
            Outcome::Refused { reason, .. } => {
                said.push(format!(
                    "package {} {}",
                    request.name,
                    needs(&reason)
                ));
            }
            Outcome::NotFound => {
                said.push(format!(
                    "package {} was not found by kpsewhich; it is consumed rather than loaded",
                    request.name
                ));
            }
        }
    }
    (tex, said, loaded)
}

/// Turn the engine's message into the sentence the report prints.
///
/// `! Undefined control sequence \pgfutil@packagewarning.` becomes
/// `needs \pgfutil@packagewarning`, which is the fact worth having: the next
/// primitive or kernel macro to write. Anything else is quoted whole rather
/// than being paraphrased into something that might not be what happened.
fn needs(reason: &str) -> String {
    let trimmed = reason.trim().trim_start_matches("! ").trim_end_matches('.');
    match trimmed.strip_prefix("Undefined control sequence ") {
        Some(cs) => format!("needs {cs}"),
        None => format!("is not loadable: {trimmed}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_and_a_comma_list_of_packages_are_all_requests() {
        let src = "\\documentclass[11pt]{extreport}\n\\usepackage{amsmath,amssymb}\n";
        let got = requests(src);
        assert_eq!(got[0].name, "extreport");
        assert_eq!(got[0].extension, "cls");
        assert_eq!(got[0].options, "11pt");
        assert_eq!(got[1].name, "amsmath");
        assert_eq!(got[2].name, "amssymb");
        assert_eq!(got[1].extension, "sty");
    }

    #[test]
    fn a_commented_out_usepackage_is_not_a_request() {
        assert!(requests("% \\usepackage{tikz}\n").is_empty());
        assert_eq!(requests("\\usepackage{a} % \\usepackage{b}\n").len(), 1);
    }

    #[test]
    fn an_undefined_control_sequence_is_reported_as_what_the_package_needs() {
        assert_eq!(
            needs("! Undefined control sequence \\DeclareRelease."),
            "needs \\DeclareRelease"
        );
        assert_eq!(
            needs("! Missing number, found \\catcode."),
            "is not loadable: Missing number, found \\catcode"
        );
    }
}
