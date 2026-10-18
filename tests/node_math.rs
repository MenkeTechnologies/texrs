//! texrs's OpenType math font loading against luaotfload's.
//!
//! `scripts/node-math-freeze.sh` loads each math font with luahbtex as
//! unicode-math does (`mode=base;script=math`) at 10pt and records what the
//! engine then holds (`tests/data/node_math.txt`): the MathConstants, and per
//! character the next larger variant, the italic correction, the top accent
//! position, the extensible recipe and the math kerns. Latin Modern Math is
//! recorded line by line, the others as a SHA-256 of the same lines. This
//! loads the same files with `texrs::otf` and requires the same.
//!
//! The fonts are TeX Live's, found through kpsewhich; a machine without them
//! skips.

use sha2::{Digest, Sha256};
use texrs::otf::tfm::{self, MathKern, Options, Part};

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
    for line in include_str!("data/node_math.txt").lines() {
        if let Some(file) = line.strip_prefix("##### ") {
            out.push(Expected::Full {
                file: file.to_string(),
                lines: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("digest ") {
            let w: Vec<&str> = rest.split(' ').collect();
            out.push(Expected::Digest {
                file: w[0].to_string(),
                sha: w[1].to_string(),
                lines: w[2].parse().unwrap(),
            });
        } else if !line.starts_with('#') {
            if let Some(Expected::Full { lines, .. }) = out.last_mut() {
                lines.push(line.to_string());
            }
        }
    }
    out
}

fn r(x: f64) -> i64 {
    tfm::round(x)
}

fn parts(v: &[Part]) -> String {
    v.iter()
        .map(|p| {
            format!(
                "{:X}:{}:{}:{}:{}",
                p.glyph,
                r(p.start),
                r(p.end),
                r(p.advance),
                if p.extender { "1" } else { "nil" }
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn kerns(v: &[MathKern]) -> String {
    v.iter()
        .map(|k| format!("{}/{}", r(k.height), r(k.kern)))
        .collect::<Vec<_>>()
        .join(",")
}

/// The dump's lines for the font at `path`, as the freeze script writes them.
fn dump(path: &std::path::Path) -> Vec<String> {
    let face = texrs::otf::Face::open(path, 0).unwrap();
    let opts = Options {
        math: true,
        ..Options::text()
    };
    let s = face.scale_with(655_360, &opts);
    let mut out: Vec<String> = s
        .math_constants
        .iter()
        .flatten()
        .map(|(k, v)| format!("const {k} {}", r(*v)))
        .collect();
    for (k, c) in &s.characters {
        let mut p = Vec::new();
        if let Some(n) = c.next {
            p.push(format!("next {n:X}"));
        }
        if let Some(i) = c.italic {
            p.push(format!("italic {}", r(i)));
        }
        if let Some(a) = c.top_accent {
            p.push(format!("accent {}", r(a)));
        }
        if let Some(v) = &c.vert_variants {
            p.push(format!("vert_variants {}", parts(v)));
        }
        if let Some(v) = &c.horiz_variants {
            p.push(format!("horiz_variants {}", parts(v)));
        }
        if let Some(m) = &c.mathkern {
            for (name, v) in [
                ("top_right", &m.top_right),
                ("top_left", &m.top_left),
                ("bottom_right", &m.bottom_right),
                ("bottom_left", &m.bottom_left),
            ] {
                if let Some(v) = v {
                    p.push(format!("{name} {}", kerns(v)));
                }
            }
        }
        if !p.is_empty() {
            out.push(format!("{k:X} {}", p.join(" ")));
        }
    }
    out
}

#[test]
fn math_fonts_match_luaotfload() {
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
                        "{file}: differs from luatex ({lines} lines there, {} here)",
                        got.len()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
