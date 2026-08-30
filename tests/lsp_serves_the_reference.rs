//! Coverage gate: the reference manual is the language server's answers.
//!
//! `docs/reference.html` used to be rendered from `src/corpus.rs` and the
//! server used to answer from `src/corpus.rs`, which made them agree by
//! coincidence of a shared constant rather than by construction: a primitive
//! the server could not resolve still appeared in the manual, fully documented,
//! and nothing said so. Two of them were in exactly that state — `^^X` and the
//! backtick hovered as the generic banner.
//!
//! The page is now rendered from `lsp::served()`, which reads the completion
//! and hover responses. This test holds that arrangement up: every documented
//! primitive must be one the server actually serves, and what the page says
//! must be what the server said.

use std::collections::BTreeMap;

use texrs::corpus::CORPUS;
use texrs::docs::reference_html;
use texrs::lsp::{hover_chapter, served};

#[test]
fn the_server_resolves_every_documented_primitive() {
    let mut unresolved = Vec::new();
    for (name, chapter, ..) in CORPUS {
        match hover_chapter(name) {
            None => unresolved.push(format!("{name}: hover returns the banner")),
            Some(got) if got != *chapter => unresolved.push(format!(
                "{name}: hover says {got:?}, corpus says {chapter:?}"
            )),
            Some(_) => {}
        }
    }
    assert!(
        unresolved.is_empty(),
        "{} primitive(s) the manual documents are not resolved by the server, \
         so the page would claim what an editor cannot show:\n  {}",
        unresolved.len(),
        unresolved.join("\n  ")
    );
}

#[test]
fn what_the_server_serves_is_what_the_corpus_holds() {
    let served: BTreeMap<String, _> = served().into_iter().map(|e| (e.name.clone(), e)).collect();
    assert_eq!(
        served.len(),
        CORPUS.len(),
        "the server serves {} primitives where the corpus holds {}",
        served.len(),
        CORPUS.len()
    );
    for (name, chapter, doc, example) in CORPUS {
        let entry = served
            .get(*name)
            .unwrap_or_else(|| panic!("{name} is in the corpus and not served"));
        assert_eq!(&entry.chapter, chapter, "{name}: chapter");
        assert_eq!(&entry.doc, doc, "{name}: the completion item's detail");
        assert_eq!(
            &entry.example, example,
            "{name}: the example the completion documentation carries"
        );
    }
}

#[test]
fn every_served_primitive_reaches_the_page() {
    let page = reference_html();
    for entry in served() {
        assert!(
            !entry.chapter.is_empty(),
            "{}: served with no chapter, so the page has nowhere to put it",
            entry.name
        );
        // The page marks up backticks as <code> and escapes the markup
        // characters, so the detail is compared through its longest run of
        // plain text rather than whole -- still enough that a row carrying a
        // DIFFERENT primitive's detail would fail.
        let plain = entry
            .doc
            .split(|c| c == '`' || c == '&' || c == '<' || c == '>')
            .max_by_key(|s| s.len())
            .unwrap_or("")
            .trim();
        assert!(
            plain.len() >= 20,
            "{}: no plain-text run long enough to look for",
            entry.name
        );
        assert!(
            page.contains(plain),
            "{}: the server's detail line is not on the page (looked for {plain:?})",
            entry.name
        );
    }
}
