//! Pages are drawn in parallel; the file must not depend on how many threads
//! drew them.
//!
//! A size opened on one page and closed on a later one is state a page
//! inherits from the pages before it, so the document below opens one that
//! runs across several page boundaries. Drawing each page from a fresh stack
//! would set the middle pages at the body size, which the assertion on the
//! drawn sizes catches; the comparison across thread counts catches any
//! dependence on scheduling.

use std::collections::BTreeSet;
use texrs::pdfread::Document;
use texrs::typeset::{size_span, to_pdf, Families, Layout, SIZE_POP, SIZE_PUSH};

fn document() -> String {
    let filler = "the quick brown fox jumps over the lazy dog and keeps running ".repeat(14);
    let mut text = String::new();
    for p in 0..260 {
        // Opened here, closed thirty paragraphs (several pages) on.
        if p == 3 {
            text.push_str(&format!("{SIZE_PUSH}7;9{SIZE_PUSH}"));
        }
        if p == 33 {
            text.push(SIZE_POP);
        }
        match p % 17 {
            5 if !(3..33).contains(&p) => text.push_str(&size_span(14.0, 17.0, &filler)),
            _ => text.push_str(&filler),
        }
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
fn pages_drawn_on_many_threads_match_the_serial_run() {
    std::env::set_var("SOURCE_DATE_EPOCH", "1700000000");
    let text = document();
    let serial = render(&text, 1);
    assert!(
        texrs::pdf_page_count(&serial) > 8,
        "the document must span enough pages to fan out"
    );
    // A page wholly inside the open span is set at its size (the folio at the
    // foot is the only text on it at the body size).
    let doc = Document::read(serial.clone()).expect("reads");
    assert!(
        sizes_on(&doc, 3).contains("7"),
        "the size stack must reach pages after the one that opened it"
    );
    for threads in [2, 8] {
        assert!(
            serial == render(&text, threads),
            "PDF differs at {threads} threads"
        );
    }
}
