//! The library API, from an embedder's side.
//!
//! `texrs` the binary is one caller; anything linking the crate is another, and
//! what it needs is different. It never sees the terminal, so output has to come
//! back as values; it runs more than one document in one process, so nothing may
//! leak from one to the next; and it cannot afford a panic, so every failure has
//! to be a `Result`.
//!
//! Those three properties are easy to break without noticing, because the
//! binary runs one document and exits: a leak between runs is invisible to it.
//! `MESSAGES`, `BUILDING` and the fault channel are thread-locals, and the
//! macro table and registers live in a `Lowerer` built per call — the tests
//! below are what say so out loud.

use std::collections::HashSet;

#[test]
fn output_comes_back_as_a_value_rather_than_going_to_the_terminal() {
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{ONE}\n\\message{TWO}\n\\end\n";
    assert_eq!(texrs::run_messages(src).expect("runs"), "ONE TWO");
    // And as a list, for a caller that wants the messages apart.
    assert_eq!(
        texrs::run_messages_list(src).expect("runs"),
        vec!["ONE".to_string(), "TWO".to_string()]
    );
}

#[test]
fn a_failure_is_a_value_not_a_panic_or_an_exit() {
    let err = texrs::run_messages("\\nosuchprimitive\n").expect_err("must not run");
    assert!(
        err.0.contains("Undefined control sequence"),
        "unhelpful: {}",
        err.0
    );
    // And the process is still usable afterwards.
    assert_eq!(
        texrs::run_messages("\\catcode`\\{=1 \\catcode`\\}=2\n\\message{AFTER}\n\\end\n")
            .expect("runs"),
        "AFTER"
    );
}

#[test]
fn one_run_leaves_nothing_behind_for_the_next() {
    // Catcodes, macros and registers are all per-run state. A second document
    // that relied on the first's would print something here; it must not.
    let first = "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6\n\
                 \\def\\leaked{LEAKED}\n\\count1=99\n\\message{\\the\\count1}\n\\end\n";
    assert_eq!(texrs::run_messages(first).expect("runs"), "99");

    // The macro is gone. What a second document sees for an undefined name is
    // the name itself today (`tests/known_gaps.txt`: an undefined control
    // sequence prints rather than raising, where tex raises), so this asserts
    // the property that matters -- the first document's body did not expand --
    // rather than the shape of the failure, which will change when that gap
    // closes.
    let second =
        texrs::run_messages("\\catcode`\\{=1 \\catcode`\\}=2\n\\message{\\leaked}\n\\end\n");
    match second {
        Ok(out) => assert!(
            !out.contains("LEAKED"),
            "the macro table survived the run: {out:?}"
        ),
        Err(e) => assert!(e.0.contains("Undefined control sequence"), "{}", e.0),
    }

    // The register is back to INITEX zero, not 99.
    assert_eq!(
        texrs::run_messages("\\catcode`\\{=1 \\catcode`\\}=2\n\\message{\\the\\count1}\n\\end\n")
            .expect("runs"),
        "0"
    );

    // And the catcodes are back to INITEX's, where `{` is an ordinary
    // character -- so a document that forgets the preamble sees it as text.
    let err = texrs::run_messages("\\message XY\\end\n")
        .expect_err("without the preamble there are no group characters")
        .0;
    assert!(
        err.contains("Missing {"),
        "the catcode table survived the run: {err}"
    );
}

#[test]
fn the_message_buffer_does_not_accumulate_across_runs() {
    // The buffer is a thread-local, so a run that failed to clear it would show
    // up as the previous document's output prepended to this one's.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{ONLY}\n\\end\n";
    for _ in 0..3 {
        assert_eq!(texrs::run_messages(src).expect("runs"), "ONLY");
    }
}

#[test]
fn a_failed_run_does_not_poison_the_next_one() {
    // The fault channel is a thread-local too: a fault left in it would turn
    // the NEXT successful run into an error.
    let _ = texrs::run_messages("\\nosuchprimitive\n");
    assert_eq!(
        texrs::run_messages("\\catcode`\\{=1 \\catcode`\\}=2\n\\message{FINE}\n\\end\n")
            .expect("a later run must not inherit the fault"),
        "FINE"
    );
}

#[test]
fn compiling_and_running_are_separable() {
    // An embedder that compiles once and runs many times needs the chunk on its
    // own, and the run to be a function of the chunk alone.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\count1=7\n\\advance\\count1 by 5\n\
               \\message{\\the\\count1}\n\\end\n";
    let chunk = texrs::compile(src).expect("compiles");
    for _ in 0..3 {
        let msgs = texrs::runtime::run(chunk.clone()).expect("runs");
        assert_eq!(msgs, vec!["12".to_string()]);
    }
}

#[test]
fn the_engine_is_usable_from_a_thread_that_did_not_build_it() {
    // Nothing in the pipeline is bound to the main thread; an embedder runs
    // documents on a pool.
    let handles: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                let src = format!(
                    "\\catcode`\\{{=1 \\catcode`\\}}=2\n\\count1={i}\n\\message{{\\the\\count1}}\n\\end\n"
                );
                texrs::run_messages(&src).expect("runs")
            })
        })
        .collect();
    let got: HashSet<String> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    assert_eq!(
        got,
        (0..4).map(|i| i.to_string()).collect::<HashSet<_>>(),
        "each thread must see its own document, not another's"
    );
}

#[test]
fn a_diagnostic_carries_the_line_it_broke_on() {
    // The editor path: an embedder that is a language server needs a position,
    // and `compile` alone does not give one.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\message{FINE}\n\\nope\n\\end\n";
    let (err, line) = texrs::compile_located(src).expect_err("must not compile");
    assert!(err.0.contains("Undefined control sequence"), "{}", err.0);
    assert_eq!(line, 3, "the error is on line 3");
}
