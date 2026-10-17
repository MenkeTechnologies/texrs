//! DVI pages are encoded in parallel and joined in order; the file must not
//! depend on how many threads encoded them.
//!
//! Each page's `bop` points at the one before it, so a page joined out of
//! order or with a stale back-pointer is a different file from the serial
//! run's. That serial run joins pages the same way, so the back-pointers
//! themselves are checked against a single writer by the unit test in
//! `src/dvi.rs`.
//!
//! Skipped, loudly, without `cmr10.tfm`: the DVI path needs a real font
//! metric file to set anything.

use texrs::dvi::Dvi;
use texrs::typeset::{to_dvi_chain, FontChain, Layout};

fn document() -> String {
    let filler = "the quick brown fox jumps over the lazy dog and keeps running ".repeat(14);
    (0..260)
        .map(|p| format!("{filler}paragraph {p}\n\n"))
        .collect()
}

fn render(text: &str, chain: &FontChain, threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    pool.install(|| to_dvi_chain(text, chain, &Layout::default()))
}

#[test]
fn pages_encoded_on_many_threads_match_the_serial_run() {
    let Ok(chain) = FontChain::load("cmr10", &["cmsy10", "cmmi10"]) else {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    };
    let text = document();
    let serial = render(&text, &chain, 1);
    let dvi = Dvi::parse(&serial).expect("parses");
    assert!(
        dvi.pages() > 8,
        "the document must span enough pages to fan out"
    );
    for threads in [2, 8] {
        assert!(
            serial == render(&text, &chain, threads),
            "DVI differs at {threads} threads"
        );
    }
}
