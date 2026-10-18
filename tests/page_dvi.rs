//! Pages on the list path, against `tex`'s own DVI file: plain.tex's output
//! routine with running heads read from `\topmark`, `\firstmark` and
//! `\botmark`, footnotes and insertions, and `\vsplit`. Each document in
//! `tests/page_cases` must come out as `tex` writes it, byte for byte.
//! Skipped without a `tex` on PATH or without `cmr10.tfm`.

use std::path::Path;
use texrs::dvi_parity;

#[test]
fn every_page_document_is_texs_dvi_byte_for_byte() {
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("skipping: no `tex` on PATH");
        return;
    };
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/page_cases");
    let short = dvi_parity::short_of_bytes(&oracle, &dir);
    assert!(short.is_empty(), "not tex's DVI: {short:#?}");
}
