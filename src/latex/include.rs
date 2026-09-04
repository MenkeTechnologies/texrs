//! The files a document reads into itself, gathered before the run.
//!
//! Everything this layer decides in Rust is decided by SCANNING the source:
//! which counters exist (`counters.rs`), which `\newif` switches to write
//! (`switches.rs`), which labels are referenced and which keys are cited
//! (`aux.rs`). A book is not one file -- it is a `book.tex` of `\include`s and
//! twenty chapters -- so a scan of `book.tex` alone sees none of it, and each
//! of those decisions came out wrong in the same way:
//!
//!  * a `\newcounter` in a chapter got no register, and the `\setcounter` that
//!    used it stopped the run with `! Missing number, found \message.`;
//!  * a `\ref` in a chapter was never seeded, so `\@setref` met an undefined
//!    name and ate the two tokens after the reference;
//!  * a `\newif` in an included preamble fragment got no `\NAMEtrue`.
//!
//! So the text handed to those scans is the document AND what it reads. This is
//! not the mouth's `\input` -- it does not expand, does not respect a
//! conditional, and reads a file the document would have skipped. That is the
//! right trade for what it feeds: every one of those decisions is a definition
//! made ahead of the run, and defining a counter the document turns out not to
//! use costs a register, while missing one costs the document.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// How deep the gather follows one file into another.
///
/// A book is `book.tex` -> chapter -> section, three levels at the most in the
/// corpus. The bound is what stops a file that includes itself.
const MAX_DEPTH: usize = 4;

/// `src`, and the text of every file it reads, joined.
///
/// The result is for SCANNING only: it is not in the order the mouth would read
/// it and the pieces are not separated by anything meaningful.
pub fn gathered(src: &str) -> String {
    let mut out = String::from(src);
    let mut seen = BTreeSet::new();
    gather_into(src, 0, &mut seen, &mut out);
    out
}

fn gather_into(src: &str, depth: usize, seen: &mut BTreeSet<PathBuf>, out: &mut String) {
    if depth >= MAX_DEPTH {
        return;
    }
    for name in named(src) {
        let Some(path) = resolve(&name) else { continue };
        if !seen.insert(path.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.push('\n');
        out.push_str(&text);
        gather_into(&text, depth + 1, seen, out);
    }
}

/// The file names in `src`'s `\input` and `\include` directives.
///
/// Both spellings of `\input`: `\input{name}`, which is LaTeX's, and
/// `\input name`, which is TeX's and ends at the first space. `\include` always
/// takes braces.
pub fn named(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for command in ["\\input", "\\include"] {
        let mut rest = src;
        while let Some(at) = rest.find(command) {
            let after = &rest[at + command.len()..];
            rest = after;
            // `\includegraphics` and `\inputencoding` are not these.
            if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let after = after.trim_start_matches([' ', '\t']);
            let name = match after.strip_prefix('{') {
                Some(body) => match body.find('}') {
                    Some(end) => body[..end].trim().to_string(),
                    None => continue,
                },
                // The unbraced form ends at whitespace, and a control sequence
                // in it -- `\input size1\@ptsize.clo' -- means the name is
                // built by expansion and is not one this scan can resolve.
                None => after
                    .split(|c: char| c.is_whitespace())
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            };
            if !name.is_empty() && !name.contains('\\') && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Where the name is, searched the way `Lowerer::open_input` searches: the
/// working directory first, then `TEXINPUTS`, with `.tex` supplied for a name
/// that carries no extension.
fn resolve(name: &str) -> Option<PathBuf> {
    let candidates = match std::path::Path::new(name).extension().is_some() {
        true => vec![name.to_string()],
        false => vec![format!("{name}.tex"), name.to_string()],
    };
    let mut dirs = vec![PathBuf::from(".")];
    if let Ok(paths) = std::env::var("TEXINPUTS") {
        dirs.extend(std::env::split_paths(&paths));
    }
    for dir in &dirs {
        for candidate in &candidates {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_spellings_of_input_and_include_are_names() {
        let got = super::named("\\input{a}\n\\input b.tex\n\\include{c}\n");
        assert_eq!(got, ["a", "b.tex", "c"]);
    }

    #[test]
    fn a_longer_command_is_not_input_and_a_built_name_is_not_scanned() {
        assert!(super::named("\\includegraphics{f.pdf}").is_empty());
        assert!(super::named("\\input size1\\@ptsize.clo").is_empty());
    }
}
