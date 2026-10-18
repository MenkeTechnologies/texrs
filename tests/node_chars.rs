//! texrs's node-mode font loading against luaotfload's, character by
//! character.
//!
//! `scripts/node-chars-freeze.sh` loads each font with luahbtex in
//! `mode=node` at 10pt and records the characters table luaotfload hands the
//! engine (`tests/data/node_chars.txt`): every slot -- Unicode, duplicate
//! copies and the private slots from U+F0000 -- with its width, height,
//! depth and glyph index, and the font's parameters. This loads the same file
//! with `texrs::otf` and requires the same table.
//!
//! The `/ToUnicode` column is recorded and not yet compared: for a glyph no
//! Unicode reaches, luaotfload derives it from the GSUB lookups
//! (font-oup.lua's `checklookups` and `getcomponents`), which are not read
//! yet. The fonts are TeX Live's, found through kpsewhich; a machine without
//! them skips.

use std::collections::BTreeMap;

struct Font {
    file: String,
    params: Vec<(String, String)>,
    chars: BTreeMap<u32, (String, String, String, u32)>,
}

fn expected() -> Vec<Font> {
    let mut out: Vec<Font> = Vec::new();
    for line in include_str!("data/node_chars.txt").lines() {
        if let Some(file) = line.strip_prefix("##### ") {
            out.push(Font {
                file: file.to_string(),
                params: Vec::new(),
                chars: BTreeMap::new(),
            });
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let font = out.last_mut().expect("a font header first");
        let w: Vec<&str> = line.split(' ').collect();
        if w[0] == "param" {
            font.params.push((w[1].to_string(), w[2].to_string()));
            continue;
        }
        let slot = u32::from_str_radix(w[0], 16).unwrap();
        font.chars.insert(
            slot,
            (
                w[1].to_string(),
                w[2].to_string(),
                w[3].to_string(),
                w[4].parse().unwrap(),
            ),
        );
    }
    out
}

fn r(x: f64) -> String {
    texrs::otf::tfm::round(x).to_string()
}

#[test]
fn node_mode_characters_match_luaotfload() {
    let mut failures = Vec::new();
    for font in expected() {
        let Some(path) = texrs::kpse::locate(&font.file) else {
            eprintln!("skipping {}: not installed", font.file);
            continue;
        };
        let face = texrs::otf::Face::open(&path, 0).unwrap();
        let s = face.scale(655_360);
        let p = &s.parameters;
        let got_params: Vec<(String, String)> = [
            ("slant", r(p.slant)),
            ("space", r(p.space)),
            ("space_stretch", r(p.space_stretch)),
            ("space_shrink", r(p.space_shrink)),
            ("x_height", r(p.x_height)),
            ("quad", r(p.quad)),
            ("extra_space", r(p.extra_space)),
            ("8", p.capheight.map_or("nil".into(), r)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if got_params != font.params {
            failures.push(format!(
                "{}: parameters\n  luatex {:?}\n  texrs  {:?}",
                font.file, font.params, got_params
            ));
        }
        let got: BTreeMap<u32, (String, String, String, u32)> = s
            .characters
            .iter()
            .map(|(&k, c)| {
                (
                    k,
                    (
                        r(c.width),
                        r(c.height),
                        c.depth.map_or("nil".into(), r),
                        c.index,
                    ),
                )
            })
            .collect();
        let missing: Vec<String> = font
            .chars
            .keys()
            .filter(|k| !got.contains_key(k))
            .map(|k| format!("{k:X}"))
            .collect();
        let extra: Vec<String> = got
            .keys()
            .filter(|k| !font.chars.contains_key(k))
            .map(|k| format!("{k:X}"))
            .collect();
        let differing: Vec<String> = font
            .chars
            .iter()
            .filter_map(|(k, v)| {
                let g = got.get(k)?;
                (g != v).then(|| format!("{k:X}: luatex {v:?} texrs {g:?}"))
            })
            .take(10)
            .collect();
        if !missing.is_empty() || !extra.is_empty() || !differing.is_empty() {
            failures.push(format!(
                "{}: {} slots luatex has and texrs not {:?}, {} the other way {:?}, differing:\n  {}",
                font.file,
                missing.len(),
                &missing[..missing.len().min(10)],
                extra.len(),
                &extra[..extra.len().min(10)],
                differing.join("\n  ")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
