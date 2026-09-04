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
//! Three things are decided in Rust because the engine has no primitive for
//! them, each in its own module:
//!
//!  * [`load`] finds the real `.sty` and `.cls` with `kpsewhich` and reads them
//!    through the engine, and REPORTS by name every package that would not go
//!    through and the control sequence that stopped it.
//!  * [`counters`] allocates a `\count` register per counter the document
//!    declares, because `\newcount`'s allocator needs to freeze a number into a
//!    name and this engine's `\edef` freezes nothing.
//!  * [`aux`] is the `.aux` round trip `\ref` rests on, because there is no
//!    `\write`, no `\openin` and no `\read`.

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
    out.push_str(PRELUDE);
    out.push_str("\\catcode`\\@=11\n");
    out.push_str(&counters::allocations(about));
    // The switches the KERNEL declares as well as `about`'s own: a class reads
    // \if@compatibility without declaring it, and latex.ltx is where it comes
    // from.
    out.push_str(&switches::definitions(PRELUDE));
    out.push_str(&switches::definitions(about));
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
        out.push_str("\\def\\@ctrtotext#1{}\n");
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
/// Which one a document gets is decided by the CLASS FILE, not by a list of
/// class names: `report.cls` and `book.cls` say `\newcounter {chapter}` and
/// `article.cls` does not, and neither do the classes built on article that a
/// name-matching rule would have to be told about one at a time. The space
/// before the brace is the class file's own -- `report.cls:264` -- so what is
/// looked for is the counter NAME after the command, through the same scan
/// `counters.rs` uses. A class `kpsewhich` cannot find is read as report's,
/// which is what this layer carried before there was a choice to make.
fn numbering(src: &str) -> &'static str {
    const ARTICLE: &str = "\
\\def\\thesection{\\arabic{section}}\n\
\\def\\thefigure{\\arabic{figure}}\n\
\\def\\thetable{\\arabic{table}}\n\
\\def\\theequation{\\arabic{equation}}\n";
    let Some(class) = load::requests(src).into_iter().find(|r| r.extension == "cls") else {
        return "";
    };
    let Some(path) = load::resolve(&class.name, "cls") else {
        return "";
    };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    match counters::declared(&text).iter().any(|c| c == "chapter") {
        true => "",
        false => ARTICLE,
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
