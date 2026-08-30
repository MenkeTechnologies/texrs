//! Coverage gate: the reference's generated tables must keep matching the
//! engine they are generated from.
//!
//! Three sections of `docs/reference.html` are derived rather than written —
//! the category table from `CatTable::new()`, the divergence list from
//! `tests/known_gaps.txt`, the environment table from a list in `docs.rs`. The
//! first two cannot drift by construction; this test is what proves the
//! construction still holds, and the third has no source to derive from, so it
//! is checked against the `env::var` calls in `src/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use texrs::catcode::Cat;
use texrs::docs::{known_gaps, reference_html};

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Every `.rs` under `src/`, so a variable introduced in a new module is still
/// seen.
fn sources() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                if let Ok(text) = fs::read_to_string(&path) {
                    out.push(text);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&manifest("src"), &mut out);
    out
}

#[test]
fn the_category_table_has_a_row_for_every_category() {
    let page = reference_html();
    for cat in Cat::ALL {
        let row = format!("<td><code>{}</code></td><td>{}</td>", cat as u8, cat.name());
        assert!(
            page.contains(&row),
            "category {} ({}) has no row in the reference table",
            cat as u8,
            cat.name()
        );
    }
}

#[test]
fn the_category_table_states_initex_and_not_plain() {
    let page = reference_html();
    // The escape character is INITEX's; the brace is plain.tex's. Getting this
    // backwards is the single most misleading thing the table could say, so it
    // is asserted rather than trusted.
    let escape_row = format!("<td><code>0</code></td><td>{}</td>", Cat::Escape.name());
    let begin_row = format!("<td><code>1</code></td><td>{}</td>", Cat::BeginGroup.name());
    let escape_at = page.find(&escape_row).expect("no escape row");
    let begin_at = page.find(&begin_row).expect("no begin-group row");
    let escape_cell = &page[escape_at..escape_at + 200];
    let begin_cell = &page[begin_at..begin_at + 200];
    assert!(
        escape_cell.contains("\\"),
        "INITEX puts the backslash in category 0 and the table does not say so"
    );
    assert!(
        begin_cell.contains("&mdash;"),
        "category 1 is empty under INITEX -- the brace is plain.tex's, not the \
         engine's, and a table that shows it here teaches the wrong thing"
    );
}

#[test]
fn every_written_down_gap_reaches_the_page() {
    let text = fs::read_to_string(manifest("tests/known_gaps.txt")).unwrap();
    let entries = known_gaps(&text);
    // An independent count of the file, so a parser that silently dropped an
    // entry would not also silently lower the expectation.
    let cases = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count();
    assert_eq!(
        entries.len(),
        cases,
        "the gap parser found {} entries in a file with {cases} case lines",
        entries.len()
    );
    let page = reference_html();
    for (case, reason) in entries {
        assert!(
            page.contains(&case),
            "{case} is a written-down divergence and is not in the reference"
        );
        assert!(
            !reason.trim().is_empty(),
            "{case} has no reason, so the reference would show a filename and \
             nothing else"
        );
    }
}

#[test]
fn every_texrs_environment_variable_is_documented() {
    let page = reference_html();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for text in sources() {
        let mut rest = text.as_str();
        while let Some(at) = rest.find("\"TEXRS_") {
            rest = &rest[at + 1..];
            // Stop at the end of the identifier, not at the closing quote: the
            // same name also appears inside format strings such as
            // `"TEXRS_CACHE={value} turns it off"`.
            let end = rest
                .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(rest.len());
            names.insert(rest[..end].to_string());
        }
    }
    assert!(!names.is_empty(), "found no TEXRS_* variables to check");
    let missing: Vec<&String> = names.iter().filter(|n| !page.contains(n.as_str())).collect();
    assert!(
        missing.is_empty(),
        "{} environment variable(s) the engine reads are not in the reference:\n  {}",
        missing.len(),
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}
