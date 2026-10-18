//! Formulas on the list path, against `tex`'s own DVI file.
//!
//! Each document in `tests/math_cases` sets formulas in paragraphs and in
//! displays; `--dvi` must write the file `tex` writes for it, byte for byte.
//! Skipped without a `tex` on PATH or without `cmr10.tfm`, as
//! `tests/dvi_parity.rs` is.

use std::path::Path;
use texrs::dvi_parity;

#[test]
fn every_formula_document_is_texs_dvi_byte_for_byte() {
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("skipping: no `tex` on PATH");
        return;
    };
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/math_cases");
    let short = dvi_parity::short_of_bytes(&oracle, &dir);
    assert!(short.is_empty(), "not tex's DVI: {short:#?}");
}
