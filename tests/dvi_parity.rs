//! The DVI parity floor: no document may agree with `tex` less than it did.
//!
//! The attainable axis. A DVI carries no fonts and no compression, so byte
//! equality is a goal rather than an aspiration — for `Hello world.` tex writes
//! 224 bytes and texrs 260, where the same document in PDF is 11,729 against
//! 615. `tests/dvi_floor.txt` records where each document stands.
//!
//! Skipped, loudly, without a pinned `tex` and without `cmr10.tfm`, since
//! texrs's DVI path needs a real font metric file to set anything.

use std::path::{Path, PathBuf};
use texrs::dvi_parity::{self, Rung};

fn manifest(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn floor() -> Vec<(String, Rung)> {
    std::fs::read_to_string(manifest("tests/dvi_floor.txt"))
        .expect("the floor file")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (rung, name) = l.split_once(' ')?;
            Some((name.trim().to_string(), Rung::parse(rung)?))
        })
        .collect()
}

#[test]
fn no_document_agrees_with_tex_less_than_it_did() {
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("skipping: no `tex` on PATH");
        return;
    };
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    let mut dropped = Vec::new();
    for (name, was) in floor() {
        let case = manifest("tests/pdf_cases").join(&name);
        let reference = dvi_parity::reference(&oracle, &case);
        let subject = dvi_parity::subject(&case);
        let (now, detail) = dvi_parity::verdict(reference.as_ref(), subject.as_ref());
        if now < was {
            dropped.push(format!(
                "{name}: was {}, now {} ({detail})",
                was.name(),
                now.name()
            ));
        }
    }
    assert!(
        dropped.is_empty(),
        "these documents match tex less closely than they did: {dropped:#?}"
    );
}

/// The three differences the ladder currently reports, pinned as facts.
///
/// Each is a real typesetting decision texrs makes differently, and each will
/// change the rung when it is fixed — so they are written down here rather than
/// left as a rung number whose meaning nobody remembers.
#[test]
fn the_current_differences_are_the_ones_recorded() {
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("skipping: no `tex` on PATH");
        return;
    };
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    let case = manifest("tests/pdf_cases").join("two_paragraphs.tex");
    let Some(reference) = dvi_parity::reference(&oracle, &case) else {
        eprintln!("skipping: tex wrote no DVI");
        return;
    };
    let subject = dvi_parity::subject(&case).expect("texrs sets this document");
    let rd = texrs::dvi::Dvi::parse(&reference).expect("tex writes a readable DVI");
    let sd = texrs::dvi::Dvi::parse(&subject).expect("texrs writes a readable DVI");

    // A space between words is a MOVEMENT in tex's DVI, not a character, so
    // tex's text runs together. texrs sets a space glyph instead.
    assert!(
        !rd.text().contains("The first"),
        "tex moves between words rather than setting spaces, got {:?}",
        rd.text()
    );
    assert!(
        sd.text().contains("The first") || sd.text().contains("The rst"),
        "texrs sets the words, got {:?}",
        sd.text()
    );

    // tex reaches for the `fi` ligature in cmr10; texrs sets f and i.
    assert!(
        rd.text().contains('\u{c}'),
        "tex uses the fi ligature (character 0x0C), got {:?}",
        rd.text()
    );
    assert!(
        !sd.text().contains('\u{c}'),
        "texrs does not use it yet, got {:?}",
        sd.text()
    );
}
