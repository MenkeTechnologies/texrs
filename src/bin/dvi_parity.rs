//! `cargo run --bin dvi-parity` — how close texrs's DVI output is to tex's.
//!
//! The attainable axis. DVI carries no fonts and no compression, so byte
//! equality is a goal rather than an aspiration, and the floor file works the
//! same way `tests/pdf_floor.txt` does.

use std::path::PathBuf;
use texrs::dvi_parity::{self, Rung};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(oracle) = dvi_parity::oracle() else {
        eprintln!("no `tex` on PATH — nothing to compare against");
        return;
    };
    println!("oracle: {} {}", oracle.program, oracle.version);

    let floor = read_floor(&root.join("tests/dvi_floor.txt"));
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
        let reference = dvi_parity::reference(&oracle, case);
        let subject = dvi_parity::subject(case);
        let (rung, detail) = dvi_parity::verdict(reference.as_ref(), subject.as_ref());
        let was = floor.iter().find(|(n, _)| *n == name).map(|(_, r)| *r);
        let mark = match was {
            Some(w) if rung < w => {
                dropped += 1;
                "DROPPED"
            }
            Some(w) if rung > w => "CLIMBED",
            _ => "",
        };
        println!("{:<10} {:<26} {mark} {detail}", rung.name(), name);
        lines.push(format!("{} {}", rung.name(), name));
    }

    if record {
        let body = format!(
            "# How far each document's DVI matches tex's, highest rung reached.\n\
             # Rungs: NONE < PARSES < PAGES < TEXT < STRUCTURE < BYTES.\n\
             # BYTES is the goal and a reachable one: DVI carries no fonts and\n\
             # no compression. Re-record with `cargo run --bin dvi-parity -- --record`.\n{}\n",
            lines.join("\n")
        );
        let _ = std::fs::write(root.join("tests/dvi_floor.txt"), body);
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
