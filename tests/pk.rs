//! The packed-font decoder against `gftype`'s picture of the same font.
//!
//! A `.pk` glyph is three encodings in one nybble stream -- one nybble for a
//! small run, two for a middling one, a run of zeros for a large one, and a
//! repeat count that arrives mid-row and applies when the row ends. Every one
//! of those can be got slightly wrong in a way that still produces a plausible
//! glyph. So the test is not "does an A look like an A": it is every pixel of
//! every character, against the picture `gftype` prints for the same font.
//!
//! `gftype` reads GF, not PK, so `pktogf` converts first. Both are Knuth's and
//! Rokicki's own programs, and they take a different path through the format
//! than this does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use texrs::pk::Pk;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs_pk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A packed font from the installation, at a resolution it ships at.
fn installed(name: &str) -> Option<PathBuf> {
    let found = Command::new("kpsewhich")
        .arg("-format=pk")
        .arg(name)
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&found.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// `gftype`'s picture of every character: the rows it prints between the two
/// corner markers, right-trimmed.
fn pictures(pk: &std::path::Path, dir: &std::path::Path) -> Option<BTreeMap<u32, Vec<String>>> {
    let gf = dir.join("f.gf");
    let made = Command::new("pktogf").arg(pk).arg(&gf).output().ok()?;
    if !made.status.success() {
        return None;
    }
    // -i asks for the picture, -m for the mnemonics that name each character.
    let out = Command::new("gftype")
        .arg("-i")
        .arg("-m")
        .arg(&gf)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();

    let mut all = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut rows: Vec<String> = Vec::new();
    let mut in_picture = false;
    for line in text.lines() {
        if let Some(rest) = line.split("beginning of char ").nth(1) {
            let code: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            current = code.parse().ok();
            rows.clear();
            in_picture = false;
            continue;
        }
        // The picture is bracketed by two lines marking its corners.
        if line.starts_with(".<--") {
            match in_picture {
                false => in_picture = true,
                true => {
                    in_picture = false;
                    if let Some(code) = current.take() {
                        all.insert(code, std::mem::take(&mut rows));
                    }
                }
            }
            continue;
        }
        if in_picture {
            rows.push(line.trim_end().to_string());
        }
    }
    Some(all)
}

/// Every pixel of every character of the packed fonts TeX Live ships.
#[test]
fn every_pixel_is_the_pixel_gftype_draws() {
    // A text font, an italic, a symbol font and a typewriter font: between
    // them they use every size of packed number and plenty of repeated rows.
    let fonts = [
        "cmr10.600pk",
        "cmti10.600pk",
        "cmsy10.600pk",
        "cmtt10.600pk",
    ];
    let dir = scratch();
    let mut characters = 0usize;
    let mut pixels = 0usize;
    for name in fonts {
        let Some(path) = installed(name) else {
            continue;
        };
        let Some(want) = pictures(&path, &dir) else {
            continue;
        };
        let pk = Pk::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!want.is_empty(), "{name}: gftype drew nothing");

        for (&code, rows) in &want {
            let glyph = pk
                .glyph(code)
                .unwrap_or_else(|| panic!("{name}: no character {code}"));
            assert_eq!(
                glyph.height,
                rows.len(),
                "{name} character {code}: {} rows, gftype draws {}",
                glyph.height,
                rows.len()
            );
            // gftype right-trims its rows, so this does too; a difference in
            // the trailing white would show as a difference in the ink beside
            // it.
            let mine: Vec<String> = glyph
                .rows()
                .iter()
                .map(|row| row.trim_end().to_string())
                .collect();
            assert_eq!(&mine, rows, "{name} character {code}");
            characters += 1;
            pixels += glyph.width * glyph.height;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    // If there is a TeX here at all, this compared a real font rather than
    // passing on an empty list.
    if installed("cmr10.600pk").is_some() {
        assert!(
            characters > 300,
            "only {characters} characters were compared"
        );
        assert!(pixels > 500_000, "only {pixels} pixels were compared");
    }
}

/// The measurements beside the pixels: what TeX sets the character with, and
/// where the reference point sits.
#[test]
fn a_glyph_knows_where_its_reference_point_is() {
    let Some(path) = installed("cmr10.600pk") else {
        return;
    };
    let pk = Pk::open(&path).expect("cmr10 reads");

    // The .pk's own width for a character is the .tfm's, so the two agree --
    // which is what lets a driver mix an outline font and a bitmap one.
    let Ok(tfm) = texrs::tfm::Tfm::open(
        String::from_utf8_lossy(
            &Command::new("kpsewhich")
                .arg("cmr10.tfm")
                .output()
                .expect("kpsewhich")
                .stdout,
        )
        .trim(),
    ) else {
        return;
    };
    let mut compared = 0usize;
    for code in pk.codes() {
        let glyph = pk.glyph(code).expect("a glyph");
        let Some(metrics) = tfm.char(code as u8) else {
            continue;
        };
        assert!(
            (glyph.tfm_width - metrics.width).abs() < 1e-6,
            "character {code}: the .pk says {}, the .tfm says {}",
            glyph.tfm_width,
            metrics.width
        );
        compared += 1;
    }
    assert!(compared > 100, "only {compared} widths were compared");

    // A letter with a descender reaches below the baseline, and one without
    // does not: the offset says where the baseline is.
    let p = pk.glyph(b'p' as u32).expect("a p");
    let n = pk.glyph(b'n' as u32).expect("an n");
    assert!(
        p.height as i32 - p.y_offset > n.height as i32 - n.y_offset,
        "a p descends and an n does not"
    );
}
