//! The DVI parity floor: no document may agree with `tex` less than it did.
//!
//! The attainable axis. A DVI carries no fonts and no compression, so byte
//! equality is a goal rather than an aspiration — for `Hello world.` tex writes
//! 224 bytes and texrs 192, where the same document in PDF is 11,729 from luatex
//! against 15,435. `tests/dvi_floor.txt` records where each document stands.
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

/// The two differences the ladder currently reports, pinned as facts.
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
    // neither engine's text has spaces in it. texrs set a space GLYPH until the
    // interword space became glue (tex.web 625, 658); this pinned that
    // divergence, and now pins its absence.
    assert!(
        !rd.text().contains("The first"),
        "tex moves between words rather than setting spaces, got {:?}",
        rd.text()
    );
    assert!(
        !sd.text().contains("The first"),
        "texrs moves between words too, got {:?}",
        sd.text()
    );

    // tex reaches for the `fi` ligature in cmr10, and so does texrs now that
    // the `.tfm`'s ligature program is run over each word (tex.web 906-911,
    // 1034-1040). This assertion used to say texrs did NOT -- that was the
    // divergence, and this is the same fact pinned from the other side.
    assert!(
        rd.text().contains('\u{c}'),
        "tex uses the fi ligature (character 0x0C), got {:?}",
        rd.text()
    );
    assert!(
        sd.text().contains('\u{c}'),
        "texrs uses it too, got {:?}",
        sd.text()
    );
    // And the whole of the page's text agrees, character for character, which
    // is what the ladder calls TEXT. `The first paragraph` reads
    // `The\u{c}rstparagraph` in both files: no space glyphs, one ligature.
    assert_eq!(
        rd.text(),
        sd.text(),
        "the two engines set the same characters"
    );
}

/// The ligature program reaches the file, in the two shapes cmr10 has: a
/// ligature that REPLACES two characters with one, and a kern that moves
/// between two it leaves alone.
///
/// The ladder says STRUCTURE, which is one number for a page. This says what
/// bytes are in the file, so a regression names itself.
#[test]
fn a_word_is_shipped_with_its_ligatures_and_its_kerns() {
    let Some(path) = texrs::typeset::find_font("cmr10") else {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    };
    let font = texrs::tfm::Tfm::open(&path).expect("cmr10 reads");
    let layout = texrs::typeset::Layout::default();

    // `office` is `o`, the ffi ligature (0o16), `c`, `e`: cmr10 turns f+f into
    // 0o13 and then 0o13+i into 0o16, so three characters become one.
    let dvi = texrs::typeset::to_dvi("office", &font, "cmr10", &layout);
    let text = texrs::dvi::Dvi::parse(&dvi).expect("parse").text();
    assert!(
        text.contains("o\u{e}ce"),
        "office is set with the ffi ligature at 0o16: {text:?}"
    );

    // `AV` kerns by -0.111112 design-size units, which at 10pt is
    // -0.111112 * 10 * 65536 sp. A DVI carries that as a rightward movement
    // of a negative amount between the two characters.
    let dvi = texrs::typeset::to_dvi("AV", &font, "cmr10", &layout);
    let kern = (-0.111112 * layout.size * 65536.0) as i32;
    // 142+3 is `right3`, and -72829sp is the widest of the four that fits.
    let wanted = [145u8, (kern >> 16) as u8, (kern >> 8) as u8, kern as u8];
    assert!(
        dvi.windows(4).any(|w| w == wanted),
        "the A/V kern is a right3 of {kern}sp: {:02x?}",
        &dvi[..dvi.len().min(120)]
    );
}

/// The round trip: a file tex wrote must survive texrs's reader and writer at
/// least as well as it did.
///
/// This asks nothing of the typesetter — only that what was read can be written
/// back. Nothing reaches IDENTICAL yet, and the two reasons are worth naming:
/// six of the nine documents come out LONGER, because the writer does not
/// choose the compact operand widths tex chose, and the other three come out
/// the same length with the font checksum zeroed and the postamble's maximum
/// page width recomputed differently.
#[test]
fn no_file_survives_the_round_trip_less_well_than_it_did() {
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("skipping: no `tex` on PATH");
        return;
    };
    let floor: Vec<(String, texrs::dvi_parity::Trip)> =
        std::fs::read_to_string(manifest("tests/dvi_trip_floor.txt"))
            .expect("the round-trip floor")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| {
                let (trip, name) = l.split_once(' ')?;
                Some((
                    name.trim().to_string(),
                    texrs::dvi_parity::Trip::parse(trip)?,
                ))
            })
            .collect();
    assert!(!floor.is_empty(), "the floor file records nothing");

    let mut dropped = Vec::new();
    for (name, was) in floor {
        let case = manifest("tests/pdf_cases").join(&name);
        let Some(dvi) = dvi_parity::reference(&oracle, &case) else {
            continue;
        };
        let (now, detail) = dvi_parity::trip_verdict(&dvi);
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
        "these files survive the round trip less well than they did: {dropped:#?}"
    );
}

/// The main loop's lookahead (§1038-§1039) runs over EVERY character of a run,
/// so the ligature/kern program of a letter is consulted with the punctuation
/// after it as its right neighbour, whatever `\sfcode` that punctuation
/// carries. cmr10 kerns `y` against `.` and `,` by -0.083334 design units
/// (`tftopl`: `KRN O 56 R -0.083334`), and `P` shares that program.
///
/// The lowerer tells the program a character's space factor code the first
/// time it differs from INITEX's, and it used to END the character run to do
/// so -- the letter before the first `.` or `,` of a document was set against
/// the right boundary and lost its kern, once, while every later `y.` kept it.
/// What tex 3.141592653 prints for each box is the fixture; the kern is in
/// every one of them.
#[test]
fn a_kern_before_a_punctuation_mark_survives_its_space_factor_code() {
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    let src = "\\showboxdepth=100 \\showboxbreadth=1000 \\tracingonline=1\n\
               \\setbox0\\hbox{ly.}\\showbox0\n\
               \\setbox0\\hbox{y,}\\showbox0\n\
               \\setbox0\\hbox{P.}\\showbox0\n\
               \\setbox0\\hbox{y. x}\\showbox0\n\
               \\setbox0\\hbox{Py, A}\\showbox0\n\
               \\end\n";
    let got = texrs::run_messages_tex(src).expect("the document runs");
    for want in [
        // The first `.` of the document: its code (3000 in plain) had not been
        // told yet, and the kern before it is the one that went missing.
        "\\hbox(6.94444+1.94444)x10.00003.\\tenrm l.\\tenrm y.\\kern-0.83334.\\tenrm .",
        // The same for the first `,` (1250).
        "\\hbox(4.30554+1.94444)x7.22224.\\tenrm y.\\kern-0.83334.\\tenrm ,",
        // `P`'s program runs into `y`'s (§545: one program, two labels).
        "\\hbox(6.83331+0.0)x8.75002.\\tenrm P.\\kern-0.83334.\\tenrm .",
        // The code still reaches the space factor: the space after `.` is
        // set with `\sfcode`.=3000`'s stretch (§1044), kern intact.
        "\\hbox(4.30554+1.94444)x16.94449.\\tenrm y.\\kern-0.83334.\\tenrm ..\
         \\glue 4.44444 plus 4.99997 minus 0.37036.\\tenrm x",
        // ...and after `,`, with 1250's.
        "\\hbox(6.83331+1.94444)x24.86116.\\tenrm P.\\tenrm y.\\kern-0.83334.\\tenrm ,.\
         \\glue 3.33333 plus 2.08331 minus 0.88889.\\tenrm A",
    ] {
        assert!(got.contains(want), "missing {want:?} in {got:?}");
    }
}

/// §897 records the character that ends a word (`hyf_bchar:=character(s)`)
/// BEFORE testing whether it is a letter, so that when the word is rebuilt
/// after hyphenation its last letter is reconstituted against that character
/// (§903's `bchar`) and the kern or ligature between them comes back. The
/// hyphenator tested the letter first, so a hyphenated word was set against
/// the font's boundary instead and `electroencephalographically.` lost the
/// kern before its full stop -- in the node list, and in the DVI tex writes
/// as `right3 -54614`.
#[test]
fn a_hyphenated_word_keeps_the_kern_before_the_punctuation_after_it() {
    if texrs::typeset::find_font("cmr10").is_none() {
        eprintln!("skipping: no cmr10.tfm to set with");
        return;
    }
    // `\pretolerance=-1` forces the second pass, which is the one that
    // hyphenates (§863); `\showbox` then shows the rebuilt word.
    let par = "\\hsize=200pt \\tolerance=10000 \\pretolerance=-1 \
               \\noindent a electroencephalographically.\\par";
    let src = format!(
        "\\showboxdepth=100 \\showboxbreadth=1000 \\tracingonline=1\n\
         \\setbox0\\vbox{{{par}}}\\showbox0\n\\end\n"
    );
    let got = texrs::run_messages_tex(&src).expect("the document runs");
    assert!(got.contains("\\discretionary"), "the word was not hyphenated: {got:?}");
    assert!(
        got.contains("\\tenrm y..\\kern-0.83334..\\tenrm ."),
        "the kern before the full stop is gone: {got:?}"
    );
    // tex's line, with the kern in it: 200pt less the word is the fil glue.
    assert!(
        got.contains("\\hbox(6.94444+1.94444)x200.0, glue set 69.6664fil"),
        "the line is not set as tex sets it: {got:?}"
    );

    // The same paragraph as a page, read back out of the DVI: a `right3` of
    // -54614sp (-0.083334 * 10pt, as tex rounds it) between `y` and `.`.
    let (_, dvi) = texrs::run_tex_dvi(&format!("{par}\n\\end\n")).expect("the document runs");
    let dvi = dvi.expect("a page was shipped");
    let kern: i32 = -54614;
    let wanted = [145u8, (kern >> 16) as u8, (kern >> 8) as u8, kern as u8];
    assert!(
        dvi.windows(4).any(|w| w == wanted),
        "no right3 of {kern}sp in the DVI: {:02x?}",
        &dvi[..dvi.len().min(400)]
    );
}
