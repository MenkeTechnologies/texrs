//! The language server answers from the same places the engine does.
//!
//! These call the server's helpers directly rather than standing up a stdio
//! connection: what is worth pinning is that completion comes from the corpus,
//! that hover finds a control sequence the way TeX's own scanner would, and
//! that a diagnostic lands on the line the error is actually on. The JSON-RPC
//! plumbing around them is `lsp-server`'s, and testing it would be testing that
//! crate.

use texrs::corpus::CORPUS;
use texrs::lsp::{completion_items, diagnostics, hover_markdown};

#[test]
fn completion_offers_every_documented_primitive() {
    let items = completion_items();
    assert_eq!(
        items.len(),
        CORPUS.len(),
        "completion and the corpus have drifted apart"
    );
    for (name, _chapter, doc, _example) in CORPUS {
        let item = items
            .iter()
            .find(|i| i.label == *name)
            .unwrap_or_else(|| panic!("{name} is in the corpus but not offered"));
        assert_eq!(
            item.detail.as_deref(),
            Some(*doc),
            "{name}: the completion detail is not the corpus doc line"
        );
    }
}

#[test]
fn hover_reads_a_control_word_from_anywhere_inside_it() {
    let text = "\\catcode`\\{=1\n\\message{\\the\\count1}\n";
    // Every column of `\message` must resolve to the same entry: an editor
    // hovers wherever the pointer happens to be, not at the backslash.
    for col in 0..8 {
        let md = hover_markdown(text, 1, col);
        assert!(
            md.contains("**`\\message`**"),
            "column {col} of line 2 hovered to: {md}"
        );
    }
}

#[test]
fn hover_stops_at_the_control_symbol_boundary() {
    // `\{` is a control SYMBOL: one character, even though letters follow it.
    let md = hover_markdown("\\catcode`\\{=1\n", 0, 1);
    assert!(
        md.contains("**`\\catcode`**"),
        "expected the catcode entry, got: {md}"
    );
}

#[test]
fn hover_off_a_primitive_falls_back_to_the_banner() {
    let md = hover_markdown("HELLO WORLD\n", 0, 3);
    assert!(md.starts_with("**texrs**"), "unexpected hover: {md}");
}

#[test]
fn a_clean_document_has_no_diagnostics() {
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{HELLO}\n\\end\n";
    assert!(diagnostics(src).is_empty());
}

#[test]
fn a_broken_document_reports_on_the_line_it_broke() {
    // The undefined `\nope` is on line 3 (0-based 2), and the diagnostic has to
    // land there rather than at the top of the file.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{FINE}\n\\nope\n\\end\n";
    let ds = diagnostics(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic: {ds:?}");
    assert_eq!(
        ds[0].range.start.line, 2,
        "diagnostic landed on the wrong line: {:?}",
        ds[0]
    );
    assert!(
        ds[0].message.starts_with("! "),
        "a diagnostic should read like a tex error: {}",
        ds[0].message
    );
}

#[test]
fn an_empty_document_is_not_an_error() {
    assert!(diagnostics("").is_empty());
    assert!(diagnostics("   \n\n").is_empty());
}
