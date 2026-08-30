//! `texrs --aot` produces a native binary that is the same program.
//!
//! The claim being tested is observational: a document compiled ahead of time
//! must print exactly what the interpreted run prints, byte for byte, including
//! the `(./file.tex … )` line. Anything less and "compiled" would mean "behaves
//! differently", which is not a compiler.
//!
//! Skipped, loudly, when there is no C toolchain to link with — the object is
//! fusevm's, but the executable is `cc`'s.

use std::path::Path;
use std::process::Command;

fn have_cc() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// AOT-compile `src` in a scratch directory and return what the binary printed.
fn aot_output(dir: &Path, name: &str, src: &str) -> String {
    let doc = dir.join(format!("{name}.tex"));
    std::fs::write(&doc, src).expect("write document");

    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg("--aot")
        .arg(&doc)
        .output()
        .expect("run texrs --aot");
    assert!(
        out.status.success(),
        "--aot failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let binary = dir.join(name);
    assert!(binary.is_file(), "--aot reported success but wrote nothing");
    let run = Command::new(&binary)
        .output()
        .expect("run the compiled binary");
    assert!(
        run.status.success(),
        "the compiled binary failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// What the interpreter prints for the same document at the same path.
fn interpreted_output(dir: &Path, name: &str) -> String {
    let doc = dir.join(format!("{name}.tex"));
    let out = Command::new(env!("CARGO_BIN_EXE_texrs"))
        .arg(&doc)
        .output()
        .expect("run texrs");
    assert!(
        out.status.success(),
        "the interpreted run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_compiled_document_prints_what_the_interpreted_one_prints() {
    if !have_cc() {
        eprintln!("skipping: no `cc` to link with");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");

    // Register arithmetic, a macro with a parameter, a conditional and a group:
    // everything that survives lowering, in one document.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
               \\def\\greet#1{HELLO-#1}\n\
               \\count1=7\n\
               \\advance\\count1 by 5\n\
               \\multiply\\count1 by 3\n\
               \\message{\\greet{WORLD}}\n\
               \\message{count=\\the\\count1}\n\
               \\message{\\ifnum\\count1>10 BIG\\else SMALL\\fi}\n\
               {\\count1=99 \\message{inner=\\the\\count1}}\n\
               \\message{outer=\\the\\count1}\n\
               \\end\n";

    let compiled = aot_output(dir.path(), "doc", src);
    let interpreted = interpreted_output(dir.path(), "doc");
    assert_eq!(
        compiled, interpreted,
        "the compiled document is a different program from the interpreted one"
    );
    assert!(
        compiled.contains("count=36") && compiled.contains("BIG") && compiled.contains("outer=36"),
        "the compiled document printed: {compiled:?}"
    );
}

#[test]
fn the_compiler_does_not_leave_its_intermediates_behind() {
    if !have_cc() {
        eprintln!("skipping: no `cc` to link with");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let out = aot_output(
        dir.path(),
        "standalone",
        "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{ALONE}\n\\end\n",
    );
    assert!(out.contains("ALONE"), "printed: {out:?}");

    for leftover in ["standalone.o", "standalone.aot_main.c"] {
        assert!(
            !dir.path().join(leftover).exists(),
            "{leftover} was left behind"
        );
    }
}
