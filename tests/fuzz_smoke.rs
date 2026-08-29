//! Stable-Rust replay of every cargo-fuzz target on its committed seed corpus.
//!
//! cargo-fuzz needs nightly, so nothing in `fuzz/` is exercised by `cargo test`
//! on its own -- the harness rots quietly until the day someone needs it. These
//! tests mirror each target's body exactly against `fuzz/corpus/<target>/`, so a
//! signature change or a seed that starts panicking fails CI on stable.
//!
//! Adding a target under `fuzz/fuzz_targets/` means adding the matching
//! `<target>_corpus_does_not_panic` here; `every_fuzz_target_has_a_smoke_test`
//! fails if that is forgotten.

use std::path::{Path, PathBuf};
use texrs::catcode::CatTable;
use texrs::lexer::Lexer;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The committed seeds in `fuzz/corpus/<target>`, sorted so a failure names the
/// same file on every machine.
///
/// Only `.tex` files: libfuzzer grows the same directories with hash-named,
/// frequently non-UTF-8 inputs of its own while it runs, and those are its
/// working state rather than anything this repository committed.
fn corpus(target: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let dir = root().join("fuzz/corpus").join(target);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "tex"))
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "fuzz/corpus/{target} must carry seed files -- an empty corpus makes \
         this test vacuous while still passing"
    );
    entries
        .into_iter()
        .map(|p| {
            let b = std::fs::read(&p).expect("read seed");
            (p, b)
        })
        .collect()
}

fn utf8(path: &Path, bytes: &[u8]) -> String {
    std::str::from_utf8(bytes)
        .unwrap_or_else(|_| panic!("seed must be valid UTF-8: {}", path.display()))
        .to_string()
}

#[test]
fn lex_corpus_does_not_panic() {
    for (path, bytes) in corpus("lex") {
        let s = utf8(&path, &bytes);
        let cats = CatTable::new();
        let mut lx = Lexer::new(&s);
        while lx.next_token(&cats).is_some() {}
    }
}

#[test]
fn lower_corpus_does_not_panic() {
    for (path, bytes) in corpus("lower") {
        let s = utf8(&path, &bytes);
        let _ = texrs::compile(&s);
    }
}

#[test]
fn run_corpus_does_not_panic() {
    for (path, bytes) in corpus("run") {
        let s = utf8(&path, &bytes);
        let _ = texrs::run_messages(&s);
    }
}

/// The corpus seeds are hand-picked, so they should all be programs texrs can
/// actually compile. A seed that stopped compiling is either a regression or a
/// seed that was never doing any work; both are worth failing over.
///
/// `crash_*` is the exception: those are inputs the fuzzer found a PANIC on, and
/// they are kept precisely because they must not compile -- they must fail with
/// a `TexError` instead of taking the process down.
#[test]
fn lower_and_run_seeds_are_real_programs() {
    for target in ["lower", "run"] {
        for (path, bytes) in corpus(target) {
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("crash_"))
            {
                let s = utf8(&path, &bytes);
                assert!(
                    texrs::run_messages(&s).is_err(),
                    "{} was kept as a crash regression but now compiles cleanly -- \
                     rename it if that is deliberate",
                    path.display()
                );
                continue;
            }
            let s = utf8(&path, &bytes);
            if let Err(e) = texrs::run_messages(&s) {
                panic!("seed no longer compiles: {} -- {}", path.display(), e.0);
            }
        }
    }
}

/// A target with no smoke test is a target nothing on stable ever builds.
#[test]
fn every_fuzz_target_has_a_smoke_test() {
    let targets: Vec<String> = std::fs::read_dir(root().join("fuzz/fuzz_targets"))
        .expect("fuzz/fuzz_targets")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(!targets.is_empty(), "no fuzz targets found");

    let me = std::fs::read_to_string(root().join("tests/fuzz_smoke.rs")).expect("read self");
    let manifest = std::fs::read_to_string(root().join("fuzz/Cargo.toml")).expect("read manifest");
    for t in targets {
        assert!(
            me.contains(&format!("fn {t}_corpus_does_not_panic")),
            "fuzz target `{t}` has no `{t}_corpus_does_not_panic` smoke test"
        );
        assert!(
            manifest.contains(&format!("name = \"{t}\"")),
            "fuzz target `{t}` is missing its [[bin]] in fuzz/Cargo.toml -- cargo-fuzz will not see it"
        );
        assert!(
            root().join("fuzz/corpus").join(&t).is_dir(),
            "fuzz target `{t}` has no seed corpus"
        );
    }
}
