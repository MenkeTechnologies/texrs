//! Advice on macro expansion: `\intercept` with before, after and around.
//!
//! What is worth pinning is not that a handler runs — it is where the weave
//! sits. Expansion is a compile-time act in texrs, so advice is woven into the
//! token stream: it must survive being read inside a `\message` body, it must
//! be undone by the group that registered it, and a handler that calls the
//! macro it advises must not weave itself forever.

fn run(body: &str) -> Result<String, String> {
    let src = format!("\\catcode`\\{{=1 \\catcode`\\}}=2 \\catcode`\\#=6\n{body}\\end\n");
    texrs::run_messages(&src).map_err(|e| e.0)
}

#[test]
fn before_advice_runs_in_front_of_the_expansion() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\trace{[in]}\n\
         \\intercept{before}{greet}{\\trace}\n\
         \\message{\\greet{WORLD}}\n")
    .expect("runs");
    assert_eq!(out, "[in]HELLO-WORLD");
}

#[test]
fn after_advice_runs_behind_it() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\note{[out]}\n\
         \\intercept{after}{greet}{\\note}\n\
         \\message{\\greet{WORLD}}\n")
    .expect("runs");
    assert_eq!(out, "HELLO-WORLD[out]");
}

#[test]
fn around_advice_wraps_the_expansion_at_proceed() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\loud{<<\\proceed>>}\n\
         \\intercept{around}{greet}{\\loud}\n\
         \\message{\\greet{WORLD}}\n")
    .expect("runs");
    assert_eq!(out, "<<HELLO-WORLD>>");
}

#[test]
fn around_advice_without_proceed_suppresses_the_call() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\silent{NOTHING}\n\
         \\intercept{around}{greet}{\\silent}\n\
         \\message{\\greet{WORLD}}\n")
    .expect("runs");
    assert_eq!(out, "NOTHING");
}

#[test]
fn a_glob_catches_macros_defined_after_the_registration() {
    // The point of matching by pattern: the advice is registered before the
    // macro it will catch even exists.
    let out = run("\\def\\note{[x]}\n\
         \\intercept{after}{sec*}{\\note}\n\
         \\def\\section#1{S:#1}\n\
         \\def\\secondary{S2}\n\
         \\def\\other{O}\n\
         \\message{\\section{A}}\n\
         \\message{\\secondary}\n\
         \\message{\\other}\n")
    .expect("runs");
    assert_eq!(out, "S:A[x] S2[x] O");
}

#[test]
fn advice_is_undone_by_the_group_that_registered_it() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\silent{NOTHING}\n\
         {\\intercept{around}{greet}{\\silent}\\message{\\greet{X}}}\n\
         \\message{\\greet{Y}}\n")
    .expect("runs");
    assert_eq!(out, "NOTHING HELLO-Y");
}

#[test]
fn a_handler_that_calls_the_macro_it_advises_does_not_recurse() {
    // Without the depth markers this expands forever: the handler's own call
    // to \greet would be advised again, and so on.
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\twice{\\greet{TWICE}}\n\
         \\intercept{before}{greet}{\\twice}\n\
         \\message{\\greet{WORLD}}\n")
    .expect("runs");
    assert_eq!(out, "HELLO-TWICEHELLO-WORLD");
}

#[test]
fn several_advices_apply_in_registration_order() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\a{[a]}\n\
         \\def\\b{[b]}\n\
         \\intercept{before}{greet}{\\a}\n\
         \\intercept{before}{greet}{\\b}\n\
         \\message{\\greet{W}}\n")
    .expect("runs");
    // The second registration wraps the first, so it lands outermost.
    assert_eq!(out, "[b][a]HELLO-W");
}

#[test]
fn an_unknown_kind_is_refused_with_the_three_that_exist() {
    let err = run(
        "\\def\\greet#1{X}\n\\def\\h{Y}\n\\intercept{sideways}{greet}{\\h}\n\\message{\\greet{A}}\n",
    )
    .expect_err("an unknown advice kind must not be ignored");
    assert!(
        err.contains("before, after or around"),
        "unhelpful error: {err}"
    );
}

#[test]
fn a_handler_taking_parameters_is_refused() {
    // Calling it would eat whatever followed the intercepted call.
    let err = run("\\def\\greet#1{HELLO-#1}\n\
         \\def\\h#1{[#1]}\n\
         \\intercept{before}{greet}{\\h}\n\
         \\message{\\greet{W}}\n")
    .expect_err("a parameterised handler must be refused");
    assert!(
        err.contains("advice handlers take none"),
        "error was: {err}"
    );
}

#[test]
fn a_handler_that_is_not_a_macro_is_refused() {
    let err = run("\\def\\greet#1{HELLO-#1}\n\
         \\intercept{before}{greet}{\\nosuch}\n\
         \\message{\\greet{W}}\n")
    .expect_err("an undefined handler must be refused");
    assert!(err.contains("is not a macro"), "error was: {err}");
}

#[test]
fn a_bad_pattern_says_so_rather_than_matching_nothing() {
    let err = run(
        "\\def\\greet#1{X}\n\\def\\h{Y}\n\\intercept{before}{gr[e}{\\h}\n\\message{\\greet{A}}\n",
    )
    .expect_err("a malformed glob must be refused");
    assert!(err.contains("Bad intercept pattern"), "error was: {err}");
}

#[test]
fn a_document_with_no_advice_is_unchanged() {
    let out = run("\\def\\greet#1{HELLO-#1}\n\\message{\\greet{WORLD}}\n").expect("runs");
    assert_eq!(out, "HELLO-WORLD");
}
