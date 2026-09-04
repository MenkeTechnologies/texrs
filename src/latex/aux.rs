//! The `.aux` file, and the two passes a cross reference needs.
//!
//! `\ref{fig:one}` cannot be answered on the pass that reads it: the figure it
//! names may be three chapters further on. LaTeX solves this by writing every
//! `\label` into `JOB.aux` as it goes and reading that file back at the START of
//! the next run, so a reference is resolved from what the PREVIOUS run knew and
//! a document has to be run twice. `\ref` to a label the `.aux` does not carry
//! sets `??` and the run ends with "Label(s) may have changed. Rerun to get
//! cross-references right."
//!
//! texrs keeps that model and moves the file handling out of TeX, because the
//! engine has no `\write`, no `\openin` and no `\read` -- three primitives, all
//! of them refused, so `\protected@write\@auxout` cannot be ported at all. What
//! replaces it is this module:
//!
//!  * [`read`] parses a `.aux` -- texrs's own from the last run, or the one a
//!    real `latex` left beside the document, which is the same format;
//!  * [`seeds`] turns it into the `\newlabel` calls that go in the preamble,
//!    which is exactly what LaTeX's `\@input{JOB.aux}` does;
//!  * [`pass`] runs the document once with `\label` diverted into the message
//!    stream, and [`write`] puts what came back into the file;
//!  * [`rerun_needed`] compares the two, which is the condition LaTeX prints
//!    the warning on.
//!
//! The `??` for an unresolved reference is produced here rather than in TeX for
//! a reason worth naming: `\@setref` decides it with `\ifx#1\relax`, and that
//! test cannot be written in texrs -- an undefined control sequence is not
//! `\ifx`-equal to `\relax` here, and `\ifcsname` is refused. So the question
//! "was this label ever written down?" is answered where the answer is known,
//! which is next to the file that would have said so.

use std::collections::BTreeMap;
use std::path::Path;

/// What a `.aux` file carries, of the parts that mean anything here.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Aux {
    /// `\newlabel{key}{{value}{page}}` -- what `\ref` and `\pageref` answer.
    pub labels: BTreeMap<String, (String, String)>,
    /// `\bibcite{key}{value}` -- what `\cite` answers.
    pub citations: BTreeMap<String, String>,
}

impl Aux {
    /// Whether it says nothing at all, which is what a first run sees.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty() && self.citations.is_empty()
    }
}

/// Parse the `.aux` text LaTeX writes.
///
/// A modern `\newlabel` carries five fields rather than two -- hyperref adds
/// the anchor, the title and the file -- and the first two are still the number
/// and the page, so both shapes are read by taking the first two groups and
/// ignoring the rest.
pub fn parse(text: &str) -> Aux {
    let mut aux = Aux::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("\\newlabel") {
            let Some((key, rest)) = group(rest) else {
                continue;
            };
            let Some((body, _)) = group(rest) else { continue };
            let Some((value, rest)) = group(&body) else {
                continue;
            };
            let page = group(rest).map(|(p, _)| p).unwrap_or_default();
            aux.labels.insert(key, (value, page));
        } else if let Some(rest) = line.strip_prefix("\\bibcite") {
            let Some((key, rest)) = group(rest) else {
                continue;
            };
            let Some((value, _)) = group(rest) else {
                continue;
            };
            aux.citations.insert(key, value);
        }
    }
    aux
}

/// The first brace group in `text`, and what follows it.
///
/// Brace-counting rather than a search for `}`: a label's value is
/// `{{2.1}{17}}` and stopping at the first close would take `{2.1`.
fn group(text: &str) -> Option<(String, &str)> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &text[start + 1..start + offset];
                    return Some((inner.to_string(), &text[start + offset + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Read `path` if it is there. A missing `.aux` is a first run, not an error.
pub fn read(path: &Path) -> Aux {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(_) => Aux::default(),
    }
}

/// The `.aux` file that belongs to a document.
pub fn path_for(document: &Path) -> std::path::PathBuf {
    document.with_extension("aux")
}

/// The `\newlabel` calls that go in the preamble.
///
/// Every label the `.aux` knows, plus `{??}{??}` for every label the document
/// REFERENCES and the `.aux` does not know. The second half is what makes
/// `\ref` to an unwritten label produce LaTeX's own `??` instead of stopping
/// the run on an undefined control sequence.
pub fn seeds(aux: &Aux, src: &str) -> String {
    let mut out = String::new();
    for (key, (value, page)) in &aux.labels {
        out.push_str(&format!("\\newlabel{{{key}}}{{{{{value}}}{{{page}}}}}\n"));
    }
    for key in referenced(src) {
        if !aux.labels.contains_key(&key) {
            out.push_str(&format!("\\newlabel{{{key}}}{{{{??}}{{??}}}}\n"));
        }
    }
    for (key, value) in &aux.citations {
        out.push_str(&format!("\\bibcite{{{key}}}{{{value}}}\n"));
    }
    for key in cited(src) {
        if !aux.citations.contains_key(&key) {
            out.push_str(&format!("\\bibcite{{{key}}}{{?}}\n"));
        }
    }
    out
}

/// The bibliography keys `src` cites.
///
/// A `\cite{a,b}` names two of them, so the braces are split on commas: the
/// seed has to cover every key, and a `\b@a,b` nobody defined would leak its own
/// name into the text the way an unseeded `\r@` did.
pub fn cited(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for group in keys_after(src, &["\\cite", "\\citep", "\\citet", "\\nocite"]) {
        for key in group.split(',') {
            let key = key.trim();
            if !key.is_empty() && !out.iter().any(|k| k == key) {
                out.push(key.to_string());
            }
        }
    }
    out
}

/// The labels `src` refers to, through any of the five commands that take one.
pub fn referenced(src: &str) -> Vec<String> {
    keys_after(src, &["\\ref", "\\pageref", "\\eqref", "\\autoref", "\\nameref"])
}

/// The labels `src` defines.
pub fn defined(src: &str) -> Vec<String> {
    keys_after(src, &["\\label"])
}

fn keys_after(src: &str, commands: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for command in commands {
        let mut rest = src;
        while let Some(at) = rest.find(command) {
            let after = &rest[at + command.len()..];
            rest = after;
            if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let after = after.trim_start_matches(['*', ' ']);
            // \hyperref[key]{text} spells its label in brackets; the five here
            // all use braces, and anything else is a different command.
            let Some(body) = after.strip_prefix('{') else {
                continue;
            };
            let Some(end) = body.find('}') else { continue };
            let key = body[..end].trim();
            if !key.is_empty() && !out.iter().any(|k| k == key) {
                out.push(key.to_string());
            }
        }
    }
    out
}

/// Run `src` once with `\label` diverted, and collect what it wrote down.
///
/// This is LaTeX's first pass. `\label` produces nothing in an ordinary run --
/// a label is a target, not text -- so the entries are taken out through the
/// message stream, which is the one channel an engine with no `\write` has.
/// `\@auxlabel` in `kernel.tex` is the diversion, and it writes exactly the
/// line LaTeX's `\protected@write\@auxout` writes.
pub fn pass(src: &str) -> Result<Aux, crate::TexError> {
    let src_d = crate::rust_ffi::desugar(src);
    let mut lowerer = crate::lower::Lowerer::new();
    lowerer.preload(&super::preamble(&src_d))?;
    lowerer.preload(
        "\\makeatletter\\let\\label\\@auxlabel\\let\\bibitem\\@auxbibitem\\makeatother\n",
    )?;
    let cmds = lowerer.lower(&src_d)?;
    let chunk = crate::compiler::Compiler::new().compile(&cmds)?;
    let messages = crate::runtime::run(chunk).map_err(crate::TexError)?;
    let mut aux = Aux::default();
    for message in messages {
        let parsed = parse(&crate::text_without_marks(&message));
        aux.labels.extend(parsed.labels.into_iter().filter(settled));
        aux.citations
            .extend(parsed.citations.into_iter().filter(|(_, v)| is_settled(v)));
    }
    Ok(aux)
}

/// Whether an entry the pass collected is a VALUE rather than the TeX that
/// would have produced one.
///
/// `\thepart` is `\Roman{part}`, and `\@roman` converts by subtracting into a
/// scratch register -- an assignment, which does not survive being expanded
/// without being executed, and the message this pass reads the entries out of
/// is exactly that. The entry written for a `\label` inside a `\part` was
/// `\newlabel{p:a}{{\count 254=\count 100 \relax }{0}}`: TeX source in the
/// place a number belongs, which the next run would have set as the reference.
///
/// Nothing here can evaluate it -- if the pass could not, neither can this --
/// so it is dropped, and the reference falls back to the `??` a label nothing
/// wrote down gets. `??` says "this run did not settle it", which is true;
/// the assignment written out as text says something that is not.
fn settled(entry: &(String, (String, String))) -> bool {
    let (_, (value, page)) = entry;
    is_settled(value) && is_settled(page)
}

fn is_settled(value: &str) -> bool {
    !value.contains('\\')
}

/// Write an `.aux` in the format LaTeX writes and [`parse`] reads.
pub fn write(path: &Path, aux: &Aux) -> std::io::Result<()> {
    let mut text = String::from("\\relax \n");
    for (key, (value, page)) in &aux.labels {
        text.push_str(&format!("\\newlabel{{{key}}}{{{{{value}}}{{{page}}}}}\n"));
    }
    for (key, value) in &aux.citations {
        text.push_str(&format!("\\bibcite{{{key}}}{{{value}}}\n"));
    }
    std::fs::write(path, text)
}

/// Whether the run has to happen again, and LaTeX's own sentence for it.
///
/// The condition is the one `\@testdef` checks: a label whose value differs
/// from what the `.aux` carried, or one that is there now and was not. A
/// document that references a label nothing defines is NOT a rerun -- running
/// again would produce the same `??` -- and LaTeX says so separately.
pub fn rerun_needed(old: &Aux, new: &Aux) -> bool {
    if old.labels.len() != new.labels.len() {
        return true;
    }
    new.labels
        .iter()
        .any(|(key, value)| old.labels.get(key) != Some(value))
}

/// The warning a run prints when [`rerun_needed`] holds. LaTeX's wording.
pub const RERUN: &str = "LaTeX Warning: Label(s) may have changed. \
                         Rerun to get cross-references right.";

/// Bring the `.aux` beside `document` up to date from `src`.
///
/// This is the WRITING half of the round trip, and it is where a run pays for
/// it: [`pass`] reads the whole document a second time. So it is not done at
/// all unless the document both defines a label and refers to one -- a book
/// full of `\label`s that never says `\ref` has nothing to resolve, and running
/// it twice to write a file nothing reads is a whole document's worth of work
/// for no answer.
///
/// A pass that fails leaves the previous `.aux` alone rather than truncating
/// it: the file a former run wrote is better than no file, and the failure is
/// already going to be reported by the run itself.
pub fn update(document: &Path, src: &str) -> bool {
    // Over the chapters as well as the book: a `\label` and the `\ref` that
    // wants it are usually in two different files.
    let src = &super::include::gathered(src);
    let refs = !defined(src).is_empty() && !referenced(src).is_empty();
    let bib = src.contains("\\bibitem") && !cited(src).is_empty();
    if !refs && !bib {
        return false;
    }
    let path = path_for(document);
    let previous = read(&path);
    let Ok(now) = pass(src) else { return false };
    let again = rerun_needed(&previous, &now);
    let _ = write(&path, &now);
    if again {
        eprintln!("texrs: {RERUN}");
    }
    again
}

/// A key for what the `.aux` currently says.
///
/// The bytecode cache is keyed on the `.tex` file and its mtime, and the
/// document's references are resolved from a DIFFERENT file -- so a first run
/// that wrote the labels and a second run that should have read them served the
/// same cached chunk, still carrying `??`. This goes in the cache's mode
/// suffix, so a changed `.aux` is a different chunk.
pub fn stamp(document: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let text = std::fs::read_to_string(path_for(document)).unwrap_or_default();
    if text.is_empty() {
        return String::new();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("-aux{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_field_newlabel_and_a_hyperref_five_field_one_read_the_same() {
        let aux = parse(
            "\\relax\n\
             \\newlabel{sec:a}{{2.1}{17}}\n\
             \\newlabel{fig:b}{{3.4}{92}{A figure}{figure.3.4}{}}\n\
             \\bibcite{knuth84}{1}\n",
        );
        assert_eq!(
            aux.labels["sec:a"],
            ("2.1".to_string(), "17".to_string())
        );
        assert_eq!(aux.labels["fig:b"], ("3.4".to_string(), "92".to_string()));
        assert_eq!(aux.citations["knuth84"], "1");
    }

    #[test]
    fn a_referenced_label_the_aux_does_not_carry_is_seeded_as_two_question_marks() {
        let aux = parse("\\newlabel{known}{{1}{2}}\n");
        let seeded = seeds(&aux, "see \\ref{known} and \\pageref{missing}");
        assert!(seeded.contains("\\newlabel{known}{{1}{2}}"));
        assert!(seeded.contains("\\newlabel{missing}{{??}{??}}"));
    }

    #[test]
    fn a_changed_value_is_a_rerun_and_an_unchanged_one_is_not() {
        let old = parse("\\newlabel{a}{{1}{1}}\n");
        let same = parse("\\newlabel{a}{{1}{1}}\n");
        let moved = parse("\\newlabel{a}{{1}{2}}\n");
        assert!(!rerun_needed(&old, &same));
        assert!(rerun_needed(&old, &moved));
        assert!(rerun_needed(&old, &Aux::default()));
    }
}
