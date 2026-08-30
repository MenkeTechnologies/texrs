//! `scripts/texrs-pdf`: a .tex to a .pdf, for a build that wants texrs.
//!
//! pandoc 3 will only take a `--pdf-engine` from a fixed list, so this is not
//! one. It does not need to be: a pandoc-driven book build already runs pandoc
//! twice, once to produce the .tex and once to produce the .pdf, and this
//! replaces the second call. texrs emits DVI, so the script is texrs --dvi
//! followed by dvipdfmx -- both of which is what tex has always done.

use std::process::Command;

fn script() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/texrs-pdf")
}

fn have(prog: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {prog}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn the_script_is_executable_and_refuses_a_run_with_no_tex_file() {
    let out = Command::new(script())
        .arg("--some-flag")
        .output()
        .expect("run");
    assert!(!out.status.success(), "no .tex means no work to do");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no .tex"), "and it says why: {err:?}");
}

#[test]
fn a_missing_file_is_reported_rather_than_guessed_at() {
    let out = Command::new(script())
        .arg("nope.tex")
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such file"));
}

#[test]
fn a_document_becomes_a_pdf() {
    if !have("dvipdfmx") {
        eprintln!("skipping: dvipdfmx not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("texrs_pdf_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let tex = dir.join("t.tex");
    std::fs::write(
        &tex,
        "\\documentclass{article}\n\\begin{document}\nhello from texrs\n\\end{document}\n",
    )
    .expect("write");

    // The built binary has to be reachable as `texrs`, which is how a build
    // script would call it.
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_texrs"))
        .parent()
        .expect("bin dir")
        .to_path_buf();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(script())
        .arg(&tex)
        .env("PATH", path)
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pdf = dir.join("t.pdf");
    let size = std::fs::metadata(&pdf).map(|m| m.len()).unwrap_or(0);
    assert!(size > 500, "a real PDF, got {size} bytes");
    let _ = std::fs::remove_dir_all(&dir);
}
