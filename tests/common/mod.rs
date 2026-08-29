//! The oracle every differential test shares.
//!
//! `tests/differential.rs` and `tests/examples.rs` both compare texrs against
//! the real `tex` binary, and they have to ask it the SAME question: two
//! extractions that differ by a space are two different parity contracts, and
//! the one that is wrong reports divergences that are not there. The same
//! reasoning put `scripts/lib.sh` under the two shell harnesses.

use std::path::Path;
use std::process::Command;

/// The engine version BUGS.md says every expectation here was measured against.
///
/// Single-sourced from that document so the number cannot drift from the prose
/// that quotes it.
pub fn pinned_version() -> String {
    let bugs = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("BUGS.md"))
        .expect("BUGS.md");
    bugs.lines()
        .find_map(|l| l.split("measured against **tex ").nth(1))
        .and_then(|rest| rest.split("**").next())
        .expect("BUGS.md must carry a `measured against **tex X.Y**` line")
        .to_string()
}

/// The reference engine, or `None` with the reason printed.
///
/// A DIFFERENT tex is refused rather than used. It does not fail loudly on its
/// own -- it reports a different set of divergences, which reads exactly like a
/// regression in texrs.
pub fn tex() -> Option<String> {
    let out = Command::new("tex").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let banner = String::from_utf8_lossy(&out.stdout);
    let version = banner
        .lines()
        .next()?
        .split("TeX ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .to_string();
    let want = pinned_version();
    if version != want {
        eprintln!(
            "skipping: oracle is tex {version}, but every expectation here was measured \
             against {want} (BUGS.md). Update BUGS.md deliberately, or install {want}."
        );
        return None;
    }
    Some("tex".to_string())
}

/// The text between `(./case.tex` and the closing `)`.
///
/// Continuation lines are joined with nothing between them: tex breaks its
/// terminal output at `max_print_line` mid-token, adding no character of its
/// own. The harness also raises that limit (see `reference`) so a wrap should
/// not happen at all; this is what keeps a wrap from being read as an empty
/// message stream if it does.
pub fn messages_of(out: &str) -> String {
    let Some(at) = out.find("(./") else {
        return String::new();
    };
    let rest = &out[at + 3..];
    let Some((_, after)) = rest.split_once(".tex") else {
        return String::new();
    };
    // The LAST paren, not the first: a message can print `)' itself, and the
    // one that ends the file is the last one tex writes before the summary.
    let body = match after.rfind(')') {
        Some(end) => &after[..end],
        None => after,
    };
    body.replace('\n', "").trim().to_string()
}

pub fn reference(tex: &str, case: &Path) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-ref-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join("case.tex");
    std::fs::copy(case, &dst).expect("copy case");
    let out = Command::new(tex)
        .args(["-interaction=nonstopmode", "case.tex"])
        // Without this tex wraps the terminal output at 79 columns, and the
        // break lands anywhere -- including right after the filename, which
        // leaves the whole message stream on the next line. Kpathsea reads the
        // setting from the environment.
        .env("max_print_line", "8000")
        .current_dir(&dir)
        .output()
        .expect("run tex");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    messages_of(&text)
}
