//! Inline Rust in a TeX document: `\rust{ … }` and `\rustcall`.
//!
//! The claim is narrow and worth pinning exactly: a function compiled from a
//! block is a NUMBER source wherever TeX reads a number. So the tests check it
//! in each of those places — a register assignment, an arithmetic operand, a
//! conditional, and a message body — rather than only in the one that was
//! easiest to write.
//!
//! Skipped, loudly, when there is no `rustc`: fusevm shells out to it to build
//! the block, and a test that quietly passed without one would be testing that
//! nothing happened.

use std::process::Command;
use std::sync::Mutex;

/// Serializes the tests that compile a block.
///
/// fusevm keys its FFI cache by body hash under one shared directory, and two
/// rustc invocations landing in it at once trample each other's intermediate
/// object files -- which is what CI saw first, as `rust-lld: cannot open
/// ...rcgu.o`. Compiling a block is slow enough that serializing costs little,
/// and the tests are about the engine rather than about concurrency.
static COMPILING: Mutex<()> = Mutex::new(());

fn have_rustc() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    Command::new(rustc)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A block exporting the two functions the tests call.
const BLOCK: &str = "\\rust{\n\
    #[no_mangle]\n\
    pub extern \"C\" fn texrs_test_twice(n: i64) -> i64 { n * 2 }\n\
    #[no_mangle]\n\
    pub extern \"C\" fn texrs_test_add(a: i64, b: i64) -> i64 { a + b }\n\
}\n";

fn run(body: &str) -> Result<String, String> {
    let src = format!("{BLOCK}\\catcode`\\{{=1 \\catcode`\\}}=2\n{body}\\end\n");
    let _guard = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    texrs::run_messages(&src).map_err(|e| e.0)
}

#[test]
fn a_block_function_is_callable_from_a_message() {
    if !have_rustc() {
        eprintln!("skipping: no rustc to build the block with");
        return;
    }
    let out = run("\\count1=21\n\\message{twice=\\rustcall texrs_test_twice \\count1 \\endrust}\n")
        .expect("runs");
    assert_eq!(out, "twice=42");
}

#[test]
fn a_call_takes_several_arguments_and_mixes_literals_with_registers() {
    if !have_rustc() {
        eprintln!("skipping: no rustc");
        return;
    }
    let out = run("\\count1=20\n\\message{sum=\\rustcall texrs_test_add \\count1 22 \\endrust}\n")
        .expect("runs");
    assert_eq!(out, "sum=42");
}

#[test]
fn a_call_is_a_number_everywhere_tex_reads_one() {
    if !have_rustc() {
        eprintln!("skipping: no rustc");
        return;
    }
    let out = run(
        "\\count1=20\n\
         \\count2=\\rustcall texrs_test_add \\count1 22 \\endrust\n\
         \\advance\\count2 by \\rustcall texrs_test_twice 4 \\endrust\n\
         \\message{after=\\the\\count2}\n\
         \\message{\\ifnum\\rustcall texrs_test_twice \\count1 \\endrust>39 BIG\\else SMALL\\fi}\n",
    )
    .expect("runs");
    // 20 + 22 = 42, + (4*2) = 50; and 20*2 = 40 > 39.
    assert_eq!(out, "after=50 BIG");
}

#[test]
fn calling_a_function_no_block_exported_says_so() {
    if !have_rustc() {
        eprintln!("skipping: no rustc");
        return;
    }
    let err = run("\\message{\\rustcall texrs_test_missing 1 \\endrust}\n")
        .expect_err("a missing function must not be a silent zero");
    assert!(
        err.contains("Undefined rust function texrs_test_missing"),
        "unhelpful error: {err}"
    );
}

#[test]
fn a_block_that_does_not_compile_reports_what_rustc_said() {
    if !have_rustc() {
        eprintln!("skipping: no rustc");
        return;
    }
    let src = "\\rust{\n    pub extern \"C\" fn broken(  -> i64 { }\n}\n\
               \\catcode`\\{=1 \\catcode`\\}=2\n\\message{X}\n\\end\n";
    let Err(err) = texrs::run_messages(src) else {
        panic!("a broken block must stop the run");
    };
    let err = err.0;
    assert!(
        err.contains("rustc failed"),
        "the rustc diagnostic did not reach the caller: {err}"
    );
}

/// The desugar itself needs no toolchain, so this one always runs.
#[test]
fn the_desugar_preserves_line_numbers_and_leaves_other_sources_alone() {
    let src = "\\rust{\n    // three\n    // lines\n}\n\\message{AFTER}\n";
    let out = texrs::rust_ffi::desugar(src);
    assert_eq!(
        src.lines().count(),
        out.lines().count(),
        "the desugar moved the lines after the block:\n{out}"
    );
    assert!(
        out.contains("\\rustcompile"),
        "no replacement emitted: {out}"
    );

    let plain = "\\catcode`\\{=1\n\\message{HELLO}\n";
    assert_eq!(
        texrs::rust_ffi::desugar(plain),
        plain,
        "a document with no block was rewritten"
    );
}

/// `\rustle` is not `\rust`: a document may define a macro whose name merely
/// starts with the keyword.
#[test]
fn a_macro_whose_name_starts_with_rust_is_not_a_block() {
    let src = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
               \\def\\rustle{LEAVES}\n\\message{\\rustle}\n\\end\n";
    assert_eq!(texrs::rust_ffi::desugar(src), src);
    assert_eq!(texrs::run_messages(src).expect("runs"), "LEAVES");
}
