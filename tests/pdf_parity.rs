//! The PDF parity floor: no document may agree with LuaTeX less than it did.
//!
//! Byte-identical output is the goal and nothing reaches it yet, so this does
//! not assert agreement — it asserts that agreement never goes backwards.
//! `tests/pdf_floor.txt` records the rung each document reached when it was
//! last measured; a change that drops one fails here, and a change that climbs
//! one is re-recorded with `cargo run --bin pdf-parity -- --record`.
//!
//! Skipped, loudly, without `luatex` (the oracle) or poppler's `pdfinfo` and
//! `pdftotext` (which read the two files), because a harness that cannot see
//! the difference must say so rather than pass.

use std::path::{Path, PathBuf};
use texrs::pdf_parity::{self, Rung};

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn floor() -> Vec<(String, Rung)> {
    std::fs::read_to_string(manifest("tests/pdf_floor.txt"))
        .expect("the floor file")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (rung, name) = l.split_once(' ')?;
            Some((name.trim().to_string(), Rung::parse(rung)?))
        })
        .collect()
}

#[test]
fn no_document_agrees_with_luatex_less_than_it_did() {
    let Some(oracle) = pdf_parity::oracle() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    if std::process::Command::new("pdfinfo")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("skipping: poppler's pdfinfo is not installed");
        return;
    }

    let mut dropped = Vec::new();
    for (name, was) in floor() {
        let case = manifest("tests/pdf_cases").join(&name);
        let reference = pdf_parity::reference(&oracle, &case);
        let subject = pdf_parity::subject(&case);
        let (now, detail) = pdf_parity::verdict(reference.as_ref(), subject.as_ref());
        if now < was {
            dropped.push(format!(
                "{name}: was {}, now {} ({detail})",
                was.name(),
                now.name()
            ));
        }
    }
    assert!(
        dropped.is_empty(),
        "these documents match LuaTeX less closely than they did: {dropped:#?}"
    );
}

/// The floor file must name documents that exist, or it is recording nothing.
#[test]
fn every_recorded_document_is_in_the_corpus() {
    for (name, _) in floor() {
        assert!(
            manifest("tests/pdf_cases").join(&name).is_file(),
            "tests/pdf_floor.txt names {name}, which is not in tests/pdf_cases"
        );
    }
}
