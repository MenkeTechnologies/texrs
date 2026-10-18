//! A plain document's pages from the list path, drawn by LuaTeX's backend
//! (`texrs::run_tex_pdf`), against what `luatex` writes for the documents of
//! `tests/pdf_cases`: every one matches luatex's file in its pages, its text
//! and its lines, and differs, if at all, only in the font programs, which
//! are the subsetter's. Skipped without `luatex`.

use std::path::Path;
use texrs::pdf_parity::{self, Rung};

#[test]
fn every_page_matches_luatexs_but_for_the_font_programs() {
    let Some(oracle) = pdf_parity::oracle() else {
        eprintln!("skipping: no `luatex` on PATH");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdf_cases");
    let mut cases: Vec<_> = std::fs::read_dir(dir)
        .expect("tests/pdf_cases")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "tex"))
        .collect();
    cases.sort();
    let mut short = Vec::new();
    for case in cases {
        let reference = pdf_parity::reference(&oracle, &case);
        let src = std::fs::read_to_string(&case).expect("read");
        let pdf = texrs::run_tex_pdf(&src).ok().and_then(|(_, pdf)| pdf);
        let (rung, detail) = pdf_parity::verdict(reference.as_ref(), pdf.as_ref());
        if rung < Rung::Fonts {
            short.push(format!("{}: {} ({detail})", case.display(), rung.name()));
        }
    }
    assert!(short.is_empty(), "short of luatex's pages: {short:#?}");
}
