//! `-X bibtex` walks a long `ITERATE` in parallel chunks and keeps a chunk only
//! when it provably saw what the one-entry-at-a-time walk would have shown it.
//! The thread count must never change a byte of the `.bbl` or of the warnings,
//! so each run here is repeated at several counts and compared exactly --
//! including styles built to defeat the speculation, whose later chunks have to
//! be thrown away and run again in order.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Enough entries to cross the parallel threshold at every count tried, with
/// `@string`s, cross-references (and one to nothing), a repeated key, and
/// names with von parts and special characters.
fn database() -> String {
    let mut s = String::from(
        "@string{jacm = \"Journal of the ACM\"}\n@preamble{\"\\newcommand{\\x}{}\"}\n",
    );
    let names = [
        "Donald E. Knuth",
        "Leslie Lamport and Edsger W. Dijkstra",
        "Maria de la Vega and Jean-Paul Sartre",
        "Kurt G{\\\"o}del",
        "{Barnes and Noble}",
    ];
    for i in 0..1200 {
        let author = names[i % names.len()];
        match i % 4 {
            0 => s.push_str(&format!(
                "@article{{e{i}, author = {{{author}}}, title = {{On Thing {i}}}, journal = jacm, year = {}, volume = {}, pages = {{{}--{}}}}}\n",
                1950 + i % 70,
                i % 30,
                i,
                i + 9
            )),
            1 => s.push_str(&format!(
                "@book{{e{i}, author = {{{author}}}, title = {{The Book of {i}}}, publisher = {{Pub}}, year = {}}}\n",
                1950 + i % 70
            )),
            2 => s.push_str(&format!(
                "@inproceedings{{e{i}, author = {{{author}}}, title = {{Talk {i}}}, crossref = {{conf{}}}, pages = {{1--2}}}}\n",
                i % 3
            )),
            _ => s.push_str(&format!(
                "@misc{{e{i}, author = {{{author}}}, title = {{Note {i}}}, crossref = {{nowhere}}}}\n"
            )),
        }
    }
    s.push_str("@article{e7, author = {Someone Else}, title = {Twice}}\n");
    for c in 0..3 {
        s.push_str(&format!(
            "@proceedings{{conf{c}, title = {{Proceedings {c}}}, year = 1990, editor = {{Ed Itor}}}}\n"
        ));
    }
    s
}

/// A style whose `ITERATE`s hand state from entry to entry in ways the first
/// entry does not show: a flag set part way through the walk and read by every
/// entry after it, and an entry that leaves a value on the stack.
const CARRIED: &str = r#"
ENTRY { title } { } { }
INTEGERS { flag count }
FUNCTION {mark}
{ cite$ "e600" = { #1 'flag := } 'skip$ if$
  cite$ write$ " " write$ flag int.to.str$ write$ newline$
}
FUNCTION {leave}
{ cite$ "e900" = { "left behind" } 'skip$ if$
  count #1 + 'count :=
}
FUNCTION {tail}
{ cite$ write$ newline$ }
EXECUTE { #0 'flag := }
READ
ITERATE {mark}
ITERATE {leave}
ITERATE {tail}
EXECUTE { count int.to.str$ write$ newline$ }
"#;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texrs_bibtex_parallel_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The `.bbl` and stderr of `-X bibtex` at `threads`.
fn run(dir: &Path, threads: &str) -> (Vec<u8>, String) {
    let _ = std::fs::remove_file(dir.join("t.bbl"));
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .current_dir(dir)
        .env("TEXRS_THREADS", threads)
        .args(["-X", "bibtex", "t.aux"])
        .output()
        .expect("texrs runs");
    let bbl = std::fs::read(dir.join("t.bbl")).expect("a .bbl was written");
    (bbl, String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Cite every entry under `style` and hold every thread count to one thread.
fn same_at_every_count(style: &str, own_style: Option<&str>) {
    let dir = scratch(style);
    std::fs::write(dir.join("refs.bib"), database()).unwrap();
    if let Some(text) = own_style {
        std::fs::write(dir.join(format!("{style}.bst")), text).unwrap();
    }
    std::fs::write(
        dir.join("t.aux"),
        format!("\\citation{{*}}\n\\bibstyle{{{style}}}\n\\bibdata{{refs}}\n"),
    )
    .unwrap();
    let (want, want_err) = run(&dir, "1");
    assert!(want.len() > 10_000, "{style}: a short .bbl\n{want_err}");
    for threads in ["2", "3", "8"] {
        let (got, got_err) = run(&dir, threads);
        assert!(
            got == want,
            "{style}: the .bbl at {threads} threads differs from one thread"
        );
        assert_eq!(got_err, want_err, "{style}: warnings at {threads} threads");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether the installation has `name.bst`, which a machine without TeX lacks.
fn installed(name: &str) -> bool {
    Command::new("kpsewhich")
        .arg(format!("{name}.bst"))
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

#[test]
fn carried_state_falls_back_to_the_ordered_walk() {
    same_at_every_count("carried", Some(CARRIED));
    // And the fallback is the ordered walk: the flag shows from e600 on, and
    // the count saw every entry.
    let dir = scratch("carried_check");
    std::fs::write(dir.join("refs.bib"), database()).unwrap();
    std::fs::write(dir.join("carried.bst"), CARRIED).unwrap();
    std::fs::write(
        dir.join("t.aux"),
        "\\citation{*}\n\\bibstyle{carried}\n\\bibdata{refs}\n",
    )
    .unwrap();
    let (bbl, _) = run(&dir, "8");
    let bbl = String::from_utf8(bbl).unwrap();
    assert!(bbl.contains("e599 0\n"), "before the flag");
    assert!(bbl.contains("e600 1\n"), "at the flag");
    assert!(bbl.contains("e1199 1\n"), "after the flag");
    assert!(bbl.ends_with("1203\n"), "the count saw every entry");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn standard_styles_are_the_same_at_every_thread_count() {
    for style in ["plain", "alpha", "abbrv", "unsrt"] {
        if installed(style) {
            same_at_every_count(style, None);
        }
    }
}
