//! Run a compiled chunk on fusevm.
//!
//! The only host callback this milestone needs is `\message`. It pops the
//! rendered pieces the compiler pushed and joins them, which is why a message
//! containing `\the\count0` reads the register at RUN time through a slot rather
//! than being frozen when the file was read.

use crate::compiler::ops;
use fusevm::{VMResult, Value, VM};
use std::cell::RefCell;

thread_local! {
    /// What `\message` has written, in order.
    static MESSAGES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// The message currently being assembled.
    static BUILDING: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Append one piece to the message being built.
fn b_msg_append(vm: &mut VM, _argc: u8) -> Value {
    let piece = render(&vm.pop());
    BUILDING.with(|b| b.borrow_mut().push_str(&piece));
    Value::Undef
}

/// Finish the message and record it.
fn b_msg_flush(_vm: &mut VM, _argc: u8) -> Value {
    let done = BUILDING.with(|b| std::mem::take(&mut *b.borrow_mut()));
    MESSAGES.with(|m| m.borrow_mut().push(done));
    Value::Undef
}

/// How a value prints inside a message. An integer prints as TeX prints one;
/// there are no floats in this subset, so anything else is a string already.
fn render(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

/// Run `chunk` and return the messages it wrote.
pub fn run(chunk: fusevm::Chunk) -> Result<Vec<String>, String> {
    run_with(chunk, None)
}

/// The same, with the `--dap` statement-marker builtin installed.
///
/// The tracing JIT is deliberately NOT enabled here: it compiles hot code and
/// the compiled form does not call the marker, so a debugger would silently stop
/// stopping. A debug run is an interpreted run.
pub fn run_debug(
    chunk: fusevm::Chunk,
    on_line: fn(&mut VM, u8) -> Value,
) -> Result<Vec<String>, String> {
    run_with(chunk, Some(on_line))
}

/// Install the `\message` builtins on `vm`.
///
/// Shared by the interpreted path and the AOT runtime hook: a compiled document
/// must call the same two functions the interpreted one does, or its output
/// would be a different program's.
pub fn register_message_builtins(vm: &mut VM) {
    vm.register_builtin(ops::MSG_APPEND, b_msg_append);
    vm.register_builtin(ops::MSG_FLUSH, b_msg_flush);
}

/// Take what `\message` has written so far, clearing the buffer.
///
/// The AOT entry needs this: fusevm runs the chunk and hands back an exit code,
/// not the messages, which live in this module's thread-local.
pub fn take_messages() -> Vec<String> {
    MESSAGES.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

fn run_with(
    chunk: fusevm::Chunk,
    on_line: Option<fn(&mut VM, u8) -> Value>,
) -> Result<Vec<String>, String> {
    MESSAGES.with(|m| m.borrow_mut().clear());
    BUILDING.with(|b| b.borrow_mut().clear());
    let mut vm = VM::new(chunk);
    register_message_builtins(&mut vm);
    if let Some(f) = on_line {
        vm.register_builtin(ops::DBG_LINE, f);
    }
    match vm.run() {
        VMResult::Ok(_) | VMResult::Halted => Ok(MESSAGES.with(|m| m.borrow().clone())),
        VMResult::Error(e) => Err(e),
    }
}
