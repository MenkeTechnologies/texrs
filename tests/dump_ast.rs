//! `--dump-ast`: the command stream between the mouth's tokens and the
//! bytecode.
//!
//! The listing is only worth having if it shows what the OTHER two cannot.
//! `--dump-tokens` runs before expansion, so a macro is still a control
//! sequence there; `--disasm` runs after code generation, so a conditional is
//! already a jump. These pin the two facts that live only in between: a macro
//! appears as what it expanded to, and a conditional still has two named
//! branches.

use std::path::{Path, PathBuf};
use std::process::Command;

fn texrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_texrs"))
}

fn write(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texrs-ast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

fn dump(path: &Path) -> String {
    let out = texrs().arg("--dump-ast").arg(path).output().expect("run");
    assert!(
        out.status.success(),
        "--dump-ast failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const HEAD: &str = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n";

#[test]
fn a_conditional_still_has_two_branches_here_and_a_jump_only_after_codegen() {
    let path = write(
        "cond.tex",
        &format!("{HEAD}\\count0=7\n\\ifnum\\count0>5 \\message{{big}}\\else\\message{{small}}\\fi\n\\end\n"),
    );
    let ast = dump(&path);
    assert!(ast.contains("IfNum \\count0 > 5"), "no conditional: {ast}");
    assert!(ast.contains("then"), "no then branch: {ast}");
    assert!(ast.contains("else"), "no else branch: {ast}");
    // Both arms survive to here: lowering a conditional does not pick one.
    assert!(
        ast.contains("\"big\"") && ast.contains("\"small\""),
        "{ast}"
    );

    // The same document after code generation has no such structure -- which is
    // what makes this listing worth printing separately.
    let out = texrs().arg("--disasm").arg(&path).output().expect("run");
    let disasm = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !disasm.contains("IfNum"),
        "disasm still shows the IR: {disasm}"
    );
}

#[test]
fn a_macro_appears_as_what_it_expanded_to() {
    let path = write(
        "macro.tex",
        &format!("{HEAD}\\def\\bump{{\\advance\\count0 by 2 }}\n\\bump\\bump\n\\message{{\\the\\count0}}\n\\end\n"),
    );
    let ast = dump(&path);
    // Expansion has happened: the definition is gone and the two calls are the
    // two register writes they expanded to.
    assert!(!ast.contains("bump"), "the macro name survived: {ast}");
    assert_eq!(
        ast.matches("Add \\count0 by 2").count(),
        2,
        "expected both calls expanded: {ast}"
    );
}

#[test]
fn a_tail_recursive_macro_shows_as_the_loop_it_lowered_to() {
    let path = write(
        "loop.tex",
        &format!(
            "{HEAD}\\def\\r{{\\advance\\count1 by 1 \\ifnum\\count1<4 \\r \\fi}}\n\\r\n\\end\n"
        ),
    );
    let ast = dump(&path);
    assert!(ast.contains("Loop"), "not lowered to a loop: {ast}");
    assert!(ast.contains("while \\count1 < 4"), "no loop test: {ast}");
    // One body, not four inlined copies -- that is the whole point of the
    // shape, and a listing that hid it would be lying about the bytecode.
    assert_eq!(ast.matches("Add \\count1 by 1").count(), 1, "{ast}");
}

#[test]
fn a_group_names_the_registers_it_saves() {
    let path = write(
        "group.tex",
        &format!("{HEAD}\\count0=1\n{{\\count0=9 \\message{{in}}}}\n\\end\n"),
    );
    let ast = dump(&path);
    assert!(ast.contains("Group saves=[0]"), "{ast}");
}

#[test]
fn a_document_that_does_not_lower_reports_the_error_rather_than_printing_a_tree() {
    let path = write(
        "bad.tex",
        &format!("{HEAD}\\count0=\\message{{x}}\n\\end\n"),
    );
    let out = texrs().arg("--dump-ast").arg(&path).output().expect("run");
    assert!(!out.status.success(), "a broken document exited zero");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(said.starts_with("! "), "not tex's error form: {said}");
    assert!(out.stdout.is_empty(), "printed a tree anyway");
}
