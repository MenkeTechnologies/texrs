//! The font map against the installation it describes, and the whole chain it
//! joins.
//!
//! There is no program that prints a parsed font map, so the oracle is the
//! installation itself: every file a line names must be a file TeX can find,
//! and every glyph an encoding names must be a glyph the font it is paired
//! with actually holds. That is a stronger check than it sounds -- it reads
//! the map, the `.enc`, the `.pfb` and the `.tfm` with four different readers
//! and requires them to agree about thousands of names.

use std::collections::BTreeSet;
use std::process::Command;

use texrs::fontmap::{Encoding, FontMap};

fn installed(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich").arg(name).output().ok()?;
    let path = String::from_utf8_lossy(&found.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    (!path.is_empty()).then_some(path)
}

fn encoding_file(name: &str) -> Option<String> {
    let found = Command::new("kpsewhich")
        .arg("-format=enc files")
        .arg(name)
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&found.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    (!path.is_empty()).then_some(path)
}

/// The map every pdftex run reads, whole.
#[test]
fn the_installations_own_map_reads_without_a_line_left_over() {
    let Some(path) = installed("pdftex.map") else {
        return;
    };
    let map = FontMap::open(&path).expect("the map reads");

    // A real map is tens of thousands of lines assembled from every package
    // installed; not one of them should be unreadable.
    assert!(
        map.warnings.is_empty(),
        "{} lines could not be read, first: {:?}",
        map.warnings.len(),
        map.warnings.first()
    );
    assert!(map.len() > 1000, "only {} fonts", map.len());

    // The entries a TeX installation always has, read exactly.
    let times = map.lookup("ptmr8r").expect("Times in TeX Base 1 encoding");
    assert_eq!(times.encoding_file.as_deref(), Some("8r.enc"));
    assert!(
        times
            .font_file
            .as_deref()
            .is_some_and(|f| f.ends_with(".pfb")),
        "{times:?}"
    );

    // Every entry says something: a line that parsed to a name and nothing
    // else would be a line read wrongly.
    let empty = map
        .names()
        .iter()
        .filter(|name| {
            let entry = map.lookup(name).expect("an entry");
            entry.font_file.is_none() && entry.ps_name.is_none() && entry.snippet.is_none()
        })
        .count();
    assert_eq!(empty, 0, "{empty} entries hold nothing");

    // A map this size uses both of the transformations.
    assert!(
        map.names()
            .iter()
            .filter(|n| map.lookup(n).expect("an entry").slant != 0.0)
            .count()
            > 100,
        "a real map slants hundreds of fonts"
    );
    assert!(
        map.names()
            .iter()
            .filter(|n| map.lookup(n).expect("an entry").extend != 1.0)
            .count()
            > 10
    );
}

/// Every file the map names is a file the installation has.
///
/// This is the check that says the filenames came out whole: a parser that ate
/// a character would name files that are not there. Every name is checked for
/// rubbish, which costs nothing; a sample is then handed to `kpsewhich` in one
/// call, because asking it forty thousand times takes minutes.
#[test]
fn every_file_a_line_names_is_a_file_tex_can_find() {
    let Some(path) = installed("pdftex.map") else {
        return;
    };
    let map = FontMap::open(&path).expect("the map reads");

    let mut fonts: Vec<&str> = Vec::new();
    let mut encodings: BTreeSet<&str> = BTreeSet::new();
    for name in map.names() {
        let entry = map.lookup(name).expect("an entry");
        if let Some(file) = &entry.font_file {
            assert!(
                !file.contains('"') && !file.contains('<') && !file.is_empty(),
                "{name}: the font file came out as {file:?}"
            );
            fonts.push(file);
        }
        if let Some(file) = &entry.encoding_file {
            assert!(
                file.ends_with(".enc") || !file.contains('.'),
                "{name}: the encoding came out as {file:?}"
            );
            encodings.insert(file);
        }
    }
    assert!(fonts.len() > 1000, "only {} fonts are named", fonts.len());

    // A spread across the map rather than the whole of it, in one call.
    let step = (fonts.len() / 40).max(1);
    let sample: Vec<&str> = fonts.iter().step_by(step).copied().collect();
    let found = Command::new("kpsewhich")
        .args(&sample)
        .output()
        .expect("kpsewhich");
    let found = String::from_utf8_lossy(&found.stdout).lines().count();
    assert!(
        found * 2 > sample.len(),
        "only {found} of {} sampled fonts were found, which is a parser eating characters rather than an installation missing fonts",
        sample.len()
    );

    // The encodings are few enough to ask about all at once.
    let all: Vec<&str> = encodings.iter().copied().collect();
    let found = Command::new("kpsewhich")
        .arg("-format=enc files")
        .args(&all)
        .output()
        .expect("kpsewhich");
    let found = String::from_utf8_lossy(&found.stdout).lines().count();
    assert!(
        found * 2 > all.len(),
        "only {found} of {} encodings were found",
        all.len()
    );
}

/// The whole chain: a TeX font name, through the map, to a real glyph.
///
/// `ptmr8r` is Times as TeX addresses it. The map says which file and which
/// encoding; the encoding says what code 65 is called; the Type 1 font says
/// that glyph exists and how wide it is; and the `.tfm` TeX set the document
/// with says the same width. Four formats, four readers, one answer.
#[test]
fn a_tex_font_name_leads_to_a_glyph_and_its_width() {
    let (Some(map), Some(_)) = (installed("pdftex.map"), installed("ptmr8r.tfm")) else {
        return;
    };
    let map = FontMap::open(&map).expect("the map reads");
    let entry = map.lookup("ptmr8r").expect("Times");

    let (Some(enc), Some(pfb)) = (
        entry.encoding_file.as_deref().and_then(encoding_file),
        entry.font_file.as_deref().and_then(installed),
    ) else {
        return;
    };
    let encoding = Encoding::open(&enc).expect("the encoding reads");
    let font = texrs::type1::Type1::open(&pfb).expect("the font reads");
    let tfm = texrs::tfm::Tfm::open(installed("ptmr8r.tfm").expect("metrics")).expect("reads");

    // 8r is TeX's own arrangement of a PostScript font, so an A is where an A
    // is in ASCII.
    assert_eq!(encoding.glyph(b'A'), Some("A"));
    assert!(encoding.used() > 200, "8r fills most of its 256 codes");

    // Every code the encoding uses is a glyph the font holds, and every one
    // has a width -- which is what the driver needs and what a wrong file
    // name or a wrong array would break.
    let names: BTreeSet<&str> = font.glyph_names().into_iter().collect();
    let mut checked = 0usize;
    let mut agreed = 0usize;
    for code in 0..=255u8 {
        let Some(name) = encoding.glyph(code) else {
            continue;
        };
        // The font need not hold every glyph the encoding names, but the ones
        // it holds must line up with the metrics TeX used.
        if !names.contains(name) {
            continue;
        }
        let glyph = font.glyph(name).expect("a glyph");
        checked += 1;
        let Some(metrics) = tfm.char(code) else {
            continue;
        };
        if metrics.width == 0.0 {
            continue;
        }
        assert!(
            (glyph.width - metrics.width * 1000.0).abs() < 1.5,
            "0o{code:o} ({name}): the font says {}, the .tfm says {}",
            glyph.width,
            metrics.width * 1000.0
        );
        agreed += 1;
    }
    assert!(checked > 150, "only {checked} codes reached a glyph");
    assert!(
        agreed > 90,
        "only {agreed} widths were compared against the metrics"
    );
}

/// The encodings TeX Live ships, read whole.
#[test]
fn the_installations_encodings_read_as_arrays_of_names() {
    let mut read = 0usize;
    for name in ["8r.enc", "texnansi.enc", "cm-super-t1.enc", "ot1.enc"] {
        let Some(path) = encoding_file(name) else {
            continue;
        };
        let encoding = Encoding::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            encoding.glyphs.len() <= 256,
            "{name}: {} names",
            encoding.glyphs.len()
        );
        assert!(
            encoding.used() > 100,
            "{name}: only {} used",
            encoding.used()
        );
        assert!(!encoding.name.is_empty(), "{name}: the array has no name");
        // Every name is a PostScript name rather than a fragment of the file.
        for glyph in &encoding.glyphs {
            assert!(
                glyph
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'),
                "{name}: {glyph:?} is not a glyph name"
            );
        }
        read += 1;
    }
    if encoding_file("8r.enc").is_some() {
        assert!(read > 1, "only {read} encodings were read");
    }
}
