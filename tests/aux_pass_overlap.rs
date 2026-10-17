//! The label pass and the typesetting run overlap, and nothing may show it.
//!
//! A LaTeX document with cross references is lowered twice per run: once by
//! the pass that collects its labels for the `.aux`, once to typeset it against
//! the `.aux`. With more than one thread and an `.aux` already on disk the two
//! run concurrently, and the typesetting is redone when the pass's `.aux` reads
//! differently from the old one (`latex::aux::update_then`). One thread runs
//! them one after the other. So the same three-run history -- a first run, an
//! unchanged rerun (overlap kept), and an edit that renumbers every label
//! (overlap thrown away) -- is played at one thread and at several, and every
//! byte of stdout, stderr, the PDF and the `.aux` must agree step for step.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Sections that label themselves and refer to their neighbours, with a
/// package that will not load so the run has a report on stderr whose place
/// relative to the rerun warning matters. Each carries mathematics, and a
/// numbered display with a label of its own: the pass once stopped at the first
/// `\frac` and wrote no `.aux`, so on any document with a formula the overlap
/// never ran and neither did the round trip.
fn document(inserted: bool) -> String {
    let mut s = String::from(
        "\\documentclass{article}\n\\usepackage{texrs-no-such-package}\n\\begin{document}\n",
    );
    if inserted {
        s.push_str("\\section{Inserted}\nEvery number after this one moves.\n");
    }
    for n in 1..=120 {
        let back = if n > 1 { n - 1 } else { 120 };
        s.push_str(&format!(
            "\\section{{Part {n}}}\\label{{sec:{n}}}\n\
             \\begin{{equation}}\\frac{{1}}{{\\sqrt{{n}}}}\\label{{eq:{n}}}\\end{{equation}}\n\
             See section~\\ref{{sec:{back}}} and \\cite{{k{n}}} for what came \
             before, and section~\\ref{{sec:{}}} for what follows. {}\n\n",
            n % 120 + 1,
            "Words to fill the paragraph out to a few lines. ".repeat(6)
        ));
    }
    // The citation numbers are what the PDF reads out of the `.aux`: a
    // `\cite` sets the number `\bibcite` gave its key. An entry put in front
    // renumbers every one after it, so a typesetting run that kept the old
    // `.aux` would set every citation one short.
    s.push_str("\\begin{thebibliography}{999}\n");
    if inserted {
        s.push_str("\\bibitem{first} An entry put in front of the rest.\n");
    }
    for n in 1..=120 {
        s.push_str(&format!("\\bibitem{{k{n}}} Entry {n}.\n"));
    }
    s.push_str("\\end{thebibliography}\n\\end{document}\n");
    s
}

#[derive(Debug, PartialEq)]
struct Step {
    stdout: String,
    stderr: String,
    pdf: Vec<u8>,
    aux: String,
}

fn run(dir: &Path, threads: &str) -> Step {
    let _ = std::fs::remove_file(dir.join("doc.pdf"));
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .current_dir(dir)
        .env("TEXRS_CACHE", "0")
        .env("TEXRS_THREADS", threads)
        .env("SOURCE_DATE_EPOCH", "1700000000")
        .arg("doc.tex")
        .output()
        .expect("texrs runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Step {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        pdf: std::fs::read(dir.join("doc.pdf")).unwrap_or_default(),
        aux: std::fs::read_to_string(dir.join("doc.aux")).unwrap_or_default(),
    }
}

/// First run, unchanged rerun, renumbering edit -- in one fresh directory.
fn history(threads: &str) -> Vec<Step> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("texrs_aux_overlap_{}_{threads}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("doc.tex"), document(false)).expect("write source");
    let first = run(&dir, threads);
    let again = run(&dir, threads);
    std::fs::write(dir.join("doc.tex"), document(true)).expect("write edit");
    let edited = run(&dir, threads);
    let _ = std::fs::remove_dir_all(&dir);
    vec![first, again, edited]
}

const RERUN: &str = "Rerun to get cross-references right.";

#[test]
fn overlapping_the_label_pass_changes_no_byte_of_any_run() {
    let one = history("1");
    // The history does what it is meant to: the first run and the edit find
    // labels that moved, the rerun between them finds none.
    assert!(one[0].stderr.contains(RERUN), "{}", one[0].stderr);
    assert!(!one[1].stderr.contains(RERUN), "{}", one[1].stderr);
    assert!(one[2].stderr.contains(RERUN), "{}", one[2].stderr);
    assert!(one[1].aux.contains("\\newlabel{sec:7}{{7}"), "{}", one[1].aux);
    assert!(one[2].aux.contains("\\newlabel{sec:7}{{8}"), "{}", one[2].aux);
    assert!(one[1].aux.contains("\\bibcite{k7}{7}"), "{}", one[1].aux);
    assert!(one[2].aux.contains("\\bibcite{k7}{8}"), "{}", one[2].aux);
    // The equations are numbered by the displays alone, and the inserted
    // section has none, so eq:7 is 7 before the edit and after it.
    assert!(one[1].aux.contains("\\newlabel{eq:7}{{7}"), "{}", one[1].aux);
    assert!(one[2].aux.contains("\\newlabel{eq:7}{{7}"), "{}", one[2].aux);
    assert!(one.iter().all(|s| !s.pdf.is_empty()));

    for threads in ["2", "8"] {
        let many = history(threads);
        for (step, (a, b)) in one.iter().zip(&many).enumerate() {
            assert_eq!(a.stdout, b.stdout, "stdout, step {step}, {threads} threads");
            assert_eq!(a.stderr, b.stderr, "stderr, step {step}, {threads} threads");
            assert_eq!(a.aux, b.aux, "aux, step {step}, {threads} threads");
            assert!(a.pdf == b.pdf, "pdf bytes, step {step}, {threads} threads");
        }
    }
}
