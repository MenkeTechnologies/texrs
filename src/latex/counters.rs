//! Register numbers for the counters a document declares.
//!
//! A LaTeX counter is a `\count` register, and `latex.ltx` allocates one per
//! counter with plain TeX's `\newcount`. That allocator cannot be written in
//! texrs -- `src/latex/allocate.rs` handles `\newcount` and its eleven
//! relatives the same way, and for the same reason: it reads a register, adds
//! one and FREEZES the result into a name, and freezing is exactly what this
//! engine's `\edef` will not do --
//! `\count255=5 \edef\x{\the\count255}\count255=9` leaves `\x` saying 9.
//! `src/latex/kernel.tex` says the same thing at more length, with the other
//! three refusals that rule out the alternatives.
//!
//! So the allocation happens here, before the run, over the names the source
//! asks for. It is a scan of the text rather than an expansion: a counter whose
//! name is built by a macro is not seen, and gets the loud
//! `! Missing number, found \cr@NAME.` that an unallocated counter produces
//! rather than a wrong number.

use std::collections::BTreeSet;

/// The first register this hands out.
///
/// `kernel.tex` writes 100..=119 itself, for the counters every LaTeX document
/// has whether it declares them or not. 254 and 255 are the scratch registers
/// `\@roman` and the option machinery use.
const FIRST: usize = 120;

/// The last register available. TeX has 256 count registers; the two above 253
/// are scratch.
const LAST: usize = 253;

/// The counter names `src` declares, in the order they first appear.
///
/// Both spellings that create one: `\newcounter{name}` from ltcounts, and
/// `\newtheorem{name}` from ltthm, which declares a counter of the same name
/// unless it is told to share another one.
pub fn declared(src: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for command in ["\\newcounter", "\\newtheorem"] {
        for name in names_after(src, command) {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }
    out
}

/// Every `{...}` group that follows `command` in `src`, as a trimmed name.
///
/// `\newtheorem*{thm}` and `\newcounter {x}` both count: the star and the
/// spaces between the command and its brace are skipped, which is what the
/// mouth does with them.
fn names_after(src: &str, command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find(command) {
        let after = &rest[at + command.len()..];
        rest = after;
        // A longer command that merely starts with this one is not this one:
        // `\newcounterfoo` is its own control sequence.
        if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let after = after.trim_start_matches(['*', ' ', '\t', '\n']);
        let Some(body) = after.strip_prefix('{') else {
            continue;
        };
        let Some(end) = body.find('}') else { continue };
        let name = body[..end].trim();
        // A counter name is letters; anything else is a group this scan
        // mistook for one.
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '@') {
            out.push(name.to_string());
        }
    }
    out
}

/// Every counter name `src` NAMES, whether or not it declares it.
///
/// The commands that take a counter's name in their first brace group.
/// `\refstepcounter` is listed on its own: the scan looks for the backslash as
/// well as the letters, so `\stepcounter` does not find it.
pub fn mentioned(src: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for command in [
        "\\newcounter",
        "\\newtheorem",
        "\\setcounter",
        "\\addtocounter",
        "\\stepcounter",
        "\\refstepcounter",
        "\\value",
        "\\arabic",
        "\\roman",
        "\\Roman",
        "\\alph",
        "\\Alph",
        "\\fnsymbol",
    ] {
        for name in names_after(src, command) {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }
    out
}

/// The counter names `kernel.tex` allocates, read out of the kernel itself.
///
/// Parsed rather than listed a second time: the register block is written as
/// `\@ctrreg{NAME}{NN}` lines, and a copy of the names here would go stale the
/// first time one of them moved.
fn kernel_counters() -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = super::PRELUDE;
    while let Some(at) = rest.find("\\@ctrreg{") {
        let after = &rest[at + "\\@ctrreg{".len()..];
        rest = after;
        let Some(end) = after.find('}') else { continue };
        out.push(after[..end].to_string());
    }
    out
}

/// The register and dispatch definitions this document's counters need.
///
/// Two kinds of line, and the second is the one worth explaining:
///
///  * a counter the document DECLARES gets `\@ctrreg{NAME}{N}`, exactly the
///    shape `kernel.tex` writes for the counters every document has;
///  * a counter the document only NAMES -- `\setcounter{a}{1}` with no
///    `\newcounter{a}` above it -- gets a `\ctr@a` that produces nothing.
///
/// The second exists because a write to a counter with no register expanded to
/// the characters `\count \cr@a =1`, and an assignment is not expandable, so
/// anywhere a macro is expanded without being executed -- a `\message` body
/// above all -- those characters were written out as the document's text.
/// LaTeX answers this case with `! LaTeX Error: No counter 'a' defined.`; with
/// no error recovery to carry that into, the nearest thing is the stand-in's
/// own behaviour, which is to consume the arguments and produce nothing.
pub fn allocations(src: &str) -> String {
    let declared = declared(src);
    let mut out = String::new();
    for (index, name) in declared.iter().enumerate() {
        let register = FIRST + index;
        if register > LAST {
            // Running out is a real limit and is said rather than wrapped
            // around onto a register another counter is using.
            out.push_str(&format!(
                "% {name}: no register left (TeX has {LAST}); it will fail loudly when stepped.\n"
            ));
            continue;
        }
        out.push_str(&format!("\\@ctrreg{{{name}}}{{{register}}}\n"));
    }
    let known: BTreeSet<String> = kernel_counters().into_iter().chain(declared).collect();
    for name in mentioned(src) {
        if known.contains(&name) {
            continue;
        }
        // `\cl@NAME` as well, because \stepcounter walks the reset list of the
        // counter it stepped and an undefined list macro leaks its own name the
        // same way the assignment did.
        out.push_str(&format!(
            "\\@namedef{{ctr@{name}}}#1#2{{}}\
             \\expandafter\\let\\csname cl@{name}\\endcsname\\@empty\n"
        ));
    }
    if out.is_empty() {
        return out;
    }
    format!("% This document's counters -- see src/latex/counters.rs.\n{out}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn declared_counters_are_found_in_both_spellings() {
        let src = r"\newcounter{step}\newtheorem{lemma}{Lemma}\newcounter {gap}[step]";
        assert_eq!(super::declared(src), ["step", "gap", "lemma"]);
    }

    #[test]
    fn a_longer_command_is_not_this_one() {
        assert!(super::declared(r"\newcounterish{x}").is_empty());
    }

    #[test]
    fn allocations_start_past_the_kernel_block() {
        let out = super::allocations(r"\newcounter{step}");
        assert_eq!(out.lines().last().unwrap(), r"\@ctrreg{step}{120}");
    }

    #[test]
    fn a_counter_named_but_never_declared_is_given_a_write_that_does_nothing() {
        let out = super::allocations(r"\setcounter{a}{1}");
        assert!(out.contains(r"\@namedef{ctr@a}#1#2{}"), "{out}");
        // A counter the kernel already allocated is NOT given one: its write is
        // the real assignment.
        assert!(!super::allocations(r"\setcounter{page}{1}").contains("ctr@page"));
    }
}
