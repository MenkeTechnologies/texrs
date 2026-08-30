//! A macro that names itself: TeX's loop.
//!
//! `\def\r{BODY \ifnum A<B \r \fi}` is how TeX writes a loop -- run the body,
//! test, invoke yourself again. Lowering used to INLINE a macro into the stream
//! and lower through its body, and it lowers both arms of a run-time conditional
//! because neither is decided yet. Those together cannot terminate for this
//! shape: the copy contains the call that gets copied. It exhausted the Rust
//! stack and aborted the process -- no diagnostic, no exit code.
//!
//! It now lowers to a real loop: the body once, then a backward jump. That is
//! finite bytecode, it costs no call frame, and its stack use is constant, which
//! is why it runs iteration counts real `tex` refuses (tex pushes an input-stack
//! level per call and gives up at 10000).
//!
//! Anything NOT of that exact shape still inlines, and the depth bound in
//! `lower.rs` keeps that from aborting.

use fusevm::Op;

fn loop_src(n: i64, rel: &str, start: i64) -> String {
    format!(
        concat!(
            "\\catcode`\\{{=1 \\catcode`\\}}=2\n",
            "\\count0={start}\n",
            "\\def\\r{{\\advance\\count0 by 1 \\ifnum\\count0{rel}{n} \\r \\fi}}\n",
            "\\r \\message{{\\the\\count0}}\n",
            "\\end\n"
        ),
        start = start,
        rel = rel,
        n = n
    )
}

fn message(src: &str) -> String {
    texrs::run_messages(src).expect("run")
}

#[test]
fn a_tail_recursive_macro_runs_instead_of_aborting() {
    // The shape that used to take the process down. tex prints 3 for this file.
    assert_eq!(message(&loop_src(3, "<", 0)), "3");
}

#[test]
fn the_loop_counts_the_same_as_tex_does() {
    // Spot values across the range tex itself can still reach, so the numbers
    // are checked against a real engine's semantics rather than against a
    // rewrite of the same arithmetic.
    assert_eq!(message(&loop_src(1, "<", 0)), "1");
    assert_eq!(message(&loop_src(100, "<", 0)), "100");
    assert_eq!(message(&loop_src(9000, "<", 0)), "9000");
}

#[test]
fn a_loop_whose_test_fails_first_time_runs_its_body_once() {
    // `\r` runs its body BEFORE reaching its own conditional, so this is a
    // do-while, not a while. Getting it backwards would print 0.
    assert_eq!(message(&loop_src(3, ">", 0)), "1");
}

#[test]
fn the_loop_is_a_backward_jump_and_not_an_inlined_copy() {
    // The point of the change: finite bytecode with a back edge. An inlined
    // copy would grow the op count with the iteration bound; a loop does not.
    let ops = |n: i64| {
        texrs::compile(&loop_src(n, "<", 0))
            .expect("compile")
            .ops
            .len()
    };
    let (small, large) = (ops(3), ops(100_000));
    assert_eq!(
        small, large,
        "op count must not depend on the iteration bound: {small} vs {large}"
    );
    let chunk = texrs::compile(&loop_src(3, "<", 0)).expect("compile");
    assert!(
        chunk.ops.iter().any(|o| matches!(o, Op::JumpIfTrue(_))),
        "a loop must close with a conditional back edge"
    );
}

#[test]
fn it_runs_iteration_counts_real_tex_cannot() {
    // tex pushes an input-stack level per recursive call and stops at
    // `[input stack size=10000]`; a loop's stack use is constant. Verified
    // against TeX Live 2026: 12000 fails there, this does not.
    assert_eq!(message(&loop_src(12_000, "<", 0)), "12000");
    assert_eq!(message(&loop_src(1_000_000, "<", 0)), "1000000");
}

#[test]
fn mutual_recursion_is_still_bounded_rather_than_aborting() {
    // Two macros naming each other are not the loop idiom -- the recogniser
    // requires a SELF call -- so this still inlines, and the depth bound is what
    // keeps it from being a crash.
    let src = concat!(
        "\\catcode`\\{=1 \\catcode`\\}=2\n",
        "\\count0=99\n",
        "\\def\\a{\\ifnum\\count0<3 \\b \\fi}\n",
        "\\def\\b{\\ifnum\\count0<3 \\a \\fi}\n",
        "\\a \\message{ok}\n",
        "\\end\n"
    );
    let err = texrs::compile(src).expect_err("must not compile");
    assert!(err.0.contains("capacity exceeded"), "got {:?}", err.0);
}

#[test]
fn an_ordinary_macro_still_inlines() {
    // The recogniser must not swallow a plain macro used inside a conditional.
    let src = concat!(
        "\\catcode`\\{=1 \\catcode`\\}=2\n",
        "\\count0=1\n",
        "\\def\\q{Q}\n",
        "\\def\\r{\\ifnum\\count0<3 \\q \\fi}\n",
        "\\r \\message{done}\n",
        "\\end\n"
    );
    texrs::compile(src).expect("an ordinary macro in an arm must still lower");
}

#[test]
fn deeply_nested_conditionals_stay_under_the_bound() {
    // A real document nests nowhere near 256 deep; 30 levels must lower.
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=1\n");
    for _ in 0..30 {
        src.push_str("\\ifnum\\count0<9 ");
    }
    src.push_str("\\message{deep}");
    for _ in 0..30 {
        src.push_str("\\fi");
    }
    src.push_str("\n\\end\n");
    texrs::compile(&src).expect("30 levels of nesting must lower");
}
