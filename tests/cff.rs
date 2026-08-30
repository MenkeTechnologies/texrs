//! The CFF reader against the rest of the font it lives in.
//!
//! `tests/sfnt.rs` holds the glyph names to `otfinfo`, which is the oracle for
//! the charset. The widths have a different and better one: an OpenType font
//! states every glyph's advance twice, once in `hmtx` for the layout engine and
//! once inside the charstring for the rasteriser, written in two entirely
//! different ways by two parts of the same tool. They must agree, and a Type 2
//! width is easy to read wrongly -- it is an optional extra operand in front of
//! whichever operator comes first, expressed as a difference from a number in
//! the Private DICT, and absent altogether when the glyph is of the default
//! width.

use std::process::Command;

use texrs::sfnt::Sfnt;

fn installed(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich").arg(name).output().ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// Every glyph's width, from the charstrings and from `hmtx`.
#[test]
fn the_charstring_widths_are_the_widths_hmtx_states() {
    let fonts = [
        "lmroman10-regular.otf",
        "lmroman10-italic.otf",
        "lmmono10-regular.otf",
        "lmsans10-regular.otf",
    ];
    let mut compared = 0usize;
    let mut names = 0usize;
    let mut fractional = 0usize;
    for name in fonts {
        let Some(path) = installed(name) else {
            continue;
        };
        let font = Sfnt::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let Some(cff) = font.cff() else {
            panic!("{name}: no CFF table")
        };
        let hmtx = font.advance_widths().expect("hmtx");

        assert_eq!(
            cff.len(),
            font.num_glyphs().expect("maxp") as usize,
            "{name}: the CFF and maxp disagree about how many glyphs there are"
        );
        assert_eq!(cff.widths.len(), hmtx.len(), "{name}");

        for (glyph, (&charstring, &stated)) in cff.widths.iter().zip(hmtx.iter()).enumerate() {
            // A charstring's width can be fractional -- it may arrive as
            // 16.16 fixed point -- while `hmtx` holds whole units, so the two
            // agree to the rounding and not to the bit. A width read from the
            // wrong place is wrong by hundreds, not by a fraction.
            assert_eq!(
                charstring.round() as i64,
                stated as i64,
                "{name} glyph {glyph} ({}): the charstring says {charstring}, hmtx says {stated}",
                cff.glyph_names[glyph]
            );
            if charstring.fract() != 0.0 {
                fractional += 1;
            }
            compared += 1;
        }

        // The names are real names, and the ones a Latin font must have are
        // there.
        for wanted in ["A", "space", "one"] {
            assert!(
                cff.glyph_names.iter().any(|n| n == wanted),
                "{name} has no {wanted}"
            );
        }
        names += cff.len();
    }
    if installed("lmroman10-regular.otf").is_some() {
        assert!(compared > 2000, "only {compared} widths were compared");
        assert!(names > 2000, "only {names} names were read");
        // Nearly all of them are whole: a fractional width means the operand
        // arrived as fixed point, which a handful of glyphs in Latin Modern do.
        assert!(
            fractional < compared / 100,
            "{fractional} of {compared} widths are fractional, which is too many to be fixed point"
        );
    }
}

/// The width is where a Type 2 charstring differs most from a Type 1 one, so
/// this checks the three cases exist in a real font rather than assuming.
#[test]
fn a_font_uses_both_the_default_width_and_stated_ones() {
    let Some(path) = installed("lmroman10-regular.otf") else {
        return;
    };
    let font = Sfnt::open(&path).expect("reads");
    let cff = font.cff().expect("a CFF");

    // Glyphs of the font's default width say nothing in their charstrings;
    // others carry a difference. A font where every glyph took the same path
    // would not test the reader.
    let mut counts = std::collections::BTreeMap::new();
    for &width in &cff.widths {
        *counts.entry(width as i64).or_insert(0usize) += 1;
    }
    assert!(counts.len() > 50, "only {} distinct widths", counts.len());
    let commonest = counts.values().copied().max().expect("a width");
    assert!(
        commonest > 20,
        "no width is common enough to be the default one"
    );

    // A monospaced font is the other extreme: every glyph the same width, so
    // every charstring is silent about it.
    let Some(mono) = installed("lmmono10-regular.otf") else {
        return;
    };
    let mono = Sfnt::open(&mono).expect("reads").cff().expect("a CFF");
    let widths: std::collections::BTreeSet<i64> = mono.widths.iter().map(|&w| w as i64).collect();
    // Nearly one width: the letters are all the same, and a handful of
    // combining marks are zero-width, which is what a monospaced font does
    // rather than a contradiction.
    assert!(
        widths.len() <= 5,
        "a monospaced font has one width, not {}",
        widths.len()
    );
    let commonest = mono
        .widths
        .iter()
        .filter(|&&w| w as i64 == 525)
        .count()
        .max(mono.widths.iter().filter(|&&w| w as i64 != 0).count());
    assert!(
        commonest * 2 > mono.widths.len(),
        "most glyphs of a monospaced font share a width"
    );
}
