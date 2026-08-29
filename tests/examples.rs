//! Every file in `examples/` must run, and must print what real `tex` prints.
//!
//! Examples are documentation, and documentation that has drifted from the
//! engine is worse than none: someone reads it, types it, and gets a different
//! answer. So they are held to the same contract as `tests/cases` -- the
//! reference is produced by running `tex` here and now, never written by hand.
//!
//! The difference from `tests/differential.rs` is what a failure means. A case
//! in `tests/cases` may diverge if `tests/known_gaps.txt` says why; an example
//! may not. An example that cannot be kept in parity does not belong in
//! `examples/`.

mod common;

use std::path::{Path, PathBuf};

fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("examples/")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_example_runs_and_prints_something() {
    let all = examples();
    assert!(!all.is_empty(), "examples/ has no .tex files");
    for path in &all {
        let src = std::fs::read_to_string(path).expect("read example");
        match texrs::run_messages(&src) {
            Ok(msgs) => assert!(
                !msgs.is_empty(),
                "{} runs but prints nothing -- an example that shows no output \
                 documents nothing",
                path.display()
            ),
            Err(e) => panic!("{} no longer runs: {}", path.display(), e.0),
        }
    }
}

#[test]
fn every_example_matches_real_tex() {
    let Some(tex) = common::tex() else {
        eprintln!("skipping: no pinned `tex` on PATH -- the harness has no oracle");
        return;
    };
    let mut bad = Vec::new();
    for path in &examples() {
        let want = common::reference(&tex, path);
        let src = std::fs::read_to_string(path).expect("read example");
        let got = match texrs::run_messages(&src) {
            Ok(m) => m,
            Err(e) => format!("ERROR: {}", e.0),
        };
        if want != got {
            bad.push(format!(
                "{}\n  tex   : {want:?}\n  texrs : {got:?}",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} example(s) diverge from tex -- an example is not allowed a known gap:\n\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}
