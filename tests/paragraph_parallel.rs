//! Paragraphs are broken in parallel; the file must not depend on how many
//! threads broke them.
//!
//! Each paragraph starts from state the paragraphs before it left: the
//! centring flag, the list depth and the face and size stacks. The document
//! below opens each of those in one paragraph and closes it thirty
//! paragraphs on, so a paragraph broken from a fresh state instead of the one
//! the serial walk reaches is centred, indented or measured differently, and
//! the comparison against the one-thread run (which walks the paragraphs in
//! turn, with no prescan) catches it.

use std::collections::BTreeSet;
use texrs::pdfread::Document;
use texrs::typeset::{
    indent_mark, to_pdf, Families, Layout, CENTRE, CENTRE_END, FACE_POP, FACE_PUSH, SIZE_POP,
    SIZE_PUSH,
};

fn document() -> String {
    let filler = "the quick brown fox jumps over the lazy dog and keeps running ".repeat(14);
    let mut text = String::new();
    for p in 0..240 {
        // Each region opens in one paragraph and closes thirty on.
        match p {
            10 => text.push(CENTRE),
            40 => text.push(CENTRE_END),
            60 => text.push_str(&indent_mark(2)),
            90 => text.push_str(&indent_mark(0)),
            110 => text.push_str(&format!("{FACE_PUSH}m")),
            140 => text.push(FACE_POP),
            // A size moves the breaker's stack only inside a word that also
            // carries a face marker, so the two are opened in one word.
            160 => text.push_str(&format!("{FACE_PUSH}b{SIZE_PUSH}7;9{SIZE_PUSH}")),
            190 => text.push_str(&format!("{SIZE_POP}{FACE_POP}")),
            _ => {}
        }
        text.push_str(&filler);
        text.push_str(&format!("paragraph {p}\n\n"));
    }
    text
}

fn render(text: &str, threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    pool.install(|| to_pdf(text, &Families::default(), &Layout::default(), None, None).bytes)
}

/// The sizes the page's `Tf` operators select.
fn sizes_on(doc: &Document, page: usize) -> BTreeSet<String> {
    let content = doc.content(&doc.pages()[page]).expect("content");
    let content = String::from_utf8_lossy(&content).into_owned();
    let words: Vec<&str> = content.split_whitespace().collect();
    words
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[1] == "Tf")
        .map(|(i, _)| words[i].to_string())
        .collect()
}

#[test]
fn paragraphs_broken_on_many_threads_match_the_serial_run() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let text = document();
    let serial = render(&text, 1);
    // The size opened at paragraph 160 reaches the pages after it: the last
    // page set at 7pt is well past the one that opened it.
    let doc = Document::read(serial.clone()).expect("reads");
    let small: Vec<usize> = (0..doc.pages().len())
        .filter(|&page| sizes_on(&doc, page).contains("7"))
        .collect();
    assert!(
        small.len() > 2,
        "the size must hold across paragraphs and pages: {small:?}"
    );
    for threads in [2, 8] {
        assert!(
            serial == render(&text, threads),
            "PDF differs at {threads} threads"
        );
    }
}
