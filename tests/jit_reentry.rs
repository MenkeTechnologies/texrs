//! Running several documents in one thread must not crash.
//!
//! The REPL re-runs its whole accumulated source at every prompt, so a session
//! is a sequence of GROWING documents in one thread. Under fusevm 0.26.0 with
//! the tracing JIT switched on for every run, the third of those faulted in
//! JIT-compiled code -- a null-pointer load, `EXC_BAD_ACCESS` at address 0,
//! with no Rust frame on the stack. `src/runtime.rs` now switches the JIT on
//! only for a chunk that has a loop in it, which is the shape a tracing JIT is
//! for; this holds that line, because the crash is a SEGV rather than a failed
//! assertion and would otherwise return as a mysteriously dead test binary.
#[test]
fn a_growing_document_run_after_every_line_does_not_fault() {
    let lines = [
        "\\catcode`\\{=1 \\catcode`\\}=2 \\catcode`\\#=6",
        "\\def\\v{OUT}",
        "{\\def\\v{IN}\\message{\\v}}",
        "\\message{[\\v]}",
    ];
    for n in 1..=lines.len() {
        let src = lines[..n].join("\n");
        texrs::run_messages_list(&src).expect("every prefix of a session runs");
    }
}

#[test]
fn a_session_survives_more_prompts_than_the_jit_needs_to_warm() {
    // Ten turns is well past any hotness threshold, and each one recompiles and
    // re-runs everything before it.
    let mut s = texrs::repl::Session::new();
    s.eval("\\catcode`\\{=1 \\catcode`\\}=2");
    for i in 0..10 {
        s.eval(&format!("\\count1={i} \\message{{[\\the\\count1]}}"));
    }
}
