//! The registers `\newcount` and its relatives hand out.
//!
//! `\newcount\foo` is plain TeX's allocator (`plain.tex`, §\alloc@): it adds
//! one to the count register that tracks how many counts have been handed out
//! and then FREEZES that number into a name with `\countdef`. The freeze is
//! what this engine cannot do -- `\countdef` reads its number while lowering,
//! and the allocation counter's value is only known at run time. `counters.rs`
//! records the same refusal for `\newcounter`, at length, and takes the same
//! way out: the arithmetic is done here, before the run, and handed to the
//! engine as the `\countdef` it could not compute for itself.
//!
//! So this scans the text for the seven allocating commands, hands each name a
//! register out of the right pool, and writes the `\countdef` / `\dimendef` /
//! `\skipdef` / `\toksdef` / `\chardef` that names it. `kernel.tex` then defines
//! `\newcount` and the rest to CONSUME the name that follows, because by the
//! time the file is read the definition has already been made.
//!
//! It is the stated blocker for six of the packages the load report names --
//! `geometry`, `calc`, `inputenc`, `keyval`, `booktabs`, `enumitem` all stop on
//! `\newcount`, `\newdimen` or `\newtoks` within their first few lines, which
//! is where a package declares its scratch space.
//!
//! What a name built by expansion gets is what `counters.rs` gives it: nothing,
//! and the loud failure that follows, rather than a register that belongs to
//! something else.

use std::collections::{BTreeMap, BTreeSet};

/// Which register file a name is allocated out of, and the primitive that
/// freezes a number into a name for it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pool {
    Count,
    Dimen,
    Skip,
    Toks,
    /// `\box`, `\read`, `\write`, `\language` and `\insert` are not registers
    /// with a value: plain TeX allocates all five with `\chardef`, because the
    /// name stands for the NUMBER of the slot rather than for its contents.
    /// texrs has none of those five stores, so what is made here is exactly
    /// what plain makes -- a name that is a number -- and using it still fails
    /// on the missing primitive rather than on the missing name.
    Char,
}

impl Pool {
    fn primitive(self) -> &'static str {
        match self {
            Pool::Count => "countdef",
            Pool::Dimen => "dimendef",
            Pool::Skip => "skipdef",
            Pool::Toks => "toksdef",
            Pool::Char => "chardef",
        }
    }

    /// The last register in this file. Every one of the five stops at 255:
    /// `\countdef`, `\dimendef`, `\skipdef` and `\toksdef` are register codes
    /// (`tex.web` §433) and `\chardef` is a character code (§434).
    fn last(self) -> usize {
        match self {
            // `counters.rs` owns 120..=253 for LaTeX's counters, `kernel.tex`
            // writes 100..=119 itself, and 254 and 255 are its scratch pair.
            // What is left below all of that is this pool's.
            Pool::Count => 99,
            _ => 255,
        }
    }
}

/// The first register in every pool.
///
/// Not zero: `\count0`..`\count9` are the page numbers every TeX document may
/// write to, and a document that says `\count1=5` must not be writing into a
/// package's scratch space. The same ten are held back in each file for the
/// same reason.
const FIRST: usize = 10;

/// The allocating commands, and what each one allocates.
///
/// `\newlength` is LaTeX's, and it is a SKIP rather than a dimension:
/// `latex.ltx` writes `\newcommand*\newlength[1]{\@ifdefinable#1{\newskip#1}}`,
/// so a LaTeX "length" stretches and shrinks. The rest are plain TeX's own
/// (`plain.tex`, the `\alloc@` block).
const COMMANDS: [(&str, Pool); 12] = [
    ("\\newcount", Pool::Count),
    ("\\newdimen", Pool::Dimen),
    ("\\newlength", Pool::Skip),
    ("\\newskip", Pool::Skip),
    // No `\muskip` register file exists (BUGS.md records it), so a muskip is
    // given a glue register: the allocation succeeds and the name is defined,
    // and an assignment in `mu` still fails on the unit. That is the same trade
    // `Pool::Char` makes -- fail on the missing store, not on the missing name.
    ("\\newmuskip", Pool::Skip),
    ("\\newtoks", Pool::Toks),
    ("\\newbox", Pool::Char),
    ("\\newinsert", Pool::Char),
    ("\\newread", Pool::Char),
    ("\\newwrite", Pool::Char),
    ("\\newlanguage", Pool::Char),
    ("\\newfam", Pool::Char),
];

/// The names `text` allocates, paired with the pool each comes out of, in the
/// order they first appear.
///
/// A name is taken only once however many times it is declared: two files that
/// both say `\newdimen\@tempdima` are asking for the same scratch register, and
/// real TeX answers the second with `! \@tempdima already defined` rather than
/// with a second register.
pub fn declared(text: &str) -> Vec<(String, &'static str)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    // In source order across all twelve commands, so the register a name gets
    // does not depend on which of them declared it.
    let mut found: Vec<(usize, String, Pool)> = Vec::new();
    for (command, pool) in COMMANDS {
        let mut rest = text;
        let mut base = 0usize;
        while let Some(at) = rest.find(command) {
            let after = &rest[at + command.len()..];
            let at_abs = base + at;
            base = at_abs + command.len();
            rest = after;
            // `\newcounter` starts with `\newcount`, and is not it. So does
            // `\newboxes`; the rule is the mouth's -- a control word's name
            // runs to the first non-letter.
            if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            let Some(name) = name_after(after) else {
                continue;
            };
            found.push((at_abs, name, pool));
        }
    }
    found.sort_by_key(|(at, _, _)| *at);
    for (_, name, pool) in found {
        if seen.insert(name.clone()) {
            out.push((name, pool.primitive()));
        }
    }
    out
}

/// The control sequence being declared, out of the text after the command.
///
/// `\newcount\foo`, `\newcount \foo` and LaTeX's `\newlength{\foo}` all name
/// `foo`. A name the engine would have to BUILD -- `\newcount\csname ...` or
/// `\expandafter\newcount\csname c@#1\endcsname` -- is not one this scan can
/// resolve, and is passed over rather than guessed at.
fn name_after(after: &str) -> Option<String> {
    let after = after.trim_start_matches([' ', '\t', '\r', '\n']);
    let after = after.strip_prefix('{').unwrap_or(after).trim_start();
    let after = after.strip_prefix('\\')?;
    let end = after
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '@'))
        .unwrap_or(after.len());
    let name = &after[..end];
    match name.is_empty() || name == "csname" || name == "expandafter" {
        true => None,
        false => Some(name.to_string()),
    }
}

/// The `\countdef`, `\dimendef`, `\skipdef`, `\toksdef` and `\chardef` lines
/// the allocations in `text` come to.
///
/// Written ahead of `PRELUDE` rather than after it, so that a file declaring a
/// name the kernel has an opinion about -- `\newdimen\maxdimen` is plain TeX's
/// own first line -- gets the kernel's definition rather than this one's. `@`
/// is made a letter for the block and put back, because nearly every name here
/// carries one and the prelude is read with `@` still `other`.
pub fn definitions(text: &str) -> String {
    let names = declared(text);
    if names.is_empty() {
        return String::new();
    }
    let mut next: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut out = String::from(
        "% The registers this file allocates -- see src/latex/allocate.rs.\n\\catcode`\\@=11\n",
    );
    for (name, primitive) in names {
        let pool = match primitive {
            "countdef" => Pool::Count,
            "dimendef" => Pool::Dimen,
            "skipdef" => Pool::Skip,
            "toksdef" => Pool::Toks,
            _ => Pool::Char,
        };
        let slot = next.entry(primitive).or_insert(FIRST);
        if *slot > pool.last() {
            // Running out is said rather than wrapped onto a register
            // something else is using -- the same call `counters.rs` makes.
            out.push_str(&format!(
                "% \\{name}: no {primitive} register left (the last is {}).\n",
                pool.last()
            ));
            continue;
        }
        out.push_str(&format!("\\{primitive}\\{name}={slot}\n"));
        *slot += 1;
    }
    out.push_str("\\catcode`\\@=12\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_command_allocates_out_of_its_own_file() {
        let got = declared("\\newcount\\a\\newdimen\\b\\newskip\\c\\newtoks\\d\\newbox\\e");
        assert_eq!(
            got,
            [
                ("a".to_string(), "countdef"),
                ("b".to_string(), "dimendef"),
                ("c".to_string(), "skipdef"),
                ("d".to_string(), "toksdef"),
                ("e".to_string(), "chardef"),
            ]
        );
    }

    #[test]
    fn a_longer_command_is_not_this_one() {
        // `\newcounter` opens with `\newcount` and allocates nothing here.
        assert!(declared("\\newcounter{step}").is_empty());
    }

    #[test]
    fn latex_writes_a_length_with_braces_and_it_is_a_skip() {
        assert_eq!(
            declared("\\newlength{\\parskip@indent}"),
            [("parskip@indent".to_string(), "skipdef")]
        );
    }

    #[test]
    fn a_name_the_engine_would_have_to_build_is_passed_over() {
        assert!(declared("\\expandafter\\newcount\\csname c@x\\endcsname").is_empty());
    }

    #[test]
    fn the_same_name_declared_twice_gets_one_register() {
        let out = definitions("\\newdimen\\@tempdima\n\\newdimen\\@tempdima\n");
        assert_eq!(out.matches("\\dimendef").count(), 1);
        assert!(out.contains("\\dimendef\\@tempdima=10"), "{out}");
    }

    #[test]
    fn the_pools_are_counted_apart_and_start_clear_of_a_documents_own() {
        let out = definitions("\\newcount\\a\\newcount\\b\\newdimen\\c");
        assert!(out.contains("\\countdef\\a=10"), "{out}");
        assert!(out.contains("\\countdef\\b=11"), "{out}");
        assert!(out.contains("\\dimendef\\c=10"), "{out}");
    }

    #[test]
    fn running_out_of_count_registers_is_said_rather_than_wrapped() {
        // A control word's name is LETTERS, so the names have to be too: `\n0`
        // is the one-letter name `n` followed by a digit, and a hundred of
        // those are one register between them.
        let name = |i: usize| format!("n{}{}", (b'a' + (i / 26) as u8) as char, (b'a' + (i % 26) as u8) as char);
        let src: String = (0..120).map(|i| format!("\\newcount\\{}\n", name(i))).collect();
        let out = definitions(&src);
        assert!(
            out.contains(&format!("\\countdef\\{}=99", name(89))),
            "the ninetieth fills the last register: {out}"
        );
        assert!(
            out.contains(&format!(
                "% \\{}: no countdef register left (the last is 99).",
                name(90)
            )),
            "{out}"
        );
    }

    #[test]
    fn the_block_makes_the_at_sign_a_letter_and_puts_it_back() {
        let out = definitions("\\newcount\\c@foo");
        assert!(out.starts_with("% The registers"), "{out}");
        assert!(out.contains("\\catcode`\\@=11\n\\countdef\\c@foo=10"), "{out}");
        assert!(out.trim_end().ends_with("\\catcode`\\@=12"), "{out}");
    }
}
