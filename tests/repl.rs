//! The interactive session: what a line of TeX means when the ones before it
//! are still in effect.
//!
//! These drive `repl::Session` directly. The reedline loop around it needs a
//! terminal, and what is worth pinning is not that a line editor edits lines —
//! it is that state carries the way TeX's own state carries: a `\catcode` set on
//! one prompt changes how the NEXT line reads, a `\def` changes what it means,
//! and a register assignment survives because the session re-runs the document
//! it has built rather than evaluating each line in a vacuum.

use texrs::repl::{Session, Turn};

fn out(t: Turn) -> Vec<String> {
    match t {
        Turn::Output(msgs) => msgs,
        Turn::Error(e) => panic!("expected output, got error: {e}"),
    }
}

#[test]
fn a_catcode_set_on_one_line_changes_how_the_next_reads() {
    let mut s = Session::new();
    // Without the catcode line first, `{` is an ordinary character and the
    // message below would not even be a group.
    assert!(out(s.eval("\\catcode`\\{=1 \\catcode`\\}=2")).is_empty());
    assert_eq!(out(s.eval("\\message{HELLO}")), vec!["HELLO".to_string()]);
}

#[test]
fn a_macro_defined_on_one_line_is_callable_on_the_next() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6");
    s.eval("\\def\\greet#1{HELLO-#1}");
    assert_eq!(
        out(s.eval("\\message{\\greet{WORLD}}")),
        vec!["HELLO-WORLD".to_string()]
    );
}

#[test]
fn register_state_carries_across_prompts() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2");
    s.eval("\\count1=7");
    s.eval("\\advance\\count1 by 5");
    assert_eq!(
        out(s.eval("\\message{\\the\\count1}")),
        vec!["12".to_string()]
    );
}

#[test]
fn each_turn_reports_only_what_its_own_line_printed() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2");
    assert_eq!(out(s.eval("\\message{ONE}")), vec!["ONE".to_string()]);
    // The session re-runs the whole document, so ONE is printed again inside
    // the program -- but the turn must report only TWO.
    assert_eq!(out(s.eval("\\message{TWO}")), vec!["TWO".to_string()]);
}

#[test]
fn a_line_that_fails_does_not_join_the_document() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2");
    let before = s.source();
    match s.eval("\\nosuchprimitive") {
        Turn::Error(e) => assert!(e.contains("Undefined control sequence"), "{e}"),
        Turn::Output(o) => panic!("expected an error, got {o:?}"),
    }
    assert_eq!(
        s.source(),
        before,
        "a failed line stayed in the session, which would fail every turn after it"
    );
    // And the session still works.
    assert_eq!(out(s.eval("\\message{STILL}")), vec!["STILL".to_string()]);
}

#[test]
fn an_end_on_one_line_does_not_end_the_session() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2");
    s.eval("\\message{FIRST}");
    s.eval("\\end");
    assert_eq!(
        out(s.eval("\\message{AFTER}")),
        vec!["AFTER".to_string()],
        "\\end stopped the session instead of just the line"
    );
}

#[test]
fn a_group_opened_and_closed_on_one_line_scopes_as_it_would_in_a_file() {
    let mut s = Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6");
    s.eval("\\def\\v{OUT}");
    assert_eq!(
        out(s.eval("{\\def\\v{IN}\\message{\\v}}")),
        vec!["IN".to_string()]
    );
    assert_eq!(out(s.eval("\\message{\\v}")), vec!["OUT".to_string()]);
}
