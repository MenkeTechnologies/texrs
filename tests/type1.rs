//! The Type 1 reader against `t1disasm` and against the metrics.
//!
//! A Type 1 font is encrypted twice over, so a reader that gets either key or
//! either skip wrong produces rubbish that still parses -- names of random
//! bytes, widths of nonsense. Two independent oracles say it did not:
//!
//!  * `t1disasm` decrypts the font and prints every glyph as text, beginning
//!    with the `hsbw` that declares its width. Comparing names and widths
//!    against it checks both decryptions and the charstring number encoding.
//!  * The `.afm` beside the font, and the `.tfm` TeX sets it with, say the same
//!    widths by two entirely different roads -- Adobe's metrics file and
//!    Knuth's. A width that agrees with all three is right.

use std::collections::BTreeMap;
use std::process::Command;

use texrs::type1::Type1;

fn installed(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich").arg(name).output().ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// What `t1disasm` prints: every glyph name, with the side bearing and width
/// its `hsbw` declares.
fn disassembled(path: &str) -> Option<BTreeMap<String, (f64, f64)>> {
    let out = Command::new("t1disasm").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut all = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        // A glyph begins `/name {`; a subroutine begins `dup N {`, which this
        // is not asking about.
        if let Some(rest) = line.strip_prefix('/') {
            if let Some(name) = rest.strip_suffix(" {") {
                current = Some(name.to_string());
                continue;
            }
        }
        let Some(name) = current.take() else { continue };
        let words: Vec<&str> = line.split_whitespace().collect();
        // The first line of a charstring is `<sb> <width> hsbw`, or four
        // numbers and `sbw` for a glyph that moves vertically too.
        match words.as_slice() {
            [sb, width, "hsbw"] => {
                if let (Ok(sb), Ok(width)) = (sb.parse(), width.parse()) {
                    all.insert(name, (sb, width));
                }
            }
            [sbx, _sby, wx, _wy, "sbw"] => {
                if let (Ok(sb), Ok(width)) = (sbx.parse(), wx.parse()) {
                    all.insert(name, (sb, width));
                }
            }
            _ => {}
        }
    }
    Some(all)
}

/// The widths in an `.afm`, by glyph name.
fn metrics(path: &str) -> Option<BTreeMap<String, f64>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if !line.starts_with("C ") {
            continue;
        }
        let mut width = None;
        let mut name = None;
        for field in line.split(';') {
            let words: Vec<&str> = field.split_whitespace().collect();
            match words.as_slice() {
                ["WX", value] => width = value.parse::<f64>().ok(),
                ["N", value] => name = Some(value.to_string()),
                _ => {}
            }
        }
        if let (Some(name), Some(width)) = (name, width) {
            out.insert(name, width);
        }
    }
    Some(out)
}

/// The fonts to read: four of Computer Modern's, which between them use both
/// width operators and every size of number.
const FONTS: [&str; 4] = ["cmr10", "cmti10", "cmsy10", "cmex10"];

/// Every glyph, against `t1disasm`.
#[test]
fn every_glyph_is_the_one_t1disasm_decrypts() {
    let mut glyphs = 0usize;
    for name in FONTS {
        let Some(path) = installed(&format!("{name}.pfb")) else {
            continue;
        };
        let Some(want) = disassembled(&path) else {
            continue;
        };
        assert!(!want.is_empty(), "{name}: t1disasm printed no glyphs");

        let font = Type1::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The same glyphs, neither more nor fewer: a reader that ran past the
        // end of the dictionary would invent some.
        let mine: Vec<&str> = font.glyph_names();
        let theirs: Vec<&str> = want.keys().map(String::as_str).collect();
        assert_eq!(mine, theirs, "{name}: a different set of glyphs");

        for (glyph_name, &(side_bearing, width)) in &want {
            let glyph = font.glyph(glyph_name).expect("a glyph");
            assert_eq!(glyph.width, width, "{name}/{glyph_name}: width");
            assert_eq!(
                glyph.left_side_bearing, side_bearing,
                "{name}/{glyph_name}: side bearing"
            );
            glyphs += 1;
        }
    }
    if installed("cmr10.pfb").is_some() {
        assert!(glyphs > 400, "only {glyphs} glyphs were compared");
    }
}

/// The same widths, from Adobe's metrics file and from Knuth's.
///
/// Three formats written by three different people at three different times,
/// agreeing to the unit, is a stronger statement about the reader than any one
/// of them alone.
#[test]
fn the_widths_agree_with_the_afm_and_the_tfm() {
    let mut compared = 0usize;
    for name in FONTS {
        let (Some(pfb), Some(afm)) = (
            installed(&format!("{name}.pfb")),
            installed(&format!("{name}.afm")),
        ) else {
            continue;
        };
        let font = Type1::open(&pfb).expect("the font reads");
        let want = metrics(&afm).expect("the metrics read");
        assert!(!want.is_empty(), "{name}: the .afm lists no characters");

        for (glyph_name, &width) in &want {
            let glyph = font
                .glyph(glyph_name)
                .unwrap_or_else(|| panic!("{name}: the font has no {glyph_name}"));
            assert_eq!(
                glyph.width, width,
                "{name}/{glyph_name}: the charstring and the .afm disagree"
            );
            compared += 1;
        }

        // And the .tfm TeX sets the font with. Its widths are fractions of the
        // design size, and the font's are whole thousandths of an em, so the
        // two agree to within one of the font's units and not exactly:
        // cmr10's Theta is 777 where the .tfm says 777.78, and its Delta is
        // 833 where the .tfm says 833.34. What the comparison catches is a
        // width read from the wrong place, which is wrong by hundreds.
        let Some(tfm) = installed(&format!("{name}.tfm")) else {
            continue;
        };
        let tfm = texrs::tfm::Tfm::open(&tfm).expect("the metrics read");
        for code in tfm.codes() {
            let Some(glyph) = font.encoded(code) else {
                continue;
            };
            let metrics = tfm.char(code).expect("a character");
            assert!(
                (glyph.width - metrics.width * 1000.0).abs() < 1.0,
                "{name} 0o{code:o} ({}): the font says {}, the .tfm says {}",
                glyph.name,
                glyph.width,
                metrics.width * 1000.0
            );
            compared += 1;
        }
    }
    if installed("cmr10.afm").is_some() {
        assert!(compared > 400, "only {compared} widths were compared");
    }
}

/// The font's own encoding, against the one the `.afm` records.
#[test]
fn the_encoding_is_the_one_the_metrics_record() {
    let Some(afm) = installed("cmr10.afm") else {
        return;
    };
    let font = Type1::open(installed("cmr10.pfb").expect("the font")).expect("reads");
    let text = std::fs::read_to_string(&afm).expect("the metrics");

    let mut checked = 0usize;
    for line in text.lines() {
        if !line.starts_with("C ") {
            continue;
        }
        let mut code: Option<i64> = None;
        let mut name: Option<&str> = None;
        for field in line.split(';') {
            let words: Vec<&str> = field.split_whitespace().collect();
            match words.as_slice() {
                ["C", value] => code = value.parse().ok(),
                ["N", value] => name = Some(value),
                _ => {}
            }
        }
        let (Some(code), Some(name)) = (code, name) else {
            continue;
        };
        if !(0..=255).contains(&code) {
            continue;
        }
        assert_eq!(
            font.encoding.get(&(code as u8)).map(String::as_str),
            Some(name),
            "code {code}"
        );
        checked += 1;
    }
    assert!(checked > 100, "only {checked} codes were compared");
    // Computer Modern is not in Adobe's encoding, which is the whole reason
    // TeX can put an ff ligature at position 11.
    assert!(!font.uses_standard_encoding);
}

/// The four heights a font descriptor needs come from the `.afm`, and are read
/// off it rather than guessed from the outlines.
///
/// A Type 1 font states none of them: `cmr10.pfb` has no `Ascender`, no
/// `CapHeight`, no `Descender` and no `XHeight` anywhere in it, encrypted half
/// included. `cmr10.afm` states all four, and that is where LuaTeX gets the
/// `/Ascent 694 /CapHeight 683 /Descent -194 /XHeight 431` it writes.
///
/// The bounding box is a different question with a different answer -- CMR10's
/// outlines reach 750 and -250, its letters 694 and -194 -- so the two must not
/// agree here or the metrics were not read at all.
#[test]
fn the_descriptor_heights_come_from_the_metrics_file() {
    let (Some(pfb), Some(afm)) = (installed("cmr10.pfb"), installed("cmr10.afm")) else {
        return;
    };
    let font = Type1::open(&pfb).expect("the font reads");
    let metrics = font.afm_metrics.expect("the metrics beside it were found");

    // Against the file itself, read a second way: the assertion is that the
    // reader agrees with the metrics, not with a number typed in here.
    let text = std::fs::read_to_string(&afm).expect("the metrics read");
    let stated = |key: &str| -> f64 {
        text.lines()
            .take_while(|line| !line.starts_with("StartCharMetrics"))
            .find_map(|line| line.strip_prefix(&format!("{key} ")))
            .unwrap_or_else(|| panic!("{afm} states no {key}"))
            .trim()
            .parse()
            .expect("a number")
    };
    assert_eq!(metrics.ascender, stated("Ascender"));
    assert_eq!(metrics.descender, stated("Descender"));
    assert_eq!(metrics.cap_height, stated("CapHeight"));
    assert_eq!(metrics.x_height, stated("XHeight"));

    // And they are not the bounding box, which is what they used to be.
    assert_ne!(metrics.ascender, font.font_bbox[3]);
    assert_ne!(metrics.descender, font.font_bbox[1]);

    // The font itself says none of this: the whole reason for the second file.
    let raw = std::fs::read(&pfb).expect("the font reads");
    for key in [&b"Ascender"[..], b"CapHeight", b"XHeight"] {
        assert!(
            !raw.windows(key.len()).any(|w| w == key),
            "the .pfb states {}, so the metrics file was not needed",
            String::from_utf8_lossy(key)
        );
    }

    // A font read from BYTES has no file to look beside, so it says nothing
    // rather than inventing an answer.
    assert_eq!(Type1::parse(&raw).expect("reads").afm_metrics, None);
}

/// A subset carries the glyphs it was cut to, and a reader gets them back.
///
/// This is the piece that makes an embedded Computer Modern 11 kB instead of
/// 40: measured, luatex's `/FontFile` for `Hello world.` is `/Length1 1510
/// /Length2 9354` where the whole cmr10.pfb is 4287 and 30900. Cutting one
/// wrongly is not a small error -- the eexec half is encrypted, so a body
/// rebuilt a byte out of step decrypts to noise and the page draws nothing --
/// and there are three independent oracles here that it was not:
///
///  * the subset reads back through the same parser, which decrypts both
///    halves and finds the charstrings by the lengths the file states;
///  * every kept charstring is byte for byte the one the whole font held, so
///    the outlines are the font's own rather than something re-encoded;
///  * `t1disasm`, which decrypts the font by itself and prints what it finds.
#[test]
fn a_subset_carries_the_glyphs_it_was_cut_to_and_no_others() {
    let Some(pfb) = installed("cmr10.pfb") else {
        return;
    };
    let whole = Type1::open(&pfb).expect("the font reads");
    let keep: std::collections::BTreeSet<String> = ["H", "e", "l", "o", "period"]
        .iter()
        .map(|name| name.to_string())
        .collect();
    let cut = whole.subset(&keep, "ABCDEF").expect("the font cuts");

    // §9.6.4: a subset is named for itself, so a reader never takes one cut of
    // a face for another.
    assert_eq!(cut.font_name, "ABCDEF+CMR10");
    assert_eq!(cut.glyph_names(), vec!["H", "e", "l", "o", "period"]);
    // The font's own encoding, cut to the same glyphs: CMR10 has 166 codes and
    // these five are at the five the whole font put them at.
    for (code, name) in &cut.encoding {
        assert!(keep.contains(name), "code {code} kept {name}");
        assert_eq!(whole.encoding.get(code), Some(name));
    }
    assert_eq!(cut.encoding.len(), keep.len());

    // Read back through the parser, which undoes both encryptions.
    let (bytes, clear, binary, trailer) = cut.embeddable();
    assert_eq!(bytes.len(), clear + binary + trailer, "the lengths cover it");
    let again = Type1::parse(&bytes).expect("the subset reads back");
    assert_eq!(again.font_name, "ABCDEF+CMR10");
    assert_eq!(again.glyph_names(), cut.glyph_names());
    assert_eq!(again.encoding, cut.encoding);
    for name in &keep {
        let was = whole.glyph(name).expect("the whole font has it");
        let now = again.glyph(name).expect("the subset has it");
        assert_eq!(now.charstring, was.charstring, "{name}'s outline changed");
        assert_eq!(now.width, was.width, "{name}'s width changed");
    }
    // And what it left behind is gone.
    assert!(again.glyph("Omega").is_none(), "a glyph nobody drew is in it");

    // Much smaller than the font it came out of, which is the point.
    let size = std::fs::metadata(&pfb).expect("the font").len() as usize;
    assert!(
        bytes.len() * 2 < size,
        "{} bytes is not a subset of a {size}-byte font",
        bytes.len()
    );

    // A second decryptor, sharing no code with this one.
    let dir = std::env::temp_dir().join(format!("texrs-t1subset-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join("cut.pfa");
    std::fs::write(&path, &bytes).expect("the subset writes");
    if let Ok(out) = Command::new("t1disasm").arg(&path).output() {
        // Installed and refusing the font is the finding, not a reason to
        // skip: a subset no second decryptor accepts is a subset nothing can
        // draw with.
        assert!(
            out.status.success(),
            "t1disasm refused the subset: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        for name in &keep {
            assert!(
                text.contains(&format!("/{name} {{")),
                "t1disasm did not find /{name} in the subset"
            );
        }
        assert!(
            !text.contains("/Omega {"),
            "t1disasm found a glyph the subset should not carry"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Cutting works on every face the reader reads, not only on cmr10.
///
/// The three Computer Modern faces put different things in their headers --
/// cmti10 leans, cmsy10 and cmex10 are symbol fonts with their own glyph
/// vocabularies -- and the URW faces are in Adobe's StandardEncoding, so their
/// cleartext says `/Encoding StandardEncoding def` and has no array to cut at
/// all. A subsetter that only ever saw one font's header would take one of the
/// others apart wrongly, and the encryption means it would do so silently.
#[test]
fn a_subset_of_any_face_the_reader_reads_still_decrypts() {
    let mut cut_any = false;
    for name in FONTS.iter().chain(["uhvr8a", "utmr8a"].iter()) {
        let Some(path) = installed(&format!("{name}.pfb")) else {
            continue;
        };
        let whole = Type1::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The first five glyphs it has, whatever they are called.
        let keep: std::collections::BTreeSet<String> = whole
            .glyph_names()
            .into_iter()
            .filter(|glyph| *glyph != ".notdef")
            .take(5)
            .map(str::to_string)
            .collect();
        let cut = whole
            .subset(&keep, "ZZZZZZ")
            .unwrap_or_else(|| panic!("{name}: the font would not cut"));
        cut_any = true;

        let (bytes, clear, binary, trailer) = cut.embeddable();
        assert_eq!(bytes.len(), clear + binary + trailer, "{name}: the lengths");
        let again = Type1::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(again.font_name, format!("ZZZZZZ+{}", whole.font_name));
        for glyph in &keep {
            assert_eq!(
                again.glyph(glyph).map(|g| g.charstring.clone()),
                whole.glyph(glyph).map(|g| g.charstring.clone()),
                "{name}/{glyph}: the outline changed"
            );
        }
        assert!(
            bytes.len() < std::fs::metadata(&path).expect("the font").len() as usize,
            "{name}: the cut is no smaller than the font"
        );
    }
    if installed("cmr10.pfb").is_some() {
        assert!(cut_any, "no font was cut, so nothing was tested");
    }
}
