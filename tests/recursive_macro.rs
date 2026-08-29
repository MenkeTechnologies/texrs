//! A macro that names itself must not take the process down with it.
//!
//! Lowering inlines a macro into the stream and lowers through its body, and it
//! lowers BOTH arms of a run-time conditional because neither is decided yet.
//! Those two together mean `\def\r{\ifnum\count0<3 \r \fi}` inlines into its own
//! arm forever: the recursion is in the LOWERER, not in the program, so it does
//! not matter which way the test would go at run time or how few iterations the
//! author intended. Before the depth bound this exhausted the Rust stack and
//! aborted the process -- no diagnostic, no exit code, nothing a caller could
//! act on. `tex` runs the same file and prints `3`.
//!
//! These pin the bound itself. The gap they do NOT close is that the file still
//! fails: making it print `3` needs a recursive macro to lower to a run-time
//! call instead of an inline copy rather than a bound on the copying.

/// The shape that used to abort: self-reference guarded by a conditional.
const SELF_RECURSIVE: &str = concat!(
    "\\catcode`\\{=1 \\catcode`\\}=2\n",
    "\\count0=99\n",
    "\\def\\r{\\ifnum\\count0<3 \\r \\fi}\n",
    "\\r \\message{ok}\n",
    "\\end\n"
);

#[test]
fn a_self_recursive_macro_reports_capacity_rather_than_aborting() {
    let err = texrs::compile(SELF_RECURSIVE).expect_err("must not compile");
    assert!(
        err.0.contains("capacity exceeded"),
        "want TeX's own capacity wording, got {:?}",
        err.0
    );
}

#[test]
fn the_bound_does_not_depend_on_which_arm_recurses() {
    // `\count0=1` makes `>3` false, so the recursive call sits in the arm that
    // would NOT be taken. It still has to be bounded: the lowerer emits both.
    let src = SELF_RECURSIVE
        .replace("\\count0=99", "\\count0=1")
        .replace("<3", ">3");
    let err = texrs::compile(&src).expect_err("must not compile");
    assert!(err.0.contains("capacity exceeded"), "got {:?}", err.0);
}

#[test]
fn mutual_recursion_is_bounded_too() {
    // Two macros that name each other recurse just as unboundedly, and a guard
    // that only watched for a macro naming ITSELF would miss this.
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
fn a_non_recursive_macro_in_a_skipped_arm_still_compiles() {
    // The bound must not catch ordinary nesting. This is the same shape with
    // the self-reference removed, and it has to keep working.
    let src = concat!(
        "\\catcode`\\{=1 \\catcode`\\}=2\n",
        "\\count0=99\n",
        "\\def\\q{\\message{Q}}\n",
        "\\def\\r{\\ifnum\\count0<3 \\q \\fi}\n",
        "\\r \\message{ok}\n",
        "\\end\n"
    );
    texrs::compile(src).expect("an ordinary macro in an arm must still lower");
}

#[test]
fn deeply_nested_conditionals_stay_under_the_bound() {
    // A real document nests conditionals nowhere near 256 deep. Thirty levels
    // must lower cleanly, or the bound is set too low to be safe.
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
