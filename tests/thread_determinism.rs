//! The thread count is a performance knob only.
//!
//! `TEXRS_THREADS` and `--jobs` size the rayon pool, the speculative prelex and
//! the document batch. None of them may change a byte of the PDF or a line of
//! the terminal output, so the same document is run at several counts and the
//! results compared exactly.

use std::path::PathBuf;
use std::process::Command;

/// A document big enough to cross every parallel threshold: many paragraphs
/// (paragraph-parallel line breaking), several pages, and enough source for the
/// prelex to split it. Macro use keeps the expander honest.
fn document() -> String {
    let mut s = String::from("\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\\def\\w#1{word#1 and more words to fill a line out }\n");
    for p in 0..600 {
        for i in 0..12 {
            s.push_str(&format!("\\w{{{}}}", p * 12 + i));
        }
        s.push_str("\\par\n");
    }
    s.push_str("\\message{done}\\end\n");
    s
}

/// stderr without phase-timing lines (`T <phase> <duration>`), which vary by run.
fn untimed(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .lines()
        .filter(|l| !l.starts_with("T "))
        .collect::<Vec<_>>()
        .join("\n")
}

struct Run {
    stdout: String,
    stderr: String,
    pdf: Vec<u8>,
}

fn run(dir: &PathBuf, threads: &str, via_flag: bool) -> Run {
    let _ = std::fs::remove_file(dir.join("doc.pdf"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_texrs"));
    cmd.current_dir(dir)
        .env("TEXRS_CACHE", "0")
        // The PDF carries a creation date and an /ID derived from it.
        .env("SOURCE_DATE_EPOCH", "1700000000");
    if via_flag {
        cmd.arg(format!("--jobs={threads}"));
    } else {
        cmd.env("TEXRS_THREADS", threads);
    }
    let out = cmd.arg("doc.tex").output().expect("texrs runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: untimed(&out.stderr),
        pdf: std::fs::read(dir.join("doc.pdf")).unwrap_or_default(),
    }
}

#[test]
fn one_thread_and_many_produce_identical_output() {
    let dir = std::env::temp_dir().join(format!("texrs_threads_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("doc.tex"), document()).expect("write source");

    let one = run(&dir, "1", false);
    assert!(!one.pdf.is_empty());
    for (threads, via_flag) in [("2", false), ("8", false), ("8", true), ("1", true)] {
        let many = run(&dir, threads, via_flag);
        assert_eq!(
            one.stdout, many.stdout,
            "stdout at {threads} (flag={via_flag})"
        );
        assert_eq!(
            one.stderr, many.stderr,
            "stderr at {threads} (flag={via_flag})"
        );
        assert!(
            one.pdf == many.pdf,
            "PDF differs at {threads} threads (flag={via_flag})"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A LaTeX document, whose preamble is assembled from scans that run on
/// threads of their own: the package load (with a refusal reported on stderr),
/// the class's numbering, the counter and switch allocations, and the label
/// seedings -- the second run's from the `.aux` the first one left. Every
/// piece has to land in the same place at any thread count, and the page count
/// the run reports is the writer's, so it is checked against the file.
#[test]
fn a_latex_preamble_is_the_same_at_any_thread_count() {
    let dir = std::env::temp_dir().join(format!("texrs_threads_latex_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let mut src = String::from(
        "\\documentclass{article}\n\\usepackage{no-such-package-texrs}\n\
         \\newcounter{thing}\\newif\\ifdraft\n\\begin{document}\n",
    );
    for i in 0..40 {
        src.push_str(&format!(
            "\\section{{S{i}}}\\label{{s{i}}} See \\ref{{s{}}} and \\ref{{gone}}.\n\n",
            (i + 1) % 40
        ));
        for _ in 0..8 {
            src.push_str("Words enough to fill out a page or two of text here. ");
        }
        src.push_str("\n\n");
    }
    src.push_str("\\end{document}\n");
    std::fs::write(dir.join("doc.tex"), src).expect("write source");

    let mut seen: Vec<Vec<Run>> = Vec::new();
    for threads in ["1", "8"] {
        let _ = std::fs::remove_file(dir.join("doc.aux"));
        // Twice: the first run writes the `.aux`, the second seeds from it.
        seen.push(vec![run(&dir, threads, false), run(&dir, threads, false)]);
    }
    for pass in 0..2 {
        let (one, many) = (&seen[0][pass], &seen[1][pass]);
        assert_eq!(one.stdout, many.stdout, "stdout, run {pass}");
        assert_eq!(one.stderr, many.stderr, "stderr, run {pass}");
        assert!(one.pdf == many.pdf, "PDF differs, run {pass}");
    }
    let last = &seen[0][1];
    let pages = texrs::pdf_page_count(&last.pdf);
    assert!(pages > 1, "{pages}");
    let said = format!(
        "Output written on doc.pdf ({pages} pages, {} bytes).",
        last.pdf.len()
    );
    assert!(last.stdout.contains(&said), "{}", last.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_thread_count_is_ignored_not_fatal() {
    let dir = std::env::temp_dir().join(format!("texrs_threads_bad_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(
        dir.join("doc.tex"),
        "\\catcode`\\{=1 \\catcode`\\}=2 \\message{ok}\\end\n",
    )
    .expect("write source");
    let good = run(&dir, "1", false);
    let bad = run(&dir, "zero", false);
    assert_eq!(good.stdout, bad.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}
