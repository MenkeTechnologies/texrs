//! A glyph the document's own face lacks, fetched from a face that has it.
//!
//! The corpus books ship their faces -- `\setmainfont{Arimo}` with `Path=` and
//! `UprightFont=`, embedded whole as `/FontFile2` -- and Arimo has no box
//! drawing. Neither has the Symbol font, which is the only other place the PDF
//! backend used to look, so U+2500 fell to an ASCII stand-in: measured on
//! `arb/docs/book.tex`, lualatex puts 5,614 of them on the page and texrs put
//! none. Every one of those books states where a missing glyph comes from, in
//! the one line that made their build require LuaTeX:
//!
//! ```text
//! \directlua{luaotfload.add_fallback("symfb", {"Arial Unicode MS:mode=base;", ...})}
//! ```
//!
//! These tests are that line being honoured: the chain is read, the face is
//! asked, the glyph is drawn, and it is drawn in a way a reader can copy back
//! out as the character the document wrote.

use std::collections::BTreeSet;

/// A `\directlua` chunk is read for its fallback chain and for nothing else.
#[test]
fn the_document_says_which_faces_a_missing_glyph_comes_from() {
    let chain = texrs::typeset::fallback_chain(
        "luaotfload.add_fallback(\"symfb\", {\"Arial Unicode MS:mode=base;\", \
         \"STIX Two Math:mode=base;\", \"Noto Emoji:mode=base;\", })",
    );
    assert_eq!(
        chain,
        vec![
            "Arial Unicode MS".to_string(),
            "STIX Two Math".to_string(),
            "Noto Emoji".to_string(),
        ],
        "the families, in order, with luaotfload's own options cut off at the colon"
    );

    // Anything else in a chunk yields no chain from the TEXT reader, and guessing
    // at what a chunk computes would put a face nobody asked for in a book.
    assert!(texrs::typeset::fallback_chain("tex.print('x')").is_empty());
    assert!(texrs::typeset::fallback_chain("luaotfload.add_fallback(\"only\")").is_empty());
}

/// A family installed here that carries `ch`, or `None`.
///
/// Font availability is not a property of the code, so a test that needs a real
/// face says which character it needs and skips rather than failing on a
/// machine without one.
fn family_carrying(ch: char) -> Option<&'static str> {
    [
        "Arial Unicode MS",
        "DejaVu Sans",
        "DejaVu Sans Mono",
        "Noto Sans Symbols 2",
        "FreeSerif",
        "Menlo",
        "Apple Symbols",
    ]
    .into_iter()
    .find(|family| {
        let Some(path) = texrs::typeset::find_fallback_family(family) else {
            return false;
        };
        let Ok(sfnt) = texrs::sfnt::Sfnt::open(&path) else {
            return false;
        };
        // Glyph 0 is `.notdef`: a cmap that answers with it has not answered.
        !sfnt.is_cff()
            && sfnt
                .cmap()
                .is_ok_and(|m| m.get(&(ch as u32)).is_some_and(|g| *g != 0))
    })
}

/// The bytes a content stream holds for `codes`, escaped the way `Page::draw`
/// escapes them.
fn escaped(codes: &[u8]) -> String {
    codes
        .iter()
        .map(|&b| match b {
            b'(' | b')' | b'\\' => format!("\\{}", b as char),
            32..=126 => (b as char).to_string(),
            _ => format!("\\{b:03o}"),
        })
        .collect()
}

/// The glyph id a `/ToUnicode` map says means `ch`.
fn glyph_meaning(pdf: &str, ch: char) -> Option<u16> {
    let wanted = format!("<{:04X}>\n", ch as u32);
    let line = pdf.lines().find(|line| line.ends_with(wanted.trim_end()))?;
    let code = line.strip_prefix('<')?.split('>').next()?;
    u16::from_str_radix(code, 16).ok()
}

fn book(chain: Option<&str>, body: &str) -> String {
    let lua = match chain {
        Some(family) => {
            format!(
                "\\directlua{{luaotfload.add_fallback(\"symfb\", {{\"{family}:mode=base;\"}})}}\n"
            )
        }
        None => String::new(),
    };
    format!(
        "\\documentclass{{article}}\n\\usepackage{{fontspec}}\n{lua}\
         \\begin{{document}}\n{body}\n\\end{{document}}\n"
    )
}

/// The character reaches the page, out of the face the document named for it,
/// and reaches the page's TEXT as itself.
#[test]
fn a_character_the_face_lacks_is_drawn_from_the_face_the_document_named() {
    let Some(family) = family_carrying('─') else {
        eprintln!("skipping: no installed face carries U+2500");
        return;
    };
    let pdf = texrs::run_pdf(&book(Some(family), "tree ─── branch")).expect("pdf");
    // Inflated: the font dictionaries are packed into an object stream and
    // are not in the file's own bytes at all. A raw scan would find neither
    // what is asserted here nor what is asserted absent.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&pdf)).into_owned();

    // A composite font, because a simple one's encoding has 256 slots and none
    // of them means U+2500.
    assert!(
        text.contains("/Subtype /Type0") && text.contains("/Encoding /Identity-H"),
        "the borrowed face must be addressed by glyph"
    );
    assert!(
        text.contains("/FontFile2"),
        "and carried in the file, not named and hoped for"
    );

    // What the glyph MEANS, which is what makes the line copy back out.
    let glyph = glyph_meaning(&text, '─').expect("U+2500 must be in a /ToUnicode map");
    assert!(glyph != 0, "glyph 0 is .notdef and draws nothing");

    // And that glyph is what the page draws: two bytes, high first, which is
    // what `/Identity-H` reads. Three of them in a row, for the three the
    // document wrote.
    let drawn = escaped(&[(glyph >> 8) as u8, (glyph & 0xFF) as u8]);
    assert!(
        text.contains(&format!("({}) Tj", drawn.repeat(3))),
        "the page must draw glyph {glyph} three times; it does not"
    );

    // The stand-in is what this replaces. A hyphen where the box drawing was is
    // the old behaviour, and the run holding the box drawing must not be it.
    assert!(
        !text.contains("(tree ---) Tj") && !text.contains("(---) Tj"),
        "the box drawing must not still be set as hyphens"
    );
}

/// The Symbol font stays the first place a missing glyph is looked for, and
/// keeps the map that makes an arrow searchable.
#[test]
fn the_symbol_font_is_still_asked_before_the_document_s_own_chain() {
    let Some(family) = family_carrying('─') else {
        return;
    };
    // A face carrying U+2500 carries U+2192 as well, so this says which of the
    // two the arrow came from rather than whether it arrived at all.
    let pdf = texrs::run_pdf(&book(Some(family), "a → b")).expect("pdf");
    // Inflated: the font dictionaries are packed into an object stream and
    // are not in the file's own bytes at all. A raw scan would find neither
    // what is asserted here nor what is asserted absent.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&pdf)).into_owned();
    assert!(
        text.contains("/BaseFont /Symbol"),
        "the arrow comes from Symbol, which costs the file nothing"
    );
    // Symbol's own code for `arrowright`, out of `psyr.afm`.
    assert!(
        text.contains(&format!("({}) Tj", escaped(&[174]))),
        "the page must draw Symbol's code 174"
    );
    assert_eq!(
        glyph_meaning(&text, '→'),
        Some(174),
        "and say that it means U+2192"
    );
}

/// A document that names no chain is set exactly as it was before there was
/// one: the stand-ins, and no borrowed face in the file.
#[test]
fn without_a_chain_a_missing_glyph_still_falls_to_its_stand_in() {
    let pdf = texrs::run_pdf(&book(None, "tree ─── branch")).expect("pdf");
    // Inflated: the font dictionaries are packed into an object stream and
    // are not in the file's own bytes at all. A raw scan would find neither
    // what is asserted here nor what is asserted absent.
    let text = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&pdf)).into_owned();
    assert!(
        !text.contains("/Identity-H"),
        "nothing was named, so nothing may be borrowed"
    );
    assert!(
        text.contains("(tree --- branch) Tj"),
        "the ASCII stand-in is what U+2500 falls to with no chain"
    );
}

/// The face is carried as the glyphs the document borrowed, not whole.
///
/// Arial Unicode is 23 MB and the corpus books' own chain names it first;
/// embedding it whole to draw nine box-drawing glyphs would put 23 MB into
/// every book. The subset keeps the glyph IDS, because a `/CIDToGIDMap
/// /Identity` font addresses a glyph by the id the face gave it.
#[test]
fn a_borrowed_face_is_carried_as_the_glyphs_that_were_borrowed() {
    let Some(family) = family_carrying('─') else {
        return;
    };
    let path = texrs::typeset::find_fallback_family(family).expect("just found it");
    let whole = texrs::sfnt::Sfnt::open(&path).expect("readable");
    let glyph = *whole
        .cmap()
        .expect("cmap")
        .get(&('─' as u32))
        .expect("carries U+2500");

    let bytes = whole
        .subset(&BTreeSet::from([glyph]))
        .expect("the subset builds");
    let subset = texrs::sfnt::Sfnt::parse(bytes.clone()).expect("and re-reads");

    assert_eq!(
        subset.num_glyphs().expect("maxp"),
        whole.num_glyphs().expect("maxp"),
        "the ids must not move: a code in the file IS one of them"
    );
    assert!(
        bytes.len() < std::fs::metadata(&path).expect("stat").len() as usize,
        "a subset that is not smaller has subsetted nothing"
    );

    // The glyph asked for still has its outline, and one that was not asked for
    // has none: that is what was dropped.
    let outline = |sfnt: &texrs::sfnt::Sfnt, glyph: u16| -> usize {
        let loca = sfnt.table("loca").expect("loca");
        let long = sfnt.head().expect("head").long_loca;
        let at = |g: usize| match long {
            true => u32::from_be_bytes(loca[g * 4..g * 4 + 4].try_into().unwrap()) as usize,
            false => u16::from_be_bytes(loca[g * 2..g * 2 + 2].try_into().unwrap()) as usize * 2,
        };
        at(glyph as usize + 1) - at(glyph as usize)
    };
    assert!(
        outline(&subset, glyph) > 0,
        "the borrowed glyph must keep its outline"
    );
    let untouched = (1..whole.num_glyphs().expect("maxp"))
        .find(|g| *g != glyph && outline(&whole, *g) > 0)
        .expect("some other glyph has an outline");
    assert_eq!(
        outline(&subset, untouched),
        0,
        "a glyph nobody borrowed must be empty in the subset"
    );
}
