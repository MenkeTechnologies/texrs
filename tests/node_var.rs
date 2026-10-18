//! texrs's loading of a variable font at an instance against luaotfload's.
//!
//! `scripts/node-var-freeze.sh` loads each font with luahbtex in `mode=node`
//! at 10pt with `axis=` values and records a SHA-256 of its characters table
//! and parameters, the lines `tests/node_chars.rs` compares
//! (`tests/data/node_var.txt`). This loads the same file at the same
//! instance with `texrs::otf` and requires the same digest. PennstanderVF is
//! TeX Live's (found through kpsewhich); the Arimo and Orbitron files are the
//! publication corpus's, found next to this repository when that checkout is
//! there. A missing font is skipped.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn r(x: f64) -> String {
    texrs::otf::tfm::round(x).to_string()
}

fn locate(file: &str) -> Option<PathBuf> {
    if let Some(p) = texrs::kpse::locate(file) {
        return Some(p);
    }
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../MenkeTechnologiesPublications/strykelang/docs/.fonts")
        .join(file);
    corpus.exists().then_some(corpus)
}

fn dump(path: &std::path::Path, instance: Option<&str>) -> Vec<String> {
    let face = texrs::otf::Face::open_instance(path, 0, instance).unwrap();
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
fn variable_instances_match_luaotfload() {
    let mut failures = Vec::new();
    for line in include_str!("data/node_var.txt").lines() {
        let Some(rest) = line.strip_prefix("digest ") else {
            continue;
        };
        let w: Vec<&str> = rest.split(' ').collect();
        let (file, axis) = w[0].split_once('|').unwrap();
        let (sha, lines) = (w[1], w[2]);
        let Some(path) = locate(file) else {
            eprintln!("skipping {file}: not here");
            continue;
        };
        let got = dump(&path, (!axis.is_empty()).then_some(axis));
        let mut h = Sha256::new();
        for l in &got {
            h.update(l.as_bytes());
            h.update(b"\n");
        }
        let mine: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if mine != sha {
            if let Ok(dir) = std::env::var("TEXRS_DUMP_DIR") {
                let name = format!(
                    "{dir}/{}-{}.texrs",
                    file.trim_end_matches(".ttf"),
                    if axis.is_empty() { "default" } else { axis }
                );
                std::fs::write(name, got.join("\n") + "\n").unwrap();
            }
            failures.push(format!(
                "{file} at {axis:?}: differs ({lines} lines from luatex, {} here)",
                got.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
