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
    let got = ops("\\catcode`\\{=1 \\catcode`\\}=2\n\\count0=7 \\divide\\count0 by 2\n\\end\n");
    assert!(
        got.contains(&Op::TruncInt),
        "TeX's \\divide is integer division; got {got:?}"
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
    let body: Vec<u32> = chunk
        .ops
        .iter()
        .enumerate()
        .skip(512)
        .map(|(i, _)| chunk.lines[i])
        .collect();
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
