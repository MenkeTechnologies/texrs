//! Formulas on the list path, against `tex`'s own DVI file.
//!
//! Each document in `tests/math_cases` sets formulas in paragraphs and in
//! displays; `--dvi` must write the file `tex` writes for it, byte for byte.
//! Skipped without a `tex` on PATH or without `cmr10.tfm`, as
//! `tests/dvi_parity.rs` is.

use std::path::{Path, PathBuf};
use texrs::dvi_parity::{self, Rung};

fn cases() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/math_cases");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("tests/math_cases")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    v.sort();
    v
}

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
    let mut short = Vec::new();
    for case in cases() {
        let reference = dvi_parity::reference(&oracle, &case);
        let subject = dvi_parity::subject(&case);
        let (rung, detail) = dvi_parity::verdict(reference.as_ref(), subject.as_ref());
        if rung != Rung::Bytes {
            short.push(format!("{}: {} ({detail})", case.display(), rung.name()));
        }
    }
    assert!(short.is_empty(), "not tex's DVI: {short:#?}");
}
