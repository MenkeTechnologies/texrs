//! texrs's node-mode font loading against luaotfload's, character by
//! character.
//!
//! `scripts/node-chars-freeze.sh` loads each font with luahbtex in
//! `mode=node` at 10pt and records the characters table luaotfload hands the
//! engine (`tests/data/node_chars.txt`): every slot -- Unicode, duplicate
//! copies and the private slots from U+F0000 -- with its width, height,
//! depth, glyph index and `/ToUnicode`, and the font's parameters. Three fonts
//! are recorded line by line, the larger ones as a SHA-256 of the same lines.
//! This loads the same files with `texrs::otf` and requires the same table.
//!
//! The fonts are TeX Live's, found through kpsewhich; a machine without them
//! skips.

use sha2::{Digest, Sha256};

enum Expected {
    Full {
        file: String,
        lines: Vec<String>,
    },
    Digest {
        file: String,
        sha: String,
        lines: usize,
    },
}

fn expected() -> Vec<Expected> {
    let mut out: Vec<Expected> = Vec::new();
    for line in include_str!("data/node_chars.txt").lines() {
        if let Some(file) = line.strip_prefix("##### ") {
            out.push(Expected::Full {
                file: file.to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("digest ") {
            let w: Vec<&str> = rest.split(' ').collect();
            out.push(Expected::Digest {
                file: w[0].to_string(),
                sha: w[1].to_string(),
                lines: w[2].parse().unwrap(),
            });
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(Expected::Full { lines, .. }) = out.last_mut() {
            lines.push(line.to_string());
        }
    }
    out
}

fn r(x: f64) -> String {
    texrs::otf::tfm::round(x).to_string()
}

/// The dump's lines for the font at `path`, as the freeze script writes them.
fn dump(path: &std::path::Path) -> Vec<String> {
    let face = texrs::otf::Face::open(path, 0).unwrap();
    let s = face.scale(655_360);
    let p = &s.parameters;
    let mut out: Vec<String> = [
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
    .map(|(k, v)| format!("param {k} {v}"))
    .collect();
    for (k, c) in &s.characters {
        out.push(format!(
            "{k:X} {} {} {} {} {}",
            r(c.width),
            r(c.height),
            c.depth.map_or("nil".into(), r),
            c.index,
            c.tounicode
        ));
    }
    out
}

#[test]
fn node_mode_characters_match_luaotfload() {
    let mut failures = Vec::new();
    for e in expected() {
        let file = match &e {
            Expected::Full { file, .. } | Expected::Digest { file, .. } => file.clone(),
        };
        let Some(path) = texrs::kpse::locate(&file) else {
            eprintln!("skipping {file}: not installed");
            continue;
        };
        let got = dump(&path);
        match e {
            Expected::Full { lines, .. } => {
                if got != lines {
                    let diffs: Vec<String> = lines
                        .iter()
                        .zip(&got)
                        .filter(|(a, b)| a != b)
                        .take(10)
                        .map(|(a, b)| format!("  luatex {a}\n  texrs  {b}"))
                        .collect();
                    failures.push(format!(
                        "{file}: {} lines from luatex, {} from texrs\n{}",
                        lines.len(),
                        got.len(),
                        diffs.join("\n")
                    ));
                }
            }
            Expected::Digest { sha, lines, .. } => {
                let mut h = Sha256::new();
                for l in &got {
                    h.update(l.as_bytes());
                    h.update(b"\n");
                }
                let mine: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
                if mine != sha {
                    failures.push(format!(
                        "{file}: the table differs from luatex's ({} lines there, {} here); \
                         dump both in full to see where",
                        lines,
                        got.len()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
