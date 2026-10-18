//! texrs's TrueType subsets against the `/FontFile2` luatex embeds.
//!
//! `scripts/ttsubset-freeze.sh` sets a page per font with luahbtex in
//! mode=node, writes it uncompressed, and keeps the font's `/FontFile2` and
//! `/CIDSet` streams (`tests/data/ttsubset/`). The CIDSet names the glyphs
//! the page used; this subsets the same font file to those glyphs with
//! `texrs::otf::ttsubset` and requires luatex's font program byte for byte,
//! and its CIDSet. The fonts are TeX Live's; a machine without them skips.

use std::collections::BTreeSet;

#[test]
fn truetype_subsets_are_luatexs() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/ttsubset");
    let mut failures = Vec::new();
    for name in ["DejaVuSans", "Arimo-Regular", "NotoSerif-Regular"] {
        let Some(path) = texrs::kpse::locate(&format!("{name}.ttf")) else {
            eprintln!("skipping {name}: not installed");
            continue;
        };
        let font = std::fs::read(path).unwrap();
        let expected = std::fs::read(dir.join(format!("{name}.fontfile"))).unwrap();
        let cidset = std::fs::read(dir.join(format!("{name}.cidset"))).unwrap();
        let used: BTreeSet<u16> = (1..cidset.len() * 8)
            .filter(|&c| cidset[c / 8] & (1 << (7 - c % 8)) != 0)
            .map(|c| c as u16)
            .collect();
        let got = texrs::otf::ttsubset::make_tt_subset(&font, 1, &used).unwrap();
        if got.cidset != cidset {
            failures.push(format!("{name}: CIDSet differs"));
        }
        if got.fontfile != expected {
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
