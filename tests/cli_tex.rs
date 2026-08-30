//! The command line, compared against `tex`'s.
//!
//! `tests/cli.rs` holds the flags against the man page and the completion;
//! this holds the GRAMMAR against the engine it copies. Each case runs the same
//! invocation through both binaries and compares what reaches the terminal,
//! because that is what a build script sees — including the details nobody
//! writes down, like which side of the closing paren a message lands on.
//!
//! Skipped, loudly, when no pinned `tex` is installed, like every other
//! differential test here.

mod common;

use std::path::Path;
use std::process::Command;

/// Run `args` through both engines in `dir` and return `(tex, texrs)` output.
fn both(tex: &str, dir: &Path, args: &[&str]) -> (String, String) {
    let reference = Command::new(tex)
        .arg("-interaction=nonstopmode")
        .args(args)
        // Without this tex wraps at 79 columns and the comparison is with the
        // wrapping, not with the output.
        .env("max_print_line", "8000")
        .current_dir(dir)
        .output()
        .expect("run tex");
    let subject = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run texrs");

    // tex's first line is its banner, which texrs deliberately does not print,
    // and its last lines are the DVI summary, which texrs has no stomach to
    // produce. What both engines write is the middle: the file line.
    let tex_out = String::from_utf8_lossy(&reference.stdout)
        .lines()
        .find(|l| l.starts_with("(./"))
        .unwrap_or("")
        .to_string();
    let texrs_out = String::from_utf8_lossy(&subject.stdout)
        .lines()
        .find(|l| l.starts_with("(./"))
        .unwrap_or("")
        .to_string();
    (tex_out, texrs_out)
}

/// A scratch directory holding the two documents the cases use.
fn documents() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    // One that ends the run, one that merely runs out.
    std::fs::write(
        dir.path().join("ended.tex"),
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{FROMFILE}\n\\end\n",
    )
    .expect("write");
    std::fs::write(
        dir.path().join("open.tex"),
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{FROMFILE}\n",
    )
    .expect("write");
    dir
}

#[test]
fn a_bare_name_finds_the_tex_file() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH");
        return;
    };
    let dir = documents();
    let (want, got) = both(&tex, dir.path(), &["ended", "\\end"]);
    assert_eq!(got, want, "`texrs ended` must read ended.tex, as tex does");
}

#[test]
fn end_inside_the_file_closes_the_paren_with_a_space() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex`");
        return;
    };
    let dir = documents();
    let (want, got) = both(&tex, dir.path(), &["ended.tex", "\\end"]);
    assert_eq!(got, want);
    assert!(want.ends_with(" )"), "tex changed its own shape: {want:?}");
}

#[test]
fn a_file_that_runs_out_closes_the_paren_without_one() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex`");
        return;
    };
    let dir = documents();
    let (want, got) = both(&tex, dir.path(), &["open.tex", "\\end"]);
    assert_eq!(got, want);
    assert!(
        want.ends_with("FROMFILE)"),
        "tex changed its own shape: {want:?}"
    );
}

#[test]
fn arguments_after_an_unfinished_file_are_read_after_it() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex`");
        return;
    };
    let dir = documents();
    let (want, got) = both(&tex, dir.path(), &["open.tex", "\\message{AFTER}", "\\end"]);
    assert_eq!(got, want);
    assert!(
        want.contains(") AFTER"),
        "the trailing input belongs outside the paren: {want:?}"
    );
}

#[test]
fn a_file_that_ended_never_reads_the_arguments_after_it() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex`");
        return;
    };
    let dir = documents();
    let (want, got) = both(
        &tex,
        dir.path(),
        &["ended.tex", "\\message{AFTER}", "\\end"],
    );
    assert_eq!(got, want);
    assert!(
        !want.contains("AFTER"),
        "`\\end` in the file must stop the run: {want:?}"
    );
}

#[test]
fn batchmode_writes_nothing_to_the_terminal() {
    let dir = documents();
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args(["-interaction=batchmode", "ended.tex"])
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "batchmode printed: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn a_backslash_argument_is_input_and_prints_bare() {
    // No file was opened, so tex prints no `(./…)` line at all — the messages
    // stand on their own.
    let dir = documents();
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("\\catcode`\\{=1 \\catcode`\\}=2 \\message{FROMLINE}\\end")
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "FROMLINE");
}

#[test]
fn texs_own_flags_do_not_stop_a_run() {
    // The point of accepting them: an invocation written for tex drives texrs.
    let dir = documents();
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .args([
            "-ini",
            "-halt-on-error",
            "-file-line-error",
            "-recorder",
            "-8bit",
            "-progname=texrs",
            "-jobname=doc",
            "ended.tex",
        ])
        .current_dir(dir.path())
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "texrs refused a tex invocation: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("FROMFILE"));
}
