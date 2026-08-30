//! texrs is a fusevm FRONTEND, and this is the test that says so.
//!
//! Byte-for-byte output parity does not prove where the work happens: a
//! tree-walking interpreter passes the same corpus. What distinguishes a
//! frontend is that the program becomes BYTECODE — count registers are VM slots,
//! arithmetic is native ops, and a conditional is a real branch the JIT can see.
//! So this asserts the emitted chunk, not the printed answer.

use fusevm::Op;

fn ops(src: &str) -> Vec<Op> {
    let chunk = texrs::compile(src).expect("compile");
    // The prologue zeroes all 256 count slots; skip it to see the program.
    chunk.ops.iter().skip(512).cloned().collect()
}

#[test]
fn count_arithmetic_lowers_to_native_ops() {
    let got = ops("\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=7 \\advance\\count0 by 5\n\\end\n");
    // `\count0=7` is LoadInt/SetSlot; `\advance` is GetSlot/LoadInt/Add/SetSlot.
    assert!(
        got.contains(&Op::Add),
        "\\advance must be a native Add, got {got:?}"
    );
    assert!(
        got.iter().any(|o| matches!(o, Op::GetSlot(0))),
        "a count register must be a VM slot, got {got:?}"
    );
    assert!(
        got.iter().any(|o| matches!(o, Op::SetSlot(0))),
        "a count assignment must write a slot, got {got:?}"
    );
}

#[test]
fn divide_truncates_to_an_integer() {
    // The property, not the mechanism. `\divide` used to lower to fusevm's
    // numeric `Div` plus a `TruncInt`; it now goes through the checked builtin,
    // because TeX raises `Arithmetic overflow` on a division by zero rather
    // than producing an infinity (tex.web §1236). What must stay true either
    // way is that the answer is TeX's integer one -- and that no bare numeric
    // `Div`, whose result is a float, reaches the register.
    let got = ops("\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=7 \\divide\\count0 by 2\n\\end\n");
    assert!(
        !got.contains(&Op::Div),
        "a float division reaches the register; got {got:?}"
    );
    assert_eq!(
        texrs::run_messages(
            "\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=7 \\divide\\count0 by 2\n\
             \\message{\\the\\count0 }\n\\end\n"
        )
        .expect("runs"),
        "3",
        "7/2 truncates to 3, as tex prints it"
    );
    assert_eq!(
        texrs::run_messages(
            "\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=-7 \\divide\\count0 by 2\n\
             \\message{\\the\\count0 }\n\\end\n"
        )
        .expect("runs"),
        "-3",
        "truncation is toward zero, not floor"
    );
}

#[test]
fn a_conditional_lowers_to_a_real_branch() {
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=5\n\\ifnum\\count0>3 \\count1=1 \\else \\count1=2 \\fi\n\\end\n";
    let got = ops(src);
    assert!(
        got.contains(&Op::NumGt),
        "the comparison must be a VM op, got {got:?}"
    );
    assert!(
        got.iter().any(|o| matches!(o, Op::JumpIfFalse(_))),
        "the branch must be a real jump, not a host-side if: {got:?}"
    );
    assert!(
        got.iter().any(|o| matches!(o, Op::Jump(_))),
        "the else arm must be jumped over: {got:?}"
    );
}

#[test]
fn a_meaning_conditional_is_folded_at_compile_time() {
    // `\iftrue` depends on nothing the VM holds, so emitting a branch for it
    // would be bytecode with nothing to test. It must fold.
    let got =
        ops("\\catcode`\\{=1 \\catcode`\\}=2\n\\iftrue \\count0=1 \\else \\count0=2 \\fi\n\\end\n");
    assert!(
        !got.iter().any(|o| matches!(o, Op::JumpIfFalse(_))),
        "a compile-time-known conditional must not emit a branch: {got:?}"
    );
}

/// Every op carries the source line it came from.
///
/// Before this, every op in the chunk reported line 0: a disassembly could not
/// be read against the document, and a source-line debugger had nothing to map.
/// The line is taken at the token that STARTS a command, so a construct that
/// ends at the newline is still attributed to the line it began on.
#[test]
fn ops_carry_the_line_they_came_from() {
    // Lines: 1 catcodes, 2 assignment, 3 arithmetic, 4 message.
    let src = "\\catcode`\\{=1 \\catcode`\\}=2\n\\count1=7\n\\advance\\count1 by 5\n\\message{\\the\\count1}\n\\end\n";
    let chunk = texrs::compile(src).expect("compiles");

    // The register prologue is emitted before any line directive, so it carries
    // line 0; everything after it must carry a real line.
    //
    // Its length is MEASURED rather than written here. It was a hardcoded 512,
    // which was two ops for each of 256 count registers -- and when dimension
    // registers were added the bank became 512 and the skip landed inside the
    // prologue, failing on ops that were never the document's. Taking the
    // leading run of line-0 ops keeps the check itself exactly as strong: a
    // zero anywhere AFTER the prologue is still a failure.
    let prologue = chunk.lines.iter().take_while(|l| **l == 0).count();
    let body: Vec<u32> = chunk.lines.iter().copied().skip(prologue).collect();
    assert!(!body.is_empty(), "no document ops after the prologue");
    assert!(
        body.iter().all(|l| *l >= 2),
        "an op after the prologue still reports a line before the document's \
         first command: {body:?}"
    );
    assert!(
        body.windows(2).all(|w| w[0] <= w[1]),
        "line stamps are not monotonic through a straight-line document: {body:?}"
    );
    assert_eq!(
        *body.last().unwrap(),
        4,
        "the last op should belong to the \\message on line 4"
    );
}

/// The constant pool is INTERNED, and a book is why.
///
/// A `LoadConst` operand is a u16, so the pool holds 65,536 entries. Coalescing
/// text runs across the line directives between them took the books under that;
/// a 4 MB reference still went past it and the compile panicked inside fusevm.
/// Text repeats -- the same words, the same spacing -- so identical strings are
/// one constant, and the pool grows with what a document SAYS rather than with
/// how often it says it.
#[test]
fn identical_text_is_one_constant() {
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2\n");
    // Separated by a command, so each is its own run rather than one long one.
    for _ in 0..1_000 {
        src.push_str("alpha\\message{.}\nbeta\\message{.}\n");
    }
    src.push_str("\\end\n");
    let chunk = texrs::compile_text(&src).expect("compiles");
    let strings = chunk
        .constants
        .iter()
        .filter(|v| matches!(v, fusevm::Value::Str(_)))
        .count();
    assert!(
        strings < 20,
        "2,000 emissions of a handful of strings became {strings} constants; \
         the pool is not interned and a book will exhaust it"
    );
}

/// A document past the pool is REFUSED, not a panic out of the VM.
#[test]
fn a_document_past_the_pool_is_refused_with_a_message() {
    let mut src = String::from("\\catcode`\\{=1 \\catcode`\\}=2\n");
    for i in 0..70_000 {
        src.push_str(&format!("\\message{{line {i}}}\n"));
    }
    src.push_str("\\end\n");
    let err = texrs::compile_text(&src).expect_err("the pool cannot hold it");
    assert!(
        err.0.contains("distinct strings"),
        "the refusal says what ran out: {}",
        err.0
    );
}
