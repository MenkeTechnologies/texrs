//! texrs's CFF subsets against the `/FontFile3` luatex embeds.
//!
//! `scripts/cffsubset-freeze.sh` sets a page per OpenType CFF font with
//! luahbtex in mode=node, writes it uncompressed, and keeps the font's
//! `/FontFile3` and `/CIDSet` streams and its `/FontName`
//! (`tests/data/cffsubset/`). The CIDSet names the glyphs the page used;
//! this subsets the same font file to those glyphs with
//! `texrs::otf::cffsubset` under the same name and requires luatex's font
//! program byte for byte, and its CIDSet. Both of luatex's writers are
//! covered: name-keyed fonts become CID-keyed, CID-keyed ones keep their
//! FDArray. The fonts are TeX Live's; a machine without them skips.

use std::collections::BTreeSet;

#[test]
fn cff_subsets_are_luatexs() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/cffsubset");
    let mut failures = Vec::new();
    for name in [
        "lmroman10-regular",
        "texgyretermes-regular",
        "STIXTwoMath-Regular",
        "HaranoAjiMincho-Regular",
    ] {
        let Some(path) = texrs::kpse::locate(&format!("{name}.otf")) else {
            eprintln!("skipping {name}: not installed");
            continue;
        };
        let font = std::fs::read(path).unwrap();
        let expected = std::fs::read(dir.join(format!("{name}.fontfile"))).unwrap();
        let cidset = std::fs::read(dir.join(format!("{name}.cidset"))).unwrap();
        let fontname = std::fs::read_to_string(dir.join(format!("{name}.fontname"))).unwrap();
        let (tag, base) = fontname.trim().split_once('+').unwrap();
        let used: BTreeSet<u16> = (1..cidset.len() * 8)
            .filter(|&c| cidset[c / 8] & (1 << (7 - c % 8)) != 0)
            .map(|c| c as u16)
            .collect();
        let got = match texrs::otf::cffsubset::make_cff_subset(&font, 1, &used, tag, base) {
            Ok(got) => got,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        if got.cidset != cidset {
            failures.push(format!("{name}: CIDSet differs"));
        }
        if got.fontfile != expected {
            if let Ok(out) = std::env::var("TEXRS_DUMP_DIR") {
                std::fs::write(format!("{out}/{name}.cff"), &got.fontfile).unwrap();
            }
            let at = got
                .fontfile
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(got.fontfile.len().min(expected.len()));
            failures.push(format!(
                "{name}: font program differs at byte {at} ({} bytes from luatex, {} here)",
                expected.len(),
                got.fontfile.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
