//! Every case in `tests/cases` must produce the same `\message` stream as the
//! real `tex` binary.
//!
//! No expectation is written by hand: the reference is produced by running
//! `tex` here and now, so a case cannot be made to pass by changing texrs. The
//! engine version line is not compared -- texrs does not claim to be TeX Live --
//! but everything tex writes between the opened filename and the closing paren
//! is.
//!
//! Skipped, loudly, when no `tex` is installed; the harness is worth nothing
//! without its oracle and silently passing would be worse than not running.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tex() -> Option<String> {
    let out = Command::new("tex").arg("--version").output().ok()?;
    out.status.success().then(|| "tex".to_string())
}

/// The text between `(./case.tex` and the closing `)`.
fn messages_of(out: &str) -> String {
    for line in out.lines() {
        let Some(rest) = line.strip_prefix("(./") else {
            continue;
        };
        let Some((_, after)) = rest.split_once(".tex") else {
            continue;
        };
        return after.trim().trim_end_matches(')').trim().to_string();
    }
    String::new()
}

fn reference(tex: &str, case: &Path) -> String {
    let dir = std::env::temp_dir().join(format!("texrs-ref-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join("case.tex");
    std::fs::copy(case, &dst).expect("copy case");
    let out = Command::new(tex)
        .args(["-interaction=nonstopmode", "case.tex"])
        .current_dir(&dir)
        .output()
        .expect("run tex");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    messages_of(&text)
}

fn cases() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/cases")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_case_matches_real_tex() {
    let Some(tex) = tex() else {
        eprintln!("skipping: no `tex` on PATH -- the harness has no oracle");
        return;
    };
    let mut failures = Vec::new();
    let all = cases();
    assert!(!all.is_empty(), "no cases found");
    for case in &all {
        let want = reference(&tex, case);
        let src = std::fs::read_to_string(case).expect("read case");
        let got = match texrs::run_messages(&src) {
            Ok(m) => m,
            Err(e) => format!("ERROR: {}", e.0),
        };
        if want != got {
            failures.push(format!(
                "{}\n  tex   : {want:?}\n  texrs : {got:?}",
                case.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases diverge from tex:\n\n{}",
        failures.len(),
        all.len(),
        failures.join("\n\n")
    );
}
