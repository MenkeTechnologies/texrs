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
//!
//! `examples/extensions/` is the one exception, and it is not an escape hatch:
//! those documents use constructs texrs ADDS to TeX — inline Rust — for which
//! there is no `tex` behaviour to be in parity WITH. They still have to run and
//! print something, which is what the first test below checks for every example
//! in the tree.

mod common;

use std::path::{Path, PathBuf};

/// The examples held to parity with real tex: everything directly in
/// `examples/`.
fn examples() -> Vec<PathBuf> {
    tex_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"))
}

/// The examples that use constructs tex does not have, in
/// `examples/extensions/`. They must run; there is nothing to compare them to.
fn extensions() -> Vec<PathBuf> {
    tex_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/extensions"))
}

fn tex_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_example_runs_and_prints_something() {
    let mut all = examples();
    all.extend(extensions());
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

/// The same comparison with no TeX installed, from the outputs
/// `cargo run --bin parity -- --freeze` recorded.
///
/// CI has no tex, so the live test below skips there — which meant the
/// examples, which are the documentation, went unverified on every push. This
/// is what actually runs in CI; the live one is what proves the frozen file
/// still describes tex.
#[test]
fn every_example_still_prints_what_tex_printed() {
    let frozen = texrs::parity::thawed(include_str!("data/examples_expected.txt"));
    let on_disk: std::collections::BTreeSet<String> = examples()
        .iter()
        .chain(extensions().iter())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let recorded: std::collections::BTreeSet<String> =
        frozen.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(
        on_disk, recorded,
        "the frozen examples and the examples on disk disagree -- run \
         `cargo run --bin parity -- --freeze`"
    );

    let extension_names: std::collections::BTreeSet<String> = extensions()
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    let mut bad = Vec::new();
    for (name, want) in frozen {
        // An extension example uses constructs tex does not have, so what is
        // frozen for it is tex failing to understand them -- not a claim about
        // texrs. It still has to RUN, which the test above checks.
        if extension_names.contains(&name) {
            continue;
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples")
            .join(&name);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
        let got = texrs::parity::subject(&src);
        if got != want {
            bad.push(format!("{name}\n  tex   : {want:?}\n  texrs : {got:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} example(s) no longer print what tex printed:\n\n{}",
        bad.len(),
        bad.join("\n\n")
    );
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
