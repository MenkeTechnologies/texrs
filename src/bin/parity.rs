//! `parity` — run the committed corpus against real `tex` and report.
//!
//! The same comparison `cargo test --test differential` makes, in the form a
//! person wants when they are working on the engine: a line per case, the
//! divergences printed with both streams, and an exit status that is the number
//! of unlisted divergences.
//!
//! Both read one implementation of the oracle (`texrs::parity`), so the report
//! you get by hand and the verdict CI gets cannot disagree. The shell script
//! this replaces kept that logic in bash and perl alongside the Rust copy,
//! which is the arrangement that drifts.
//!
//! ```sh
//! cargo run --bin parity                 # the corpus in tests/cases
//! cargo run --bin parity -- --freeze     # record what tex says, for CI
//! cargo run --bin parity -- examples     # any directory of .tex files
//! cargo run --bin parity -- doc.tex      # one file
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use texrs::parity::{self, Verdict};

fn main() -> ExitCode {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let oracle = match parity::oracle(&repo) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("parity: {e}");
            return ExitCode::from(2);
        }
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--freeze") {
        // Both corpora, because both are checked against tex and both go
        // unchecked in CI without this: the cases, and the examples that are
        // documentation and must run.
        for (dirs, label) in [
            (vec!["tests/cases"], "parity"),
            // The extensions are examples too: they must run, and freezing what
            // tex made of a construct it does not have is what lets the replay
            // tell "this is an extension" from "this example broke".
            (vec!["examples", "examples/extensions"], "examples"),
        ] {
            let mut cases: Vec<std::path::PathBuf> = dirs
                .iter()
                .flat_map(|d| parity::cases_in(&repo.join(d)))
                .collect();
            cases.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
            let out = parity::frozen_path(&repo, label);
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let text = parity::freeze(&oracle, &cases);
            if let Err(e) = std::fs::write(&out, &text) {
                eprintln!("parity: cannot write {}: {e}", out.display());
                return ExitCode::from(2);
            }
            println!(
                "froze {} {label} case(s) of tex {} -> {}",
                cases.len(),
                oracle.version,
                out.display()
            );
        }
        return ExitCode::SUCCESS;
    }

    let arg = args.into_iter().next();
    let cases: Vec<PathBuf> = match arg.as_deref() {
        None => parity::cases_in(&repo.join("tests/cases")),
        Some(p) if Path::new(p).is_dir() => parity::cases_in(Path::new(p)),
        Some(p) => vec![PathBuf::from(p)],
    };
    if cases.is_empty() {
        eprintln!("parity: no .tex files to compare");
        return ExitCode::from(2);
    }

    println!("oracle: tex {}\n", oracle.version);
    let known = parity::known_gaps(&repo);
    let (mut ok, mut diverged, mut stale) = (0usize, 0usize, 0usize);
    for case in &cases {
        let name = case.file_name().unwrap_or_default().to_string_lossy();
        match parity::verdict(&oracle, case, &known) {
            Verdict::Parity => {
                ok += 1;
                println!("PARITY   {name}");
            }
            // A written-down gap is not a failure here, exactly as it is not
            // one in tests/differential.rs -- the two must agree about what the
            // corpus claims.
            Verdict::Known => {
                ok += 1;
                println!("KNOWN    {name}");
            }
            Verdict::Stale => {
                stale += 1;
                println!("STALE    {name} (passes -- remove it from tests/known_gaps.txt)");
            }
            Verdict::Diverges { want, got } => {
                diverged += 1;
                println!("DIVERGES {name}\n  tex   : [{want}]\n  texrs : [{got}]");
            }
        }
    }

    println!(
        "\n{ok}/{} accounted for (in parity, or a written-down gap)",
        cases.len()
    );
    if stale > 0 {
        println!("{stale} known-gap entr(y/ies) now pass and must be removed");
    }
    match diverged + stale {
        0 => ExitCode::SUCCESS,
        n => ExitCode::from(u8::try_from(n.min(250)).unwrap_or(250)),
    }
}
