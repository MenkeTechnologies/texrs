//! The LaTeX layer texrs can honour.
//!
//! LaTeX is a program written in TeX, so this carries it as TeX. There are two
//! files of it, and the difference between them is the whole shape of this
//! layer:
//!
//!  * `prelude.tex` is the STAND-IN. A macro that would have drawn something
//!    yields its text instead; a macro whose effect the stomach would have read
//!    consumes its arguments and produces nothing. Nothing in it is LaTeX's own
//!    definition, and it is not meant to be.
//!  * `kernel.tex` is the PORT. Every definition in it is the one `latex.ltx`
//!    makes, with the place it came from written above it, and every place a
//!    definition could NOT be carried over says which primitive is missing.
//!    Counters, cross references, `\newenvironment`, the option and file
//!    machinery and the footnote and caption numbering are there.
//!
//! The kernel is read after the stand-in, so a name in both ends up with the
//! ported definition and the stand-in stays for the names the port has not
//! reached.
//!
//! Four things are decided in Rust because the engine has no primitive for
//! them, each in its own module:
//!
//!  * [`load`] finds the real `.sty` and `.cls` with `kpsewhich` and reads them
//!    through the engine, and REPORTS by name every package that would not go
//!    through and the control sequence that stopped it.
//!  * [`counters`] allocates a `\count` register per counter the document
//!    declares, because `\newcount`'s allocator needs to freeze a number into a
//!    name and this engine's `\edef` freezes nothing.
//!  * [`allocate`] does the same for `\newcount` itself and its eleven
//!    relatives, for the same reason: the allocator's whole job is to freeze
//!    the next free register number into a name.
//!  * [`aux`] is the `.aux` round trip `\ref` rests on, because there is no
//!    `\write`, no `\openin` and no `\read`.

pub mod allocate;
pub mod aux;
pub mod counters;
pub mod include;
pub mod load;
pub mod switches;

/// The prelude and the kernel, compiled into the binary so a run needs no
/// support files.
///
/// One constant rather than two because every caller wants both and in this
/// order: the ported definitions have to win over the stand-ins they replace.
pub const PRELUDE: &str = concat!(
    include_str!("latex/prelude.tex"),
    include_str!("latex/kernel.tex")
);

/// Which of the two things a run is producing.
///
/// It matters to exactly one definition, `\@ctrtotext` -- `kernel.tex` says why
/// at length -- and the two entry points below differ only in that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `\message` and the expander: a counter can be read into the output.
    Messages,
    /// `--text`, `--dvi`, `--pdf`: it cannot, because neither `\number` nor
    /// `\the` reaches the text stream.
    Text,
}

/// Everything a LaTeX document needs read before itself.
///
/// The prelude and the kernel, then the register allocations for the counters
/// this document declares, then the packages that really load. The order is the
/// one the pieces depend on: `\@namedef` is the kernel's, and a counter's
/// register has to exist before `\newcounter` names it.
pub fn preamble(src: &str) -> String {
    preamble_in(src, Mode::Messages)
}

/// The same, for a run that is setting the document's own words.
pub fn preamble_text(src: &str) -> String {
    preamble_in(src, Mode::Text)
}

/// The environment a document is read in, minus the document and its packages.
///
/// Split out because `load::attempt` has to build the same thing before it can
/// say whether a `.sty` really loads: a probe running with LESS than the real
/// preamble would report a package as refused over a macro the real run would
/// have given it, and the report is the whole point of the probe. `about` is
/// the text about to be read, scanned for the declarations Rust has to make on
/// its behalf -- the counter registers and the `\newif` switches.
pub fn support(about: &str) -> String {
    let mut out = String::with_capacity(PRELUDE.len() + 8192);
    // The `\newcount` family's registers go in FIRST, ahead of the prelude, so
    // that a file allocating a name the kernel has its own answer for --
    // `\newdimen\maxdimen` is plain TeX's opening line -- ends up with the
    // kernel's. See `allocate`.
    //
    // The kernel's own declarations and `about`'s are allocated in ONE pass
    // over both, and in that order: two passes would each start at the first
    // free register and hand the same one to two different names, which is the
    // fault `counters.rs` records for the counters.
    out.push_str(&allocate::definitions(&format!("{PRELUDE}\n{about}")));
    out.push_str(PRELUDE);
    out.push_str("\\catcode`\\@=11\n");
    out.push_str(&counters::allocations(about));
    // The switches the KERNEL declares as well as `about`'s own: a class reads
    // \if@compatibility without declaring it, and latex.ltx is where it comes
    // from.
    out.push_str(&switches::definitions(PRELUDE));
    out.push_str(&switches::definitions(about));
    // Which of the files `about` asks \IfFileExists about are really there.
    out.push_str(&load::existence(about));
    out
}

fn preamble_in(src: &str, mode: Mode) -> String {
    // Everything scanned below is scanned over the document AND the chapters it
    // reads: a book declares its counters, writes its labels and refers to them
    // in files `book.tex` only names. See `include`.
    let src = &include::gathered(src);
    let (packages, said, loaded) = load::preamble(src);
    report(&said);
    // The counter registers are allocated over the document AND the files that
    // really loaded, together: two separate allocations would both start at 120
    // and hand the same register to two counters.
    let mut out = support(&format!("{src}\n{loaded}"));
    out.reserve(packages.len() + 1024);
    out.push_str(&packages);
    out.push_str(numbering(src));
    if mode == Mode::Text {
        // `\number` and `\the` are refused in the text stream -- measured:
        // `\count110=7 A\number\count110 B` under --text answers
        // `! Undefined control sequence \number.` A footnote mark and a caption
        // number therefore carry no digits on this path, which is exactly what
        // they carried before the kernel was ported, so nothing regresses; the
        // counters are still stepped and are still right everywhere they are
        // TESTED or written into the `.aux`.
        // `@' has to be made a letter again first. `support' left it one, and
        // every package `load::preamble' put in front of this ends on
        // `\makeatother' -- so by the line below it is `other' again, and
        // `\def\@ctrtotext' without this reads as `\def\@' followed by the
        // characters `ctrtotext'. It defined the wrong thing, silently, for as
        // long as any package loaded at all.
        out.push_str("\\catcode`\\@=11\n");
        out.push_str("\\def\\@ctrtotext#1{}\n");
        // \@ctrtotext is the door the KERNEL reads a counter through, and a
        // package reads one through `\thefootnote' instead -- which is
        // `\arabic{footnote}', which is `\@arabic{\count110}', which is
        // `\number'. footnote.sty does exactly that inside \@makefnmark, so
        // `\usepackage{footnote}' plus one \footnote stopped a --text run with
        // `! Undefined control sequence \number.' while the same document ran
        // before the package could load. Shutting the door and leaving the
        // window open is not shutting the door: the one definition that can
        // reach \number is stopped here too, and the digits it would have
        // produced are the digits this path already cannot carry.
        out.push_str("\\def\\@arabic#1{}\n");
    }
    // `??` for every label the document REFERENCES, before anything says what a
    // label resolves to. LaTeX decides this with `\ifx\r@x\relax' inside
    // \@setref and texrs cannot ask that question (`src/latex/aux.rs' says why),
    // so the name is always defined and the unresolved case is defined to `??'
    // -- LaTeX's own answer. Without it \@setref reached an UNDEFINED name and
    // \@firstoftwo took the two tokens after the reference instead: `\ref{x} on
    // page' came out as `n page', the reference gone and the word after it eaten.
    // A run that knows which FILE the document is overwrites these from the .aux
    // it left last time; see `preamble_at`.
    out.push_str(&aux::seeds(&aux::Aux::default(), src));
    out.push_str("\\catcode`\\@=12\n");
    out
}

/// The preamble with the labels a previous run left in `document.aux` seeded in.
///
/// Separate because it needs to know which FILE the document is, and the entry
/// points in `lib.rs` that take a string and nothing else cannot say.
pub fn preamble_at(document: &std::path::Path, src: &str, mode: Mode) -> String {
    let previous = aux::read(&aux::path_for(document));
    let mut out = preamble_in(src, mode);
    out.push_str("\\catcode`\\@=11\n");
    out.push_str(&aux::seeds(&previous, &include::gathered(src)));
    out.push_str("\\catcode`\\@=12\n");
    out
}

/// The `\the<counter>` definitions that belong to a class with no chapters.
///
/// `kernel.tex` writes report's: `\thesection` is `\thechapter.\arabic{section}`
/// and a figure is numbered `2.4`. article numbers a section `2` and a figure
/// `4`, and the difference is visible the moment a document says `\ref` -- an
/// article's first section came back as `0.1`.
///
/// Which one a document gets is decided by the CLASS FILE rather than by a list
/// of class names -- [`class_declares_chapters`] is the question, and it says
/// how it is asked. A class that is neither one of LaTeX's own nor findable by
/// `kpsewhich` is read as report's, which is what this layer carried before
/// there was a choice to make.
fn numbering(src: &str) -> &'static str {
    const ARTICLE: &str = "\
\\def\\thesection{\\arabic{section}}\n\
\\def\\thefigure{\\arabic{figure}}\n\
\\def\\thetable{\\arabic{table}}\n\
\\def\\theequation{\\arabic{equation}}\n";
    match class_declares_chapters(src) {
        Some(true) | None => "",
        Some(false) => ARTICLE,
    }
}

/// Whether the class `src` names declares a `chapter` counter.
///
/// `report.cls` and `book.cls` say `\newcounter {chapter}` and `article.cls`
/// does not, and neither do the classes built on article that a name-matching
/// rule would have to be told about one at a time -- so the CLASS FILE is what
/// is read, through the same scan `counters.rs` uses. A class file that cannot
/// be read falls back to `base_class_declares_chapters`; `None` is a class
/// outside that list which `kpsewhich` cannot find either, or a source with no
/// `\documentclass` at all.
///
/// Public because it answers a question outside this module as well.
/// `typeset::unit_numbers` counts chapter, section and subsection and joins
/// EVERY level down to the one being asked for, so an article's first section
/// comes back as `0.1` where LaTeX writes `1` -- BUGS.md records it. The join
/// has to start at the shallowest level the CLASS declares, and this is the
/// fact that says which that is.
pub fn class_declares_chapters(src: &str) -> Option<bool> {
    let class = load::requests(src)
        .into_iter()
        .find(|r| r.extension == "cls")?;
    let read = load::resolve(&class.name, "cls").and_then(|p| std::fs::read_to_string(p).ok());
    match read {
        Some(text) => Some(counters::declared(&text).iter().any(|c| c == "chapter")),
        None => base_class_declares_chapters(&class.name),
    }
}

/// Whether one of the classes LaTeX itself ships declares a `chapter` counter,
/// answered from the name because there is no file to read.
///
/// A machine with no TeX installation has no `kpsewhich` and so no class file
/// at all, and reading every unfindable class as report's numbered an
/// `article`'s first section `0.1` -- the reference resolved to a chapter the
/// document has not got. These five names are LaTeX's own classes and their
/// answer is a fact about LaTeX rather than about the installation, so it can
/// be stated here; anything else is still `None`, because a class built on
/// article is exactly what a name-matching rule cannot be told about.
fn base_class_declares_chapters(name: &str) -> Option<bool> {
    match name {
        "report" | "book" => Some(true),
        "article" | "proc" | "letter" | "slides" | "minimal" => Some(false),
        _ => None,
    }
}

/// Say, once each, what would not load.
///
/// On stderr and by name, because the failure this replaces is the silent one:
/// a package that did not load, a document that ran anyway, and output that was
/// wrong with nothing to say why. Once per process, since a corpus sweep asks
/// the same question of the same package in every document.
fn report(said: &[String]) {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(BTreeSet::new()));
    let Ok(mut seen) = seen.lock() else { return };
    for line in said {
        if seen.insert(line.clone()) {
            eprintln!("texrs: {line}");
        }
    }
}

/// Whether a source looks like a LaTeX document.
///
/// Keyed on the preamble directives rather than on a flag, because a user does
/// not think of their file as needing a mode -- `\documentclass` IS the
/// statement that this is LaTeX. A plain TeX document contains none of these
/// and is unaffected, which matters: the prelude redefines names like
/// `\section` that a plain document may have defined for itself.
pub fn looks_like_latex(src: &str) -> bool {
    const MARKERS: [&str; 7] = [
        "\\documentclass",
        "\\usepackage",
        "\\PassOptionsToPackage",
        "\\RequirePackage",
        // A header fragment included into a document with
        // `--include-in-header` has no preamble of its own -- it IS preamble --
        // and the three below are names LaTeX defines and plain TeX has none
        // of, so a file containing one is LaTeX by the same reading.
        "\\makeatletter",
        "\\newenvironment",
        "\\begin{document}",
    ];
    MARKERS.iter().any(|m| src.contains(m))
}

#[cfg(test)]
mod tests {
    /// The numbering an article gets where there is no class file to read.
    ///
    /// `\thesection` is `\arabic{section}` for a class with no chapter counter
    /// and `\thechapter.\arabic{section}` for one that has it, so reading an
    /// unfindable `article` as report's wrote `\newlabel{sec:a}{{0.1}{0}}` into
    /// the `.aux` -- a reference to a chapter the document has not got, on
    /// every machine with no TeX installation. `numbering` asks this question,
    /// and it is the answer for a name and not for a file.
    #[test]
    fn latex_s_own_classes_are_known_without_a_class_file() {
        use super::base_class_declares_chapters as declares;
        assert_eq!(declares("article"), Some(false));
        assert_eq!(declares("report"), Some(true));
        assert_eq!(declares("book"), Some(true));
        // A class built on article is what a name table cannot be told about,
        // so it stays unanswered here and the file is the only way to know.
        assert_eq!(declares("beamer"), None);
    }
}
