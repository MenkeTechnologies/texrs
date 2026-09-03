//! Compiling many documents at once.
//!
//! A TeX document is wholly independent of its neighbours -- its own mouth,
//! catcode table, macro table and chunk -- so a batch is a fan-out with nothing
//! to synchronise. `tex` cannot do it: one process compiles one file, and a
//! user who wants more reaches for `make -j` and pays a process per document.
//!
//! What has to hold no matter how the threads interleave: the same output as
//! running them one at a time, in ARGUMENT order rather than completion order,
//! and a failure in one document reported without losing the others.

use std::path::PathBuf;
use std::process::Command;

fn texrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_texrs"))
}

/// A directory of documents, and a cache of this test's own so a run never
/// touches the one the user's own texrs runs use.
struct Batch {
    dir: PathBuf,
    files: Vec<String>,
}

impl Batch {
    fn new(name: &str, bodies: &[(&str, String)]) -> Self {
        let dir = std::env::temp_dir().join(format!("texrs_batch_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let mut files = Vec::new();
        for (n, body) in bodies {
            std::fs::write(dir.join(n), body).expect("write doc");
            files.push((*n).to_string());
        }
        Self { dir, files }
    }

    fn run(&self, extra: &[&str]) -> (String, String, bool) {
        let out = texrs()
            .current_dir(&self.dir)
            .env("TEXRS_CACHE_DIR", &self.dir)
            .args(extra)
            .args(&self.files)
            .output()
            .expect("run texrs");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.success(),
        )
    }

    fn one_at_a_time(&self, extra: &[&str]) -> String {
        let mut all = String::new();
        for f in &self.files {
            let out = texrs()
                .current_dir(&self.dir)
                .env("TEXRS_CACHE_DIR", &self.dir)
                .args(extra)
                .arg(f)
                .output()
                .expect("run texrs");
            all.push_str(&String::from_utf8_lossy(&out.stdout));
        }
        all
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A document that prints a word unique to it, so a reordered batch is visible.
fn doc(tag: &str) -> String {
    format!("\\catcode`\\{{=1 \\catcode`\\}}=2\n\\message{{{tag}}}\n\\end\n")
}

#[test]
fn a_batch_prints_what_running_them_one_at_a_time_prints() {
    let bodies: Vec<(&str, String)> = vec![
        ("a.tex", doc("alpha")),
        ("b.tex", doc("beta")),
        ("c.tex", doc("gamma")),
        ("d.tex", doc("delta")),
    ];
    let b = Batch::new("same", &bodies);
    let (batch, _, ok) = b.run(&["--no-cache"]);
    assert!(ok, "batch must succeed");
    assert_eq!(batch, b.one_at_a_time(&["--no-cache"]));
}

#[test]
fn output_follows_argument_order_not_completion_order() {
    // The first document is far larger than the rest, so it finishes last on
    // any thread count. Its line still has to come first, or a build log
    // reorders itself between runs and stops being diffable.
    let mut big = String::from("\\catcode`\\{=1 \\catcode`\\}=2\n");
    for _ in 0..40_000 {
        big.push_str("\\message{x}\n");
    }
    big.push_str("\\message{SLOW}\n\\end\n");
    let bodies: Vec<(&str, String)> = vec![
        ("slow.tex", big),
        ("q1.tex", doc("Q1")),
        ("q2.tex", doc("Q2")),
        ("q3.tex", doc("Q3")),
    ];
    let b = Batch::new("order", &bodies);
    let (out, _, ok) = b.run(&["--no-cache"]);
    assert!(ok);
    // Each document prints its file line and then what became of its PDF, as
    // lualatex does. The order under test is the documents' order, so this
    // reads the file lines and ignores the rest.
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("(./")).collect();
    assert_eq!(lines.len(), 4, "one file line per document, got {out:?}");
    assert!(lines[0].starts_with("(./slow.tex"), "got {:?}", lines[0]);
    assert!(lines[1].starts_with("(./q1.tex"), "got {:?}", lines[1]);
    assert!(lines[2].starts_with("(./q2.tex"), "got {:?}", lines[2]);
    assert!(lines[3].starts_with("(./q3.tex"), "got {:?}", lines[3]);
}

#[test]
fn the_job_count_does_not_change_the_output() {
    let bodies: Vec<(&str, String)> = (0..12)
        .map(|i| {
            (
                [
                    "d0.tex", "d1.tex", "d2.tex", "d3.tex", "d4.tex", "d5.tex", "d6.tex", "d7.tex",
                    "d8.tex", "d9.tex", "d10.tex", "d11.tex",
                ][i],
                doc(&format!("tag{i}")),
            )
        })
        .collect();
    let b = Batch::new("jobs", &bodies);
    let (one, _, _) = b.run(&["--no-cache", "--jobs=1"]);
    for j in ["--jobs=2", "--jobs=4", "--jobs=16"] {
        let (many, _, ok) = b.run(&["--no-cache", j]);
        assert!(ok, "{j} must succeed");
        assert_eq!(many, one, "{j} disagreed with --jobs=1");
    }
}

#[test]
fn one_bad_document_fails_the_batch_without_losing_the_others() {
    let bodies: Vec<(&str, String)> = vec![
        ("ok1.tex", doc("first")),
        // `\undefinedmacro` is not a primitive and was never defined.
        (
            "bad.tex",
            "\\catcode`\\{=1 \\catcode`\\}=2\n\\undefinedmacro\n\\end\n".to_string(),
        ),
        ("ok2.tex", doc("second")),
    ];
    let b = Batch::new("mixed", &bodies);
    let (out, err, ok) = b.run(&["--no-cache"]);
    assert!(!ok, "a failed document must fail the batch");
    assert!(
        out.contains("first"),
        "the good documents still run: {out:?}"
    );
    assert!(
        out.contains("second"),
        "the good documents still run: {out:?}"
    );
    assert!(!err.is_empty(), "the failure is reported on stderr");
}

#[test]
fn a_single_document_still_takes_the_ordinary_path() {
    // One file must behave exactly as it always did -- the batch path is only
    // for more than one, so a lone document keeps its own error handling.
    let b = Batch::new("single", &[("only.tex", doc("solo"))]);
    let out = texrs()
        .current_dir(&b.dir)
        .args(["--no-cache", "only.tex"])
        .output()
        .expect("run");
    assert!(out.status.success());
    // The file line is what the batch path shares; the line after it says what
    // became of the PDF, and this document ships no page, so it is the same
    // "No pages of output." tex writes.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "(./only.tex solo )\nNo pages of output."
    );
}

#[test]
fn a_missing_file_in_a_batch_is_reported_and_not_fatal_to_the_rest() {
    let bodies: Vec<(&str, String)> = vec![("real.tex", doc("here"))];
    let b = Batch::new("missing", &bodies);
    let out = texrs()
        .current_dir(&b.dir)
        .args(["--no-cache", "real.tex", "nope.tex"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("here"),
        "the real document still ran: {stdout:?}"
    );
    assert!(!out.status.success(), "a missing file fails the batch");
}
