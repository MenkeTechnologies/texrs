//! The cases in `tests/luatex_cases` against `luatex`: one or more primitives
//! each, named on the case's `% primitives:` line, read the way an INITEX
//! luatex with every extra primitive enabled reads them.
//!
//! Two comparisons, as for the TeX82 corpus (`tests/parity.rs` and
//! `tests/differential.rs`). The replay runs everywhere: what luatex printed
//! was frozen into `tests/data/luatex_cases_expected.txt` by
//! `cargo run --bin primitive-census -- --freeze`, and texrs must still print
//! it. The live one runs where luatex is installed, and checks that the frozen
//! file still says what luatex says -- a frozen file can only vouch for
//! texrs, never for itself.
//!
//! texrs runs each case as a LuaTeX (`-progname=luatex`'s personality: its
//! 65536 registers, its closing paren), since that is what luatex is.
//!
//! A case that diverges is allowed only if `tests/luatex_known_gaps.txt` names
//! it, and a named case that has started passing fails until the entry goes.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn frozen() -> Vec<(String, String)> {
    texrs::parity::thawed(include_str!("data/luatex_cases_expected.txt"))
}

fn known() -> Vec<String> {
    include_str!("luatex_known_gaps.txt")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

fn case_names() -> BTreeSet<String> {
    texrs::parity::cases_in(&repo().join("tests/luatex_cases"))
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
}

#[test]
fn every_case_is_frozen_and_names_what_it_exercises() {
    let recorded: BTreeSet<String> = frozen().into_iter().map(|(n, _)| n).collect();
    let cases = case_names();
    let unfrozen: Vec<&String> = cases.difference(&recorded).collect();
    assert!(
        unfrozen.is_empty(),
        "case(s) with no frozen expectation -- run `cargo run --bin primitive-census -- \
         --freeze`: {unfrozen:?}"
    );
    let orphaned: Vec<&String> = recorded.difference(&cases).collect();
    assert!(
        orphaned.is_empty(),
        "frozen block(s) for no case: {orphaned:?}"
    );
    // The census counts a primitive complete by this line; a case without one
    // is evidence about nothing it can name.
    for name in &cases {
        let src = std::fs::read_to_string(repo().join("tests/luatex_cases").join(name)).unwrap();
        assert!(
            !texrs::luatex_oracle::primitives_of_case(&src).is_empty(),
            "{name} has no `% primitives:` line"
        );
    }
    for gap in known() {
        assert!(
            cases.contains(&gap),
            "luatex_known_gaps.txt names no case: {gap}"
        );
    }
}

#[test]
fn every_case_still_prints_what_luatex_printed() {
    let known = known();
    let mut regressions = Vec::new();
    let mut healed = Vec::new();
    for (name, want) in frozen() {
        let src = std::fs::read_to_string(repo().join("tests/luatex_cases").join(&name))
            .expect("a frozen case exists");
        let got = texrs::luatex_oracle::subject(&src);
        match (got == want, known.contains(&name)) {
            (false, false) => {
                regressions.push(format!("{name}\n  luatex: {want:?}\n  texrs : {got:?}"))
            }
            (true, true) => healed.push(name),
            _ => {}
        }
    }
    assert!(
        regressions.is_empty() && healed.is_empty(),
        "{} unlisted divergence(s):\n{}\n\nnow passing, remove from tests/luatex_known_gaps.txt: {healed:?}",
        regressions.len(),
        regressions.join("\n")
    );
}

#[test]
fn the_frozen_file_still_says_what_luatex_says() {
    let program = match texrs::luatex_oracle::program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let mut stale = Vec::new();
    for (name, frozen) in frozen() {
        let src = std::fs::read_to_string(repo().join("tests/luatex_cases").join(&name)).unwrap();
        let live = texrs::luatex_oracle::reference(&program, &src);
        if live != frozen {
            stale.push(format!("{name}\n  frozen: {frozen:?}\n  luatex: {live:?}"));
        }
    }
    assert!(
        stale.is_empty(),
        "re-freeze with `cargo run --bin primitive-census -- --freeze`:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_command_table_is_luatexs() {
    // `src/luatex_primitives.rs` is what texrs knows about every primitive it
    // has no arm for; it is only worth that if it is still luatex's own.
    let program = match texrs::luatex_oracle::program() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let live = texrs::luatex_oracle::command_table(&program).expect("luatex's table");
    let committed: Vec<(String, String, bool)> = texrs::luatex_primitives::TABLE
        .iter()
        .map(|(n, c, e)| (n.to_string(), c.to_string(), *e))
        .collect();
    assert!(
        live == committed,
        "src/luatex_primitives.rs is stale -- run `bash scripts/primitive-census.sh`"
    );
}
