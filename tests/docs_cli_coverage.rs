//! Coverage gate: `docs/reference.html` must name every option `--help` names.
//!
//! The binary, the zsh completion and the man page are already held to each
//! other by `tests/cli.rs`. The reference page is the fourth place a user meets
//! the flags, and it used to be a hand-written table of nine invocations against
//! a binary that accepts forty-one — the drift that list cannot avoid is the
//! reason the page is now generated from `cli::USAGE` instead.
//!
//! This test is what makes that generation load-bearing rather than incidental:
//! it fails if the page stops covering the grammar, whichever side moved.

use texrs::cli::USAGE;
use texrs::docs::{reference_html, usage_sections};

/// The option spellings the grammar lists, one per row, split on the `, ` that
/// separates a flag from its negation (`-file-line-error, -no-file-line-error`)
/// and truncated at the first space so `-X tfm FILE.tfm [C]` is looked for as
/// `-X tfm` rather than with its metavariables.
fn spellings() -> Vec<String> {
    let mut out = Vec::new();
    for section in usage_sections(USAGE) {
        for row in section.rows {
            for alt in row.option.split(", ") {
                let alt = alt.trim();
                if alt.is_empty() {
                    continue;
                }
                let head = if let Some(rest) = alt.strip_prefix("-X ") {
                    format!("-X {}", rest.split_whitespace().next().unwrap_or(""))
                } else {
                    alt.split(['=', ' ']).next().unwrap_or(alt).to_string()
                };
                out.push(head);
            }
        }
    }
    out
}

#[test]
fn the_grammar_parses_into_the_sections_help_prints() {
    let sections = usage_sections(USAGE);
    let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "TEX OPTIONS",
            "RUNNING",
            "LOOKING INSIDE",
            "EDITORS",
            "CACHE",
            "DOCUMENTS",
            "SYSTEM",
        ],
        "the section rules in cli::USAGE are not the ones the reference groups by"
    );
    for section in &sections {
        for row in &section.rows {
            assert!(
                !row.note.is_empty(),
                "{}: {} has no `//` note, so the reference would show a flag and \
                 no description",
                section.title,
                row.option
            );
        }
    }
}

#[test]
fn every_option_in_the_grammar_reaches_the_page() {
    let page = reference_html();
    let missing: Vec<String> = spellings()
        .into_iter()
        .filter(|s| !page.contains(s.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "{} option(s) in cli::USAGE do not appear in docs/reference.html:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn the_page_carries_a_section_for_each_group() {
    let page = reference_html();
    for section in usage_sections(USAGE) {
        let anchor = section
            .title
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");
        let id = format!("id=\"cli-{anchor}\"");
        assert!(
            page.contains(&id),
            "{} has no section on the page (looked for {id})",
            section.title
        );
    }
}
