//! Coverage gate: `src/corpus.rs` must stay pinned to the engine's own dispatch
//! tables, in BOTH directions.
//!
//! `docs/reference.html` is generated from the corpus and the language server
//! answers completion and hover from it, so a primitive the engine gained and
//! the corpus never heard of is invisible to both — and a corpus entry the
//! engine dropped is documentation for something that no longer runs.
//!
//! The dispatch tables are the source of truth, and they are Rust `match` arms
//! over string literals, so this test reads the sources with `include_str!` and
//! lifts the literals directly rather than comparing against a hand-maintained
//! list that would drift the same way.

use std::collections::BTreeSet;

use texrs::corpus::{CHAPTERS, CORPUS};

/// The primitives named in `expand.rs` and `lower.rs` dispatch, minus the ones
/// that are not control sequences a document writes.
fn dispatched() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = DISPATCHED_BY_CONSTANT
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for src in [
        include_str!("../src/expand.rs"),
        include_str!("../src/lower.rs"),
    ] {
        for line in src.lines() {
            let line = line.trim();
            // A dispatch arm: `"name" => ...`, `"a" | "b" => ...`, or a bare
            // `"name",` inside the CONDITIONALS table.
            let is_arm = line.contains("=>") || line.ends_with("\",");
            if !is_arm {
                continue;
            }
            let head = line.split("=>").next().unwrap_or(line);
            for part in head.split('|') {
                let part = part.trim().trim_end_matches(',');
                let Some(inner) = part.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
                    continue;
                };
                // `\u{0}endgroup` is lower.rs's internal stop marker, not a
                // primitive anyone can write.
                if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_alphabetic()) {
                    continue;
                }
                names.insert(inner.to_string());
            }
        }
    }
    names
}

/// The control sequences the corpus documents, without the leading backslash.
fn documented() -> BTreeSet<String> {
    CORPUS
        .iter()
        .filter_map(|(name, ..)| name.strip_prefix('\\'))
        .map(str::to_string)
        .collect()
}

/// Dispatch arms that are not primitives a document can write: internal markers,
/// and the `Arith` selector strings `lower.rs` re-matches on.
const NOT_PRIMITIVES: &[&str] = &["endgroup\u{0}"];

/// Documented primitives that have no dispatch arm of their own because another
/// primitive's scanner consumes them. They are still written in documents, so
/// they belong in the corpus.
///
/// `rust` is the block keyword: the desugarer rewrites it away before the mouth
/// ever runs, so the engine never dispatches on it — and a document still
/// writes it.
const CONSUMED_BY_A_SCANNER: &[&str] = &[
    "endcsname",
    "rust",
    // `\proceed` is substituted while an `around` advice is woven, so it is
    // consumed by the weave rather than dispatched — and a document writes it,
    // inside every around handler.
    "proceed",
];

/// Primitives whose dispatch arm matches a CONSTANT rather than a string
/// literal, so the scanner below cannot see them. The FFI names live in
/// `src/rust_ffi.rs` because the desugarer writes them and the lowerer reads
/// them, and one spelling in one place is what keeps those two agreeing.
const DISPATCHED_BY_CONSTANT: &[&str] = &[
    texrs::rust_ffi::COMPILE_CS,
    texrs::rust_ffi::CALL_CS,
    texrs::rust_ffi::END_CS,
];

#[test]
fn every_dispatched_primitive_is_documented() {
    let documented = documented();
    let missing: Vec<String> = dispatched()
        .into_iter()
        .filter(|n| !documented.contains(n) && !NOT_PRIMITIVES.contains(&n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "{} primitive(s) the engine dispatches are not in src/corpus.rs, so they \
         are missing from editor completion and docs/reference.html:\n  \\{}",
        missing.len(),
        missing.join("\n  \\")
    );
}

#[test]
fn every_documented_primitive_is_dispatched() {
    let dispatched = dispatched();
    let stale: Vec<String> = documented()
        .into_iter()
        .filter(|n| !dispatched.contains(n) && !CONSUMED_BY_A_SCANNER.contains(&n.as_str()))
        .collect();
    assert!(
        stale.is_empty(),
        "{} corpus entr(y/ies) name a control sequence the engine no longer \
         dispatches:\n  \\{}",
        stale.len(),
        stale.join("\n  \\")
    );
}

#[test]
fn every_entry_is_complete_and_in_a_known_chapter() {
    for (name, chapter, doc, example) in CORPUS {
        assert!(!name.is_empty(), "an entry has no name");
        assert!(
            CHAPTERS.contains(chapter),
            "{name}: chapter {chapter:?} is not in CHAPTERS, so the reference \
             would drop the entry"
        );
        assert!(
            doc.len() > 30,
            "{name}: the doc line is too short to be a description"
        );
        assert!(
            !example.is_empty(),
            "{name}: no syntax line -- hover would show a name and nothing else"
        );
    }
}

#[test]
fn no_duplicate_entries() {
    let mut seen = BTreeSet::new();
    for (name, ..) in CORPUS {
        assert!(seen.insert(*name), "{name} appears twice in the corpus");
    }
}
