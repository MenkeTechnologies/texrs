//! The LaTeX layer texrs can honour.
//!
//! LaTeX is a program written in TeX, so this carries it as TeX: `prelude.tex`
//! is a file of `\newcommand`s, and adding a macro is a line there rather than
//! a branch in Rust. What it covers is the part of LaTeX that lives in the
//! mouth and the expander. Everything needing the stomach -- boxes, glue,
//! fonts, output routines -- is out of reach, so a macro that would have drawn
//! something yields its text instead and a macro that would have set state the
//! stomach reads consumes its arguments and produces nothing.
//!
//! That is a deliberate reading of what "run this document" can mean for an
//! engine with no typesetter: what the text says, minus what the packages would
//! have drawn. It is not LaTeX, and a document whose meaning IS its layout will
//! not survive it.

/// The prelude, compiled into the binary so a run needs no support files.
pub const PRELUDE: &str = include_str!("latex/prelude.tex");

/// Whether a source looks like a LaTeX document.
///
/// Keyed on the preamble directives rather than on a flag, because a user does
/// not think of their file as needing a mode -- `\documentclass` IS the
/// statement that this is LaTeX. A plain TeX document contains none of these
/// and is unaffected, which matters: the prelude redefines names like
/// `\section` that a plain document may have defined for itself.
pub fn looks_like_latex(src: &str) -> bool {
    const MARKERS: [&str; 4] = [
        "\\documentclass",
        "\\usepackage",
        "\\PassOptionsToPackage",
        "\\RequirePackage",
    ];
    MARKERS.iter().any(|m| src.contains(m))
}
