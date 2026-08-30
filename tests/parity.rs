//! CI-safe replay of the parity corpus: no TeX installation needed.
//!
//! `tests/differential.rs` asks the real `tex` what each case prints, which is
//! the honest comparison and the reason nothing here is a hand-written
//! expectation. But CI has no TeX Live, so that test skips there — every push
//! has been merging a corpus nobody verified except on a machine with tex.
//!
//! This closes that: `cargo run --bin parity -- --freeze` records what the
//! oracle said into `tests/data/parity_expected.txt`, reviewed in the diff like
//! any other expectation, and this replays it anywhere.
//!
//! The two are not redundant, and the difference is worth stating because it
//! decides what a failure here means. A frozen file can only say "texrs still
//! prints what tex printed when this was frozen". Only running tex says "and
//! that is still what tex prints". So: this catches a regression in texrs, and
//! `differential.rs` catches a wrong belief about tex.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn frozen() -> Vec<(String, String)> {
    texrs::parity::thawed(include_str!("data/parity_expected.txt"))
}

#[test]
fn every_case_is_frozen_and_every_frozen_case_exists() {
    // A case added without re-freezing would otherwise be silently unchecked
    // here, and a frozen block whose case was deleted is an expectation about
    // nothing.
    let cases: BTreeSet<String> = texrs::parity::cases_in(&repo().join("tests/cases"))
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let recorded: BTreeSet<String> = frozen().into_iter().map(|(n, _)| n).collect();

    let unfrozen: Vec<&String> = cases.difference(&recorded).collect();
    assert!(
        unfrozen.is_empty(),
        "case(s) with no frozen expectation -- run `cargo run --bin parity -- \
         --freeze`: {unfrozen:?}"
    );
    let orphaned: Vec<&String> = recorded.difference(&cases).collect();
    assert!(
        orphaned.is_empty(),
        "frozen expectation(s) for case(s) that no longer exist: {orphaned:?}"
    );
}

#[test]
fn the_corpus_still_prints_what_tex_printed() {
    let known = texrs::parity::known_gaps(&repo());
    let mut regressions = Vec::new();
    let mut healed = Vec::new();

    for (name, want) in frozen() {
        let src = std::fs::read_to_string(repo().join("tests/cases").join(&name))
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        let got = texrs::parity::subject(&src);
        let listed = known.contains(&name);
        match (got == want, listed) {
            // Agrees with tex and is not listed as a gap: as it should be.
            (true, false) => {}
            // Agrees with tex while listed as a gap: the list is stale, which
            // is a failure for the same reason it is in differential.rs --
            // removing the entry is part of the fix.
            (true, true) => healed.push(name),
            // Differs, and the gap is written down: expected.
            (false, true) => {}
            (false, false) => {
                regressions.push(format!("{name}\n  tex   : {want:?}\n  texrs : {got:?}"))
            }
        }
    }

    assert!(
        regressions.is_empty() && healed.is_empty(),
        "{} regression(s), {} stale known-gap entr(y/ies)\n\n{}{}",
        regressions.len(),
        healed.len(),
        regressions.join("\n\n"),
        match healed.is_empty() {
            true => String::new(),
            false => format!(
                "\n\nthese now match tex and must come out of \
                 tests/known_gaps.txt: {}",
                healed.join(", ")
            ),
        }
    );
}

#[test]
fn the_frozen_file_names_the_engine_it_came_from() {
    // A frozen file is only as good as the engine that produced it, and the
    // version is what lets a reader tell whether a diff is texrs changing or
    // the oracle changing.
    let text = include_str!("data/parity_expected.txt");
    let pinned = texrs::parity::pinned_version(&repo()).expect("BUGS.md pins a version");
    assert!(
        text.lines()
            .next()
            .is_some_and(|l| l.contains(&format!("tex {pinned}"))),
        "the frozen file does not say it came from tex {pinned}"
    );
}
