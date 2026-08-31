//! `cargo run --bin pdf-parity` — how close texrs's PDF output is to LuaTeX's.
//!
//! The goal is byte-identical. Nothing reaches it yet, so this reports the rung
//! each document does reach and compares it with the floor recorded in
//! `tests/pdf_floor.txt`. A case that climbs is progress to be re-recorded; a
//! case that drops is a regression, and the exit status says so.

use std::path::PathBuf;
use texrs::pdf_parity::{self, Rung};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(oracle) = pdf_parity::oracle() else {
        eprintln!("no `luatex` on PATH — nothing to compare against");
        return;
    };
    println!("oracle: {} {}", oracle.program, oracle.version);

    let floor = read_floor(&root.join("tests/pdf_floor.txt"));
    let record = std::env::args().any(|a| a == "--record");

    let dir = root.join("tests/pdf_cases");
    let mut cases: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "tex"))
                .collect()
        })
        .unwrap_or_default();
    cases.sort();

    let mut lines = Vec::new();
    let mut dropped = 0;
    for case in &cases {
        let name = case
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let reference = pdf_parity::reference(&oracle, case);
        let subject = pdf_parity::subject(case);
        let (rung, detail) = pdf_parity::verdict(reference.as_ref(), subject.as_ref());
        let was = floor.iter().find(|(n, _)| *n == name).map(|(_, r)| *r);
        let mark = match was {
            Some(w) if rung < w => {
                dropped += 1;
                "DROPPED"
            }
            Some(w) if rung > w => "CLIMBED",
            _ => "",
        };
        println!("{:<9} {:<24} {mark} {detail}", rung.name(), name);
        lines.push(format!("{} {}", rung.name(), name));
    }

    if record {
        let body = format!(
            "# How far each document's PDF matches LuaTeX's, highest rung reached.\n\
             # Rungs: NONE < PRODUCED < PAGES < PAGESIZE < TEXT < BYTES.\n\
             # BYTES is the goal. Re-record with `cargo run --bin pdf-parity -- --record`\n\
             # after a change that climbs; a case that DROPS is a regression.\n{}\n",
            lines.join("\n")
        );
        let _ = std::fs::write(root.join("tests/pdf_floor.txt"), body);
        println!("\nrecorded {} case(s)", lines.len());
        return;
    }

    println!(
        "\n{} case(s); {} at the goal, {} dropped below the floor",
        cases.len(),
        lines.iter().filter(|l| l.starts_with("BYTES")).count(),
        dropped
    );
    if dropped > 0 {
        std::process::exit(1);
    }
}

/// The recorded rung per case.
fn read_floor(path: &std::path::Path) -> Vec<(String, Rung)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (rung, name) = l.split_once(' ')?;
            Some((name.trim().to_string(), Rung::parse(rung)?))
        })
        .collect()
}
