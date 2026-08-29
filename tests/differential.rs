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
    let known: Vec<String> =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/known_gaps.txt"))
            .expect("known_gaps.txt")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| l.split_whitespace().next().map(str::to_string))
            .collect();

    let mut failures = Vec::new();
    let mut fixed = Vec::new();
    let all = cases();
    assert!(!all.is_empty(), "no cases found");
    for case in &all {
        let want = reference(&tex, case);
        let src = std::fs::read_to_string(case).expect("read case");
        let got = match texrs::run_messages(&src) {
            Ok(m) => m,
            Err(e) => format!("ERROR: {}", e.0),
        };
        let name = case.file_name().unwrap().to_string_lossy().to_string();
        let listed = known.contains(&name);
        match (want == got, listed) {
            // Diverges and is not on the list: a regression, or a new gap that
            // has to be written down before it can be tolerated.
            (false, false) => {
                failures.push(format!("{name}\n  tex   : {want:?}\n  texrs : {got:?}"))
            }
            // On the list and now passing: the list is stale. Removing the entry
            // is part of the fix, so this fails too.
            (true, true) => fixed.push(name),
            _ => {}
        }
    }
    assert!(
        failures.is_empty() && fixed.is_empty(),
        "{} unlisted divergence(s), {} stale known-gap entr(y/ies) of {} cases\n\n{}{}",
        failures.len(),
        fixed.len(),
        all.len(),
        failures.join("\n\n"),
        match fixed.is_empty() {
            true => String::new(),
            false => format!(
                "\n\nthese now PASS and must be removed from tests/known_gaps.txt: {}",
                fixed.join(", ")
            ),
        }
    );
}
