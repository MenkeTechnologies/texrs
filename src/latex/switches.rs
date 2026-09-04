//! The two commands `\newif\ifNAME` is supposed to build.
//!
//! `\newif` makes three control sequences out of one: `\ifNAME`, `\NAMEtrue`
//! and `\NAMEfalse`. `kernel.tex` ports the first -- `\let#1\iffalse` -- and
//! cannot port the other two, because building their names means taking the
//! characters of `ifNAME` apart and dropping the `if`, and both ways TeX spells
//! a control sequence as characters come back from this engine as one opaque
//! token: `\expandafter\g\csstring\ifdraft\@nil` against a macro delimited by
//! `if` answers `! Use of macro doesn't match its definition.`, where the same
//! macro applied to the characters matches and yields `draft`.
//!
//! Rust can take the name apart. So the switches are written out here, ahead of
//! the file that declares them, and `\newif` is left doing the half it can do.
//! This is the same division as `counters.rs`: name arithmetic the expander
//! cannot perform is performed before the run and handed to it as definitions.
//!
//! It matters far more than it looks. `\newif` is the first thing most `.sty`
//! and `.cls` files do, and the switch is used a few lines later --
//! `article.cls` declares `\if@titlepage` and calls `\@titlepagefalse` inside
//! its own `\DeclareOption`, and without the switch the class stops there.

/// The `NAME`s in every `\newif\ifNAME` in `text`.
pub fn declared(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\newif") {
        let after = &rest[at + "\\newif".len()..];
        rest = after;
        if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let after = after.trim_start_matches([' ', '\t', '\n', '\r']);
        let Some(after) = after.strip_prefix("\\if") else {
            continue;
        };
        // A control sequence's name runs to the first non-letter. `@` is a
        // letter inside a package, which is where nearly every one of these is.
        let end = after
            .find(|c: char| !(c.is_ascii_alphabetic() || c == '@'))
            .unwrap_or(after.len());
        let name = &after[..end];
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// `\def\NAMEtrue{\let\ifNAME\iftrue}` and its false half, for each of them.
///
/// Written with `\csname` rather than by naming the control sequences directly,
/// because a switch's name holds `@` and this text is read with `@` a letter
/// only inside `\makeatletter`; `\csname` needs no catcode at all for the
/// characters of a name it builds.
pub fn definitions(text: &str) -> String {
    let names = declared(text);
    if names.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("% The \\newif switches this file declares -- see src/latex/switches.rs.\n");
    for name in names {
        out.push_str(&format!(
            "\\expandafter\\def\\csname {name}true\\endcsname\
             {{\\expandafter\\let\\csname if{name}\\endcsname\\iftrue}}\n\
             \\expandafter\\def\\csname {name}false\\endcsname\
             {{\\expandafter\\let\\csname if{name}\\endcsname\\iffalse}}\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_switch_is_found_by_the_name_after_if() {
        assert_eq!(
            super::declared("\\newif\\if@titlepage\n\\newif \\ifdraft%\n"),
            ["@titlepage", "draft"]
        );
    }

    #[test]
    fn a_longer_command_is_not_newif() {
        assert!(super::declared("\\newiffy\\ifx").is_empty());
    }

    #[test]
    fn both_halves_of_the_switch_are_written() {
        let out = super::definitions("\\newif\\ifdraft");
        assert!(out.contains("\\csname drafttrue\\endcsname"));
        assert!(out.contains("\\csname draftfalse\\endcsname"));
        assert!(out.contains("\\csname ifdraft\\endcsname\\iftrue"));
    }
}
