//! `\input`, compared against `tex`'s.
//!
//! Every case runs the same documents through both engines and compares the
//! file line, because `\input` is as much about what reaches the terminal as
//! about what gets read: tex nests a paren group per open file and writes the
//! closing one hard against the last message, with no space, which is the kind
//! of detail no specification records and only a differential test pins.
//!
//! Skipped, loudly, when no pinned `tex` is installed, like every other
//! differential test here.

mod common;

use std::path::Path;
use std::process::Command;

/// Run `case.tex` through both engines in `dir` and return `(tex, texrs)`.
fn both(tex: &str, dir: &Path) -> (String, String) {
    let line = |out: Vec<u8>| {
        String::from_utf8_lossy(&out)
            .lines()
            .find(|l| l.starts_with("(./"))
            .unwrap_or("")
            .to_string()
    };
    let reference = Command::new(tex)
        .args(["-interaction=nonstopmode", "case.tex"])
        // Without this tex wraps at 79 columns and the comparison is with the
        // wrapping rather than with the output.
        .env("max_print_line", "8000")
        .current_dir(dir)
        .output()
        .expect("run tex");
    let subject = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("case.tex")
        .current_dir(dir)
        .output()
        .expect("run texrs");
    (line(reference.stdout), line(subject.stdout))
}

/// A scratch directory with `case.tex` written, plus whatever else a case needs.
fn documents(case: &str, others: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("case.tex"), case).expect("write case");
    for (name, body) in others {
        std::fs::write(dir.path().join(name), body).expect("write support file");
    }
    dir
}

const CATS: &str = "\\catcode`\\{=1 \\catcode`\\}=2\n";

#[test]
fn an_input_file_nests_its_own_paren_group() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    // No text, only messages: text would ship a page, and a page is the one
    // thing an engine with no stomach cannot print.
    let dir = documents(
        &format!("{CATS}\\message{{[before]}}\n\\input inner\n\\message{{[after]}}\n\\end\n"),
        &[("inner.tex", "\\message{[inner ran]}\n")],
    );
    let (want, got) = both(&tex, dir.path());
    assert_eq!(
        got, want,
        "an \\input file opens a paren at its name and closes it with no space"
    );
}

#[test]
fn a_file_that_prints_nothing_still_shows_its_parens() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    // The close paren attaches to the message before it, and when the file
    // printed nothing that message IS its own open paren: `(./empty.tex)`.
    let dir = documents(
        &format!("{CATS}\\input empty\n\\message{{[done]}}\n\\end\n"),
        &[("empty.tex", "")],
    );
    let (want, got) = both(&tex, dir.path());
    assert_eq!(got, want, "an empty \\input file prints `(./empty.tex)`");
}

#[test]
fn the_extension_is_supplied_and_accepted() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    // `\input inner` and `\input inner.tex` name the same file and print the
    // same path, so the supplied extension must not show up twice.
    for named in ["inner", "inner.tex"] {
        let dir = documents(
            &format!("{CATS}\\input {named}\n\\end\n"),
            &[("inner.tex", "\\message{[inner ran]}\n")],
        );
        let (want, got) = both(&tex, dir.path());
        assert_eq!(got, want, "`\\input {named}` must read inner.tex");
    }
}

#[test]
fn what_the_file_defines_outlives_it() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    // The point of \input: state crosses the file boundary in both directions,
    // a macro defined inside being the case every real document depends on.
    let dir = documents(
        &format!("{CATS}\\input defs\n\\message{{\\greet}}\n\\end\n"),
        &[("defs.tex", "\\def\\greet{[from the input file]}\n")],
    );
    let (want, got) = both(&tex, dir.path());
    assert_eq!(
        got, want,
        "a macro defined in an \\input file is defined after it"
    );
}

#[test]
fn a_file_that_inputs_itself_stops_where_tex_does() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    // tex allows 15 text input levels counting the document's own and reports
    // `! TeX capacity exceeded, sorry [text input levels=15].` for the next.
    // texrs stops there rather than recovering, so the file line is not
    // comparable -- the error is, and matching it is what keeps a runaway
    // \input from being a stack overflow.
    let dir = documents(
        &format!("{CATS}\\input self\n\\end\n"),
        &[("self.tex", "\\input self\n")],
    );
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("case.tex")
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    let got = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let reference = Command::new(&tex)
        .args(["-interaction=nonstopmode", "case.tex"])
        .env("max_print_line", "8000")
        .current_dir(dir.path())
        .output()
        .expect("run tex");
    let want = String::from_utf8_lossy(&reference.stdout)
        .lines()
        .find(|l| l.starts_with("! TeX capacity exceeded"))
        .unwrap_or("")
        .to_string();
    assert_eq!(
        got, want,
        "the limit and its wording are tex's, not texrs's own"
    );
}

#[test]
fn a_missing_file_is_named_in_the_error() {
    // No oracle needed: tex PROMPTS for a replacement name here and texrs
    // stops, which is the error model recorded in BUGS.md. What is worth
    // pinning is that the name the document asked for reaches the message.
    let dir = documents(&format!("{CATS}\\input nosuchfile\n\\end\n"), &[]);
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("case.tex")
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("nosuchfile"),
        "the missing file must be named, got {err:?}"
    );
}
