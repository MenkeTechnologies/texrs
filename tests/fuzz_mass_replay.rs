//! Widen the fuzz targets from their seed corpus to every `.tex` in the tree,
//! plus deterministic mutations of each one.
//!
//! `tests/fuzz_smoke.rs` proves the harness still builds; this proves the engine
//! survives inputs nobody wrote by hand. The mutations are generated from a
//! fixed LCG, so a failure here reproduces byte-for-byte on any machine without
//! a corpus artifact having to be committed.
//!
//! Only the MOUTH is pointed at mutated input. The mouth is a straight scan and
//! always terminates; expansion does not -- a mutation can turn `\def\a{\b}`
//! into a macro that expands to itself, which loops in texrs exactly as it loops
//! in real tex. A test suite that can hang is worse than no test, so lowering
//! and execution replay only the committed, hand-written files.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use texrs::catcode::CatTable;
use texrs::lexer::Lexer;

fn tex_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for sub in ["tests/cases", "fuzz/corpus", "examples", "docs"] {
        collect(&root.join(sub), &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "tex") {
            out.push(p);
        }
    }
}

/// The mutations applied to every file. Each is a whole-input transform rather
/// than a random walk, so the set is small, named, and reproducible -- when one
/// fails, the report says which transform produced the input.
fn mutations(src: &str, seed: u64) -> Vec<(&'static str, String)> {
    let bytes: Vec<char> = src.chars().collect();
    let mut rng = Lcg(seed);
    let cut = |n: usize| -> String { bytes.iter().take(n).collect() };
    let mut v = vec![
        ("truncate-half", cut(bytes.len() / 2)),
        ("truncate-quarter", cut(bytes.len() / 4)),
        ("truncate-last-char", cut(bytes.len().saturating_sub(1))),
        ("no-newlines", src.replace('\n', " ")),
        ("no-backslashes", src.replace('\\', "")),
        ("no-braces", src.replace(['{', '}'], "")),
        (
            "doubled-specials",
            src.replace('^', "^^").replace('#', "##"),
        ),
        ("appended-caret", format!("{src}^^")),
        ("appended-backslash", format!("{src}\\")),
        ("appended-lone-brace", format!("{src}{{")),
    ];
    // Three random single-character splices per file: whatever the named
    // transforms above did not think of.
    for _ in 0..3 {
        let at = match bytes.is_empty() {
            true => 0,
            false => (rng.next() as usize) % bytes.len(),
        };
        let c = [
            '\\', '{', '}', '#', '^', '$', '%', '&', '~', '\r', '\n', '\0', '\u{7f}', 'é',
        ][(rng.next() as usize) % 14];
        let mut s: String = bytes.iter().take(at).collect();
        s.push(c);
        s.extend(bytes.iter().skip(at));
        v.push(("splice", s));
    }
    v
}

/// The same LCG constants glibc uses, so the sequence is fixed by the seed and
/// not by the platform's `rand`.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u32
    }
}

fn lex_to_end(src: &str) {
    let cats = CatTable::new();
    let mut lx = Lexer::new(src);
    while lx.next_token(&cats).is_some() {}
}

/// Run `f` over every input, collecting panics instead of dying on the first, so
/// one report names every offending input.
fn report<F: Fn(&str)>(inputs: &[(String, String)], f: F) {
    let mut bad = Vec::new();
    for (label, src) in inputs {
        if catch_unwind(AssertUnwindSafe(|| f(src))).is_err() {
            bad.push(label.clone());
        }
    }
    assert!(
        bad.is_empty(),
        "{} input(s) panicked:\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}

#[test]
fn the_mouth_survives_every_mutation_of_every_case() {
    let files = tex_files();
    assert!(!files.is_empty(), "no .tex files found to replay");
    let mut inputs = Vec::new();
    for (i, path) in files.iter().enumerate() {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        inputs.push((name.clone(), src.clone()));
        for (kind, mutated) in mutations(&src, i as u64 + 1) {
            inputs.push((format!("{name} [{kind}]"), mutated));
        }
    }
    assert!(
        inputs.len() > 100,
        "mass replay degenerated to {} inputs -- the corpus walk is broken",
        inputs.len()
    );
    report(&inputs, lex_to_end);
}

#[test]
fn the_frontend_survives_every_committed_case() {
    let inputs: Vec<(String, String)> = tex_files()
        .iter()
        .filter_map(|p| {
            let src = std::fs::read_to_string(p).ok()?;
            Some((p.file_name()?.to_string_lossy().into_owned(), src))
        })
        .collect();
    assert!(!inputs.is_empty(), "no .tex files found to replay");
    report(&inputs, |s| {
        let _ = texrs::compile(s);
    });
    report(&inputs, |s| {
        let _ = texrs::run_messages(s);
    });
}
