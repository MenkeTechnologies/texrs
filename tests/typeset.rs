//! Typesetting: text and font metrics into DVI pages.
//!
//! texrs could read a document and say what its words were, and could read and
//! write DVI, and nothing joined the two -- `dvi::Writer` was never called
//! outside its own tests, so a book "ran" and produced no page.
//!
//! What is asserted here is that a real DVI comes out and that the arithmetic
//! is the font's rather than invented. What is NOT claimed is `tex.web`'s
//! typesetting: lines are broken first-fit, where tex minimises total badness
//! over a whole paragraph (§813), and there is no hyphenation, no glue, one
//! font and no maths.

use texrs::tfm::Tfm;
use texrs::typeset::{break_lines, find_font, to_dvi, Layout};

fn font() -> Tfm {
    let p = find_font("cmr10").expect("cmr10.tfm (is a TeX installation present?)");
    Tfm::open(&p).expect("read cmr10")
}

#[test]
fn a_dvi_file_is_produced_and_parses_as_one() {
    let f = font();
    let dvi = to_dvi("hello world", &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("texrs must read back what it wrote");
    assert_eq!(parsed.pages(), 1, "one line of text is one page");
}

#[test]
fn every_line_fits_the_measure() {
    // The whole job of line breaking. A line wider than the measure would run
    // off the page, and nothing downstream would catch it.
    let f = font();
    let layout = Layout::default();
    let text = "the quick brown fox jumps over the lazy dog ".repeat(80);
    for line in break_lines(&text, &f, &layout) {
        let w = f.width_of(&line) * layout.size;
        assert!(
            w <= layout.measure + 0.01,
            "line is {w:.1}pt against a {:.1}pt measure: {line:?}",
            layout.measure
        );
    }
}

#[test]
fn a_word_too_wide_to_fit_still_gets_a_line() {
    // First-fit has to make progress even when a single word exceeds the
    // measure, or it loops forever or drops the word.
    let f = font();
    let long = "x".repeat(400);
    let lines = break_lines(&long, &f, &Layout::default());
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], long, "the word is kept whole, not truncated");
}

#[test]
fn paragraphs_stay_in_order_though_they_are_broken_in_parallel() {
    // Broken with rayon across paragraphs. They are independent, but their
    // ORDER is the document: a book whose paragraphs arrived in completion
    // order would be a different book.
    let f = font();
    let text: String = (0..200)
        .map(|i| format!("para{i} some filler words here\n\n"))
        .collect();
    let lines = break_lines(&text, &f, &Layout::default());
    let firsts: Vec<usize> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("para"))
        .filter_map(|r| r.split_whitespace().next())
        .filter_map(|n| n.parse().ok())
        .collect();
    assert_eq!(firsts.len(), 200, "every paragraph is there");
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "and in order");
}

#[test]
fn a_long_document_becomes_more_than_one_page() {
    let f = font();
    let text = "word ".repeat(20_000);
    let dvi = to_dvi(&text, &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    assert!(parsed.pages() > 1, "got {} pages", parsed.pages());
}

#[test]
fn the_text_survives_the_round_trip_through_dvi() {
    // The characters that went in are the characters a reader gets back, which
    // is what makes the page the document rather than a plausible shape.
    let f = font();
    let dvi = to_dvi("typesetting works", &f, "cmr10", &Layout::default());
    let parsed = texrs::dvi::Dvi::parse(&dvi).expect("parse");
    let got = parsed.text();
    assert!(got.contains("typesetting"), "got {got:?}");
    assert!(got.contains("works"), "got {got:?}");
}
