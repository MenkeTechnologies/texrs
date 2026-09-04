//! The OpenType reader against `otfinfo`.
//!
//! `otfinfo` reports four things this reads, and each is a different table:
//! `-t` is the table directory, `-i` is the `name` table, `-u` is the `cmap`,
//! and `-g` is `post` (or the CFF charset). Comparing all four over whole fonts
//! exercises the parts where a reader goes quietly wrong -- a `cmap` format 4
//! whose range offsets are counted from the wrong place gives plausible glyphs
//! for the wrong characters, and nothing but a full comparison catches it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use texrs::sfnt::Sfnt;

/// The fonts to read: Latin Modern in three shapes (CFF outlines, which is
/// what a modern TeX document is set in), and a TrueType font, whose tables
/// are laid out differently and which carries glyph names in `post`.
fn fonts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in [
        "lmroman10-regular.otf",
        "lmroman10-italic.otf",
        "lmmono10-regular.otf",
    ] {
        if let Ok(found) = Command::new("kpsewhich").arg(name).output() {
            let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
            if !path.is_empty() {
                out.push(PathBuf::from(path));
            }
        }
    }
    let truetype = Path::new(
        "/usr/local/texlive/2026/texmf-dist/fonts/truetype/intel/clearsans/ClearSans-Regular.ttf",
    );
    if truetype.exists() {
        out.push(truetype.to_path_buf());
    }
    out
}

fn otfinfo(flag: &str, path: &Path) -> Option<String> {
    let out = Command::new("otfinfo").arg(flag).arg(path).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `otfinfo -t`: every table, with its length.
#[test]
fn the_table_directory_is_the_one_otfinfo_lists() {
    let mut checked = 0usize;
    for path in fonts() {
        let Some(text) = otfinfo("-t", &path) else {
            continue;
        };
        let want: BTreeMap<String, usize> = text
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                let length: usize = words.next()?.parse().ok()?;
                // A tag is four characters, so `CFF ` carries a trailing
                // space; otfinfo prints it trimmed.
                Some((words.next()?.trim().to_string(), length))
            })
            .collect();
        assert!(
            !want.is_empty(),
            "{}: otfinfo listed no tables",
            path.display()
        );

        let font = Sfnt::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let got: BTreeMap<String, usize> = font
            .tables
            .iter()
            .map(|t| (t.tag.trim().to_string(), t.length))
            .collect();
        assert_eq!(got, want, "{}", path.display());

        // And every table's bytes are really there to read.
        for table in &font.tables {
            let tag = table.tag.trim();
            let bytes = font
                .table(&table.tag)
                .unwrap_or_else(|| panic!("{tag} vanished"));
            assert_eq!(bytes.len(), got[tag], "{}: {tag}", path.display());
        }
        checked += 1;
    }
    if !fonts().is_empty() {
        assert!(checked > 0, "no font was compared");
    }
}

/// `otfinfo -i`: the `name` table, string for string.
#[test]
fn the_names_are_the_ones_otfinfo_reports() {
    for path in fonts() {
        let Some(text) = otfinfo("-i", &path) else {
            continue;
        };
        let want: BTreeMap<&str, &str> = text
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(label, value)| (label.trim(), value.trim()))
            .collect();
        let font = Sfnt::open(&path).expect("reads");

        // The four ids a driver asks for, by the labels otfinfo prints them
        // under.
        for (label, id) in [
            ("Family", 1u16),
            ("Subfamily", 2),
            ("Full name", 4),
            ("PostScript name", 6),
        ] {
            let Some(&expected) = want.get(label) else {
                continue;
            };
            assert_eq!(
                font.name(id).as_deref().map(str::trim),
                Some(expected),
                "{}: name {id} ({label})",
                path.display()
            );
        }
        // Version is name id 5, and otfinfo prints it with its own prefix
        // stripped, so it is compared as a prefix rather than exactly.
        if let (Some(want), Some(got)) = (want.get("Version"), font.name(5)) {
            assert!(
                got.trim().contains(want.trim()) || want.trim().contains(got.trim()),
                "{}: version {got:?} against {want:?}",
                path.display()
            );
        }
    }
}

/// `otfinfo -u`: every character the font maps, and the glyph it maps to.
///
/// This is the comparison that matters. `otfinfo` prints one line per mapped
/// character -- `uni0041 36 A` -- so a `cmap` read wrongly shows up as a
/// character mapping to a glyph it does not.
#[test]
fn every_character_maps_where_otfinfo_says() {
    let mut mappings = 0usize;
    for path in fonts() {
        let Some(text) = otfinfo("-u", &path) else {
            continue;
        };
        let want: BTreeMap<u32, u16> = text
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                let code = words.next()?.strip_prefix("uni")?;
                let code = u32::from_str_radix(code, 16).ok()?;
                Some((code, words.next()?.parse().ok()?))
            })
            // otfinfo prints the required last segment of a format 4 cmap,
            // which maps U+FFFF to glyph 0. A mapping to glyph 0 is the
            // absence of a glyph, and this reader does not keep one.
            .filter(|&(_, glyph): &(u32, u16)| glyph != 0)
            .collect();
        assert!(
            !want.is_empty(),
            "{}: otfinfo mapped nothing",
            path.display()
        );

        let font = Sfnt::open(&path).expect("reads");
        let got = font.cmap().expect("a cmap");
        // otfinfo reports what the font's best Unicode subtable says, which is
        // the subtable this picks.
        assert_eq!(got, want, "{}", path.display());
        mappings += got.len();
    }
    if !fonts().is_empty() {
        assert!(mappings > 2000, "only {mappings} characters were compared");
    }
}

/// `otfinfo -g`: the glyph names, for the font that keeps them in `post`.
///
/// A CFF font keeps its names in the CFF charset instead, which this does not
/// read -- so it says so rather than answering, and the test holds it to that.
#[test]
fn the_glyph_names_are_the_ones_otfinfo_lists() {
    let mut checked = 0usize;
    for path in fonts() {
        let font = Sfnt::open(&path).expect("reads");
        match font.glyph_names() {
            Ok(names) => {
                let Some(text) = otfinfo("-g", &path) else {
                    continue;
                };
                let want: Vec<&str> = text.lines().map(str::trim).collect();
                assert_eq!(names.len(), want.len(), "{}", path.display());
                assert_eq!(names, want, "{}", path.display());
                checked += 1;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        }
    }
    // Every font, not just the TrueType one: a CFF font's names come out of
    // its charset, which is the piece this used to be missing.
    if !fonts().is_empty() {
        assert_eq!(
            checked,
            fonts().len(),
            "only {checked} of {} fonts had their names compared",
            fonts().len()
        );
    }
}

/// A font embedded as a simple `/TrueType` font carries the glyphs its 224
/// codes can name, and no more.
///
/// This is the other half of subsetting from `subset`: that one is for a font
/// addressed by glyph id, where the file names the glyphs it wants; this one is
/// for a font addressed by CHARACTER, where what it can draw is decided by
/// WinAnsi and the font's own `cmap`. The `cmap` is what makes the difference
/// -- a glyph-addressed subset drops it and this one cannot.
#[test]
fn a_font_addressed_by_character_keeps_its_cmap_and_drops_the_rest() {
    let path = Path::new("/usr/local/texlive/2026/texmf-dist/fonts/truetype/google/arimo/Arimo-Regular.ttf");
    if !path.exists() {
        return;
    }
    let font = Sfnt::open(path).expect("reads");
    let cmap = font.cmap().expect("cmap");
    let whole = std::fs::read(path).expect("the file");

    // What WinAnsi can name, which is what such a font is asked for.
    let keep: std::collections::BTreeSet<u16> = (32u8..=255)
        .filter_map(texrs::typeset::winansi_unicode)
        .filter_map(|ch| cmap.get(&(ch as u32)).copied())
        .collect();
    assert!(keep.len() > 200, "only {} codes map anywhere", keep.len());

    let bytes = font.subset_encoded(&keep).expect("the subset builds");
    assert!(
        bytes.len() < whole.len(),
        "the subset is not smaller: {} of {}",
        bytes.len(),
        whole.len()
    );
    let subset = Sfnt::parse(bytes.clone()).expect("and reads back");

    // The `cmap` came across, and says the same things: a code still finds the
    // glyph it found, which is the whole reason this variant exists.
    let after = subset.cmap().expect("the subset has a cmap");
    for ch in ['A', 'z', '0', 'ä', '\u{2013}'] {
        assert_eq!(
            after.get(&(ch as u32)),
            cmap.get(&(ch as u32)),
            "{ch:?} moved"
        );
    }
    // The ids did not move, which is what lets the `cmap` come across unread.
    assert_eq!(
        subset.num_glyphs().expect("maxp"),
        font.num_glyphs().expect("maxp")
    );

    // A glyph nobody can name is blank, and one that was kept is not.
    let outline = |font: &Sfnt, glyph: u16| -> usize {
        let long = font.head().expect("head").long_loca;
        let loca = font.table("loca").expect("loca");
        let at = |g: usize| match long {
            true => u32::from_be_bytes([
                loca[g * 4],
                loca[g * 4 + 1],
                loca[g * 4 + 2],
                loca[g * 4 + 3],
            ]) as usize,
            false => u16::from_be_bytes([loca[g * 2], loca[g * 2 + 1]]) as usize * 2,
        };
        at(glyph as usize + 1) - at(glyph as usize)
    };
    let a = *cmap.get(&(b'A' as u32)).expect("A");
    assert!(outline(&subset, a) > 0, "A was kept and is empty");
    let dropped = (0..font.num_glyphs().expect("maxp"))
        .find(|g| !keep.contains(g) && outline(&font, *g) > 0)
        .expect("a glyph WinAnsi cannot name");
    assert_eq!(
        outline(&subset, dropped),
        0,
        "glyph {dropped} is unreachable and still in the file"
    );
}
