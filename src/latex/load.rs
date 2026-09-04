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
//! the run with half a package's definitions, which is worse than none. The
//! same rule binds the other end of the load: a package that goes all the way
//! through and then leaves the engine unable to do what it could before is
//! reported rather than committed -- see `AFTERWARDS`.
//!
//! What a package itself requires is followed. `requests` scans the DOCUMENT,
//! so `graphicx` stopped on `\define@key` -- which is `keyval`'s, and which
//! `graphicx.sty` asks for on its second line. [`chain`] walks the
//! `\RequirePackage`s depth first and hands back the files in the order the
//! mouth must read them.
//!
//! Two questions about the FILE SYSTEM are answered here as well, because the
//! file system is Rust's and not the engine's: [`locate`] is the `kpsewhich`
//! `Lowerer::open_input` falls back to, and [`existence`] is what
//! `\IfFileExists` is wired to.

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
    locate(&format!("{name}.{extension}"))
}

/// Where `kpsewhich` says a whole file name is.
///
/// The same question as [`resolve`] asked with the extension already on the
/// name, which is the shape `\input` has it in: `size10.clo` is a file name and
/// not a package with an extension. `Lowerer::open_input` falls back to this
/// after the working directory and `TEXINPUTS`, so a document that reads a file
/// beside itself never pays for a process and a class that reads one out of
/// `texmf-dist` can still find it.
///
/// Memoised, including the misses: a sweep asks for the same missing
/// `textcomp.cfg` in every document, and a process per ask is what made
/// `kpsewhich` expensive in the first place.
pub fn locate(file: &str) -> Option<PathBuf> {
    if let Some(hit) = resolutions().lock().ok().and_then(|c| c.get(file).cloned()) {
        return hit;
    }
    let found = kpsewhich(file);
    if let Ok(mut c) = resolutions().lock() {
        c.insert(file.to_string(), found.clone());
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

/// The files `text` asks `\IfFileExists` about, and which of them are there.
///
/// `\IfFileExists` and `\InputIfFileExists` ask the file system, and the file
/// system is Rust's. The engine has no `\openin`, no `\ifeof` and -- the point
/// that decides the shape of this -- no way to ask whether a name a `\csname`
/// built is defined, so a run-time dispatch table cannot have a default arm.
///
/// What it CAN do is compare a built name against a sentinel: measured,
/// `\expandafter\ifx\csname @fe@NAME\endcsname\@fe@yes` is decided while
/// lowering and is FALSE for a name nothing defined. So this writes one `\let`
/// per file that really exists and nothing at all for one that does not, and
/// `kernel.tex`'s `\IfFileExists` takes the not-found arm for every other name
/// -- including one built by expansion, which is the answer that macro gave for
/// every name before this existed.
///
/// The names are scanned rather than expanded, for the reason `counters.rs`
/// gives: this is part of what the run READS, so it has to be decided before
/// the run.
pub fn existence(text: &str) -> String {
    let mut out = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for command in ["\\IfFileExists", "\\InputIfFileExists"] {
        for name in braced_names(text, command) {
            if !seen.insert(name.clone()) {
                continue;
            }
            if !exists(&name) {
                continue;
            }
            out.push_str(&format!(
                "\\expandafter\\let\\csname @fe@{name}\\endcsname\\@fe@yes\n"
            ));
        }
    }
    match out.is_empty() {
        true => out,
        false => format!("% The files this document asks for -- see src/latex/load.rs.\n{out}"),
    }
}

/// Whether `name` is somewhere the engine's `\input` would find it.
///
/// The same three places and the same order `Lowerer::open_input` searches: the
/// working directory, `TEXINPUTS`, then the TeX tree. A `\IfFileExists` that
/// said yes to a file `\input` then could not open would be worse than saying
/// no, so the two answers are made out of the same search.
fn exists(name: &str) -> bool {
    if std::path::Path::new(name).is_file() {
        return true;
    }
    if let Ok(paths) = std::env::var("TEXINPUTS") {
        if std::env::split_paths(&paths).any(|d| d.join(name).is_file()) {
            return true;
        }
    }
    locate(name).is_some()
}

/// The literal `{NAME}` after each `command` in `text`.
///
/// A name carrying a backslash is one the engine would have to BUILD --
/// `\IfFileExists{\f@encoding\f@family.fd}` -- and is passed over: it cannot be
/// matched at run time by the `\csname` this feeds, and the arm it then takes
/// is the not-found one, which is what it took before.
fn braced_names(text: &str, command: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in comment_free(text) {
        let mut rest = line.as_str();
        while let Some(at) = rest.find(command) {
            let after = &rest[at + command.len()..];
            rest = after;
            if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let after = after.trim_start_matches([' ', '\t']);
            let Some(body) = after.strip_prefix('{') else {
                continue;
            };
            let Some(end) = body.find('}') else { continue };
            let name = body[..end].trim();
            if !name.is_empty() && !name.contains('\\') {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// `text` without a trailing `\endinput` that nothing but comments follows.
///
/// `\endinput` is a primitive: it ends the current file after the line it is
/// on. texrs's mouth has no such flag, and `kernel.tex` records at length why
/// defining it as a macro that does nothing is worse than leaving it undefined
/// -- it was tried, and a `.sty` that stops early from inside a conditional
/// then read the part it meant to skip, which cost thirteen books in the sweep.
///
/// This is the one shape where ending the file and reading on to the end of it
/// are the SAME result: the \endinput docstrip writes at the bottom of every
/// generated file, with only `%% End of file ...` after it. There it is
/// replaced by \relax, which reads as nothing and still gives a \futurelet
/// something to peek at. It is what lets `keyval.sty` and `inputenc.sty` --
/// whose last line of code it is -- load at all.
///
/// An `\endinput` anywhere else is left exactly where it is, so a file that
/// needs the real primitive stops and is REPORTED rather than misread.
pub fn without_trailing_endinput(text: String) -> String {
    let Some(at) = text.rfind("\\endinput") else {
        return text;
    };
    // On its own line: what precedes it on the line must be blank.
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    if !text[line_start..at].trim().is_empty() {
        return text;
    }
    // And what follows, to the end of the file, must be comments and blanks --
    // including the rest of its own line, which TeX would have read.
    let after = &text[at + "\\endinput".len()..];
    let (first, rest) = match after.find('\n') {
        Some(n) => (&after[..n], &after[n + 1..]),
        None => (after, ""),
    };
    let commentary = |line: &str| {
        let line = line.trim();
        line.is_empty() || line.starts_with('%')
    };
    if !commentary(first) || !rest.lines().all(commentary) {
        return text;
    }
    // `\relax` rather than nothing. The token has to stay: `inputenc.sty` ends
    // `\ProcessOptions\endinput`, `\ProcessOptions` peeks for a star with
    // `\futurelet`, and with the token gone the peek ran off the end of the file
    // -- `! Missing token for \futurelet.` Real TeX peeks at the `\endinput`
    // itself there and finds it is not a star, which is what `\relax` gives it.
    format!("{}\\relax{}", &text[..at], after)
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
        Some(_) => read_through(&chain(request)),
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(key, outcome.clone());
    }
    outcome
}

/// How far a `\RequirePackage` chain is followed.
///
/// `graphicx` requires `graphics` requires `trig` and `graphics.cfg`: three
/// deep in the shallowest real case. The bound is what stops two packages that
/// require each other, which `seen` alone does not -- it stops a cycle, but a
/// long chain of real dependencies is what makes the load expensive.
const MAX_REQUIRES: usize = 8;

/// Every file reading `request` really reads, in the order the mouth reads it.
///
/// A `.sty` requires other packages, and `requests` scans only the DOCUMENT --
/// so `graphicx` stopped on `\define@key`, which is `keyval`'s and which
/// `graphicx.sty` asks for on its second line. Following the chain is what
/// makes a package's own `\RequirePackage` mean anything.
///
/// Depth first and dependencies before the file that wanted them, which is the
/// order `\RequirePackage` puts them in: the requiring file's own definitions
/// come after everything it required.
pub fn chain(request: &Request) -> Vec<(Request, PathBuf)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    walk(request, 0, &mut seen, &mut out);
    out
}

fn walk(
    request: &Request,
    depth: usize,
    seen: &mut std::collections::BTreeSet<PathBuf>,
    out: &mut Vec<(Request, PathBuf)>,
) {
    let Some(path) = resolve(&request.name, request.extension) else {
        return;
    };
    if !seen.insert(path.clone()) {
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if depth < MAX_REQUIRES {
        let required = requests(&text);
        warm(
            &required
                .iter()
                .map(|r| format!("{}.{}", r.name, r.extension))
                .collect::<Vec<_>>(),
        );
        for inner in required {
            walk(&inner, depth + 1, seen, out);
        }
    }
    out.push((request.clone(), path));
}

/// The load itself, into a Lowerer nothing else will use.
///
/// `\makeatletter` is what `\usepackage` does before it opens a file
/// (latex.ltx, `\@pushfilename` sets `\catcode`\@=11`), and every `.sty` is
/// written expecting it.
///
/// The whole chain goes through ONE Lowerer, because that is what the real run
/// does: a package's definitions have to be there when the file that required
/// it is read. The path reported for a refusal is the LAST file in the chain --
/// the one the document asked for -- so the report still names the package the
/// document wrote rather than something it has never heard of.
fn read_through(chain: &[(Request, PathBuf)]) -> Outcome {
    let Some((_, last)) = chain.last() else {
        return Outcome::NotFound;
    };
    let mut text = String::new();
    let mut source = String::new();
    for (request, path) in chain {
        text.push_str(&std::fs::read_to_string(path).unwrap_or_default());
        text.push('\n');
        source.push_str(&preamble_for(request, path));
    }
    let mut lowerer = crate::lower::Lowerer::new();
    if let Err(e) = lowerer.preload(&super::support(&text)) {
        return Outcome::Refused {
            path: last.clone(),
            reason: e.0,
        };
    }
    if let Err(e) = lowerer.preload(&source) {
        return Outcome::Refused {
            path: last.clone(),
            reason: e.0,
        };
    }
    // The load has to leave the engine able to do what it could before it. See
    // `AFTERWARDS`.
    match lowerer.preload(AFTERWARDS) {
        Ok(()) => Outcome::Loaded(last.clone()),
        Err(e) => Outcome::Refused {
            path: last.clone(),
            reason: e.0,
        },
    }
}

/// What must still work once the package has been read.
///
/// A load that goes all the way through can still leave the run WORSE than it
/// was, because a package may redefine a kernel macro into something this
/// engine cannot run. `calc` is the measured case and it is not a small one:
/// it replaces `\setlength` with real length arithmetic, so
/// `\setlength{\parskip}{6pt plus 2pt minus 1pt}` stops being the stand-in that
/// consumed its arguments and becomes an assignment to `\parskip`, which is not
/// a register here. Every Pandoc book writes that line and
/// `\setlength{\emergencystretch}{3em}` besides -- `em` is `\fontdimen6` of the
/// current font, and the mouth has no current font -- so committing `calc` took
/// 83 documents out of the sweep while adding nothing a document could use.
///
/// So the same rule the module opens with is applied to the other end of the
/// load: a package is either in the preamble or in the report, and a package
/// that breaks what the preamble already promised belongs in the report, by
/// name and with the error it caused. This is the *post*-condition to
/// `read_through`'s existing pre-condition.
///
/// Deliberately small. It exercises what the corpus actually writes, not what a
/// format could do, because every line here is a line that can refuse a package
/// -- and a package refused for a construct no document uses is a package lost
/// for nothing.
const AFTERWARDS: &str = "\\makeatletter\n\
     \\setlength{\\parskip}{6pt plus 2pt minus 1pt}\n\
     \\setlength{\\emergencystretch}{3em}\n\
     \\addtolength{\\parskip}{1pt}\n\
     \\makeatother\n";

/// The TeX that reads one file, with everything `\usepackage` sets around it.
///
/// The options reach the package the way `\ProcessOptions` reads them:
/// `\@curroptions` is the list `\DeclareOption` code is run against, and
/// `\@currname` and `\@currext` are what `\ProvidesPackage` records the version
/// under. The `\newif` switches the file itself declares go in ahead of it, for
/// the reason `src/latex/switches.rs` gives: a `.cls` uses its own switches a
/// few lines below the `\newif` that declared them, and the engine can build
/// neither name.
fn preamble_for(request: &Request, path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    format!(
        "\\def\\@currname{{{}}}\\def\\@currext{{{}}}\\def\\@curroptions{{{}}}\n\
         \\makeatletter\n{}\\input {}\\makeatother\n",
        request.name,
        request.extension,
        request.options,
        crate::latex::switches::definitions(&text),
        path.display(),
    )
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
    // A file already read stays read: two packages that both require `keyval`
    // must not read it twice, and the document may name one the class already
    // pulled in.
    let mut read = std::collections::BTreeSet::new();
    for request in requests {
        match attempt(&request) {
            Outcome::Loaded(_) => {
                // Everything the request really reads, dependencies first: see
                // `chain`. The counter registers are allocated over all of it at
                // once by the caller, which is why the TEXT goes back too.
                for (inner, path) in chain(&request) {
                    if !read.insert(path.clone()) {
                        continue;
                    }
                    loaded.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
                    loaded.push('\n');
                    tex.push_str(&preamble_for(&inner, &path));
                }
            }
            Outcome::Refused { reason, .. } => {
                said.push(format!("package {} {}", request.name, needs(&reason)));
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
    fn a_docstrip_endinput_is_replaced_and_anything_else_is_left_alone() {
        // What docstrip writes: the last line of code, then \endinput, then two
        // comment lines. Ending the file there and reading to the end of it are
        // the same result, so the token is replaced by \relax -- which reads as
        // nothing and still gives a \futurelet something to peek at.
        let generated = "\\def\\a{1}\n\\endinput\n%%\n%% End of file `x.sty'.\n";
        let got = without_trailing_endinput(generated.to_string());
        assert!(!got.contains("\\endinput"), "{got}");
        assert!(got.contains("\\relax"), "{got}");
        assert!(got.contains("\\def\\a{1}"), "the code is kept: {got}");
        // Real content after it: the file MEANT to stop, and this cannot say so,
        // so it is left exactly as it is and stops the load by name instead.
        let early = "\\ifx\\x\\y\\endinput\\fi\n\\def\\b{2}\n";
        assert_eq!(without_trailing_endinput(early.to_string()), early);
        // Not on a line of its own is not this shape either.
        let inline = "\\def\\stop{\\endinput}\n%%\n";
        assert_eq!(without_trailing_endinput(inline.to_string()), inline);
    }

    #[test]
    fn only_a_literal_file_name_is_answered_by_the_existence_table() {
        // A name built by expansion cannot be matched by the `\csname` the
        // table feeds, so it is passed over and keeps taking the not-found arm.
        assert_eq!(
            braced_names("\\InputIfFileExists{textcomp.cfg}{}{}\n", "\\InputIfFileExists"),
            ["textcomp.cfg"]
        );
        assert!(braced_names(
            "\\IfFileExists{\\f@encoding\\f@family.fd}{}{}\n",
            "\\IfFileExists"
        )
        .is_empty());
        // A commented-out ask is not an ask.
        assert!(braced_names("% \\IfFileExists{a.tex}{}{}\n", "\\IfFileExists").is_empty());
    }

    #[test]
    fn a_package_is_read_after_what_it_requires() {
        // graphicx's second line is \RequirePackage{keyval}, and \define@key --
        // which graphicx stopped on -- is keyval's. The chain has to put keyval
        // first, and has to end on the file the document actually asked for.
        if resolve("graphicx", "sty").is_none() {
            eprintln!("skipping: kpsewhich cannot find graphicx.sty");
            return;
        }
        let names: Vec<String> = chain(&Request {
            name: "graphicx".into(),
            extension: "sty",
            options: String::new(),
        })
        .into_iter()
        .map(|(r, _)| r.name)
        .collect();
        assert_eq!(names.last().map(String::as_str), Some("graphicx"), "{names:?}");
        assert!(names.contains(&"keyval".to_string()), "{names:?}");
        assert!(
            names.iter().position(|n| n == "keyval") < names.iter().position(|n| n == "graphicx"),
            "what is required is read first: {names:?}"
        );
    }

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
