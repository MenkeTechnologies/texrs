//! The `.bst` interpreter against the real `bibtex`.
//!
//! A style is a program, so the only test worth trusting is the whole output:
//! the same `.aux`, the same database and the same style through both, and the
//! `.bbl` compared line for line. The four styles TeX Live ships are between
//! them a hard exercise of the language -- `alpha` builds labels, `plain`
//! sorts, `unsrt` does not, `abbrv` abbreviates every name -- so a difference
//! in any builtin shows up as a difference in a bibliography.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A database with the shapes that break a reimplementation: a von name, a
/// junior, an accented name, a name given "Last, First", a single-author and a
/// many-author entry, a title needing case changed, a month abbreviation, and
/// an entry type each style formats differently.
const DATABASE: &str = r#"
@preamble{ "\newcommand{\noopsort}[1]{} " }
@STRING{jacm = "Journal of the ACM"}

@article{knuth1984,
  author =  {Knuth, Donald E.},
  title =   {Literate Programming},
  journal = {The Computer Journal},
  year =    1984,
  volume =  27,
  number =  2,
  pages =   {97--111},
  month =   may
}

@book{beethoven,
  author =    {van Beethoven, Ludwig},
  title =     {The Ninth Symphony and Other Works That Have a Rather Long Title},
  publisher = {Deutsche Grammophon},
  year =      1826,
  address =   {Vienna}
}

@inproceedings{three,
  author =    {Aho, Alfred V. and Sethi, Ravi and Ullman, Jeffrey D.},
  title =     {Compilers: Principles, Techniques, and Tools},
  booktitle = {Proceedings of Something},
  year =      1986,
  pages =     {1--10},
  editor =    {de la Vega, Maria and King, Jr., Martin Luther}
}

@article{accent,
  author =  {Erd{\H o}s, P. and {\'E}mile Borel and Jean-Paul Sartre},
  title =   {On a {\TeX} Question of Some Length},
  journal = jacm,
  year =    1961,
  volume =  8
}

@misc{minimal,
  key = {zzz},
  title = {A Note With No Author}
}

@inbook{part,
  crossref =  {whole},
  title =     {A Chapter},
  chapter =   3,
  pages =     {40--50}
}

@inbook{part2,
  crossref = {whole},
  title =    {Another Chapter},
  chapter =  4,
  pages =    {51--60}
}

@book{whole,
  editor =    {Collected, Ed},
  title =     {A Collection},
  publisher = {A Publisher},
  year =      1970,
  series =    {A Series},
  volume =    2
}

@unpublished{odd,
  author = {Nobody, A. N.},
  title =  {Something Unpublished With A Very Long Title Indeed That Must Wrap Somewhere},
  note =   {In preparation}
}

@misc{noopsort,
  key =   {\noopsort{a}zzz},
  title = {Sorted By A Key}
}

@phdthesis{thesis,
  author = {Others, Anne and others},
  title =  {A Thesis},
  school = {A School},
  year =   1999
}
"#;

const AUX: &str = "\\relax\n\
     \\citation{three}\n\
     \\citation{knuth1984}\n\
     \\citation{accent}\n\
     \\citation{beethoven}\n\
     \\citation{minimal}\n\
     \\citation{thesis}\n\
     \\citation{part}\n\
     \\citation{part2}\n\
     \\citation{odd}\n\
     \\citation{noopsort}\n\
     \\citation{knuth1984}\n\
     \\bibstyle{STYLE}\n\
     \\bibdata{refs}\n";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_bibtex_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the real `bibtex` writes for this style, or `None` when there is no
/// TeX on this machine.
fn oracle(dir: &Path, style: &str) -> Option<String> {
    std::fs::write(dir.join("refs.bib"), DATABASE).unwrap();
    std::fs::write(dir.join("t.aux"), AUX.replace("STYLE", style)).unwrap();
    let out = Command::new("bibtex")
        .arg("t")
        .current_dir(dir)
        .output()
        .ok()?;
    // BibTeX exits non-zero for warnings, which this database provokes on
    // purpose; what matters is that it wrote a .bbl.
    let _ = out;
    std::fs::read_to_string(dir.join("t.bbl")).ok()
}

/// What texrs writes for the same run.
fn ours(dir: &Path, style: &str) -> String {
    let path = Command::new("kpsewhich")
        .arg(format!("{style}.bst"))
        .output()
        .expect("kpsewhich");
    let path = String::from_utf8_lossy(&path.stdout).trim().to_string();
    let style = texrs::bst::Style::open(&path).expect("the style reads");
    let aux = texrs::bib::Aux::open(dir.join("t.aux")).expect("the aux reads");
    let db = texrs::bib::Bib::parse_with(DATABASE, &style.macros());
    let (bbl, _warnings) = texrs::bstvm::run(&aux, &style, &db);
    bbl
}

fn compare(style: &str) {
    let dir = scratch(style);
    let Some(want) = oracle(&dir, style) else {
        return;
    };
    // A comparison of two empty files would pass; these say the run really
    // happened and reached the parts the database was built to exercise.
    assert!(
        want.matches("\\bibitem").count() >= 10,
        "{style}: bibtex wrote only {} entries",
        want.matches("\\bibitem").count()
    );
    assert!(
        want.contains("\\cite{whole}"),
        "{style}: the cross-referenced entry was not reached"
    );
    let got = ours(&dir, style);
    if got != want {
        // Show the first line that differs, which is what a person needs.
        let mut ours = got.lines();
        let mut theirs = want.lines();
        let mut line = 0;
        loop {
            line += 1;
            match (ours.next(), theirs.next()) {
                (Some(a), Some(b)) if a == b => continue,
                (a, b) => panic!(
                    "{style}.bst, line {line}:\n  texrs:  {a:?}\n  bibtex: {b:?}\n\n--- ours ---\n{got}\n--- theirs ---\n{want}"
                ),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn plain_writes_the_bbl_bibtex_writes() {
    compare("plain");
}

#[test]
fn unsrt_writes_the_bbl_bibtex_writes() {
    compare("unsrt");
}

#[test]
fn abbrv_writes_the_bbl_bibtex_writes() {
    compare("abbrv");
}

#[test]
fn alpha_writes_the_bbl_bibtex_writes() {
    compare("alpha");
}
