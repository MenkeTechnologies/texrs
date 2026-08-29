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

/// `\message` — the pieces are on the stack, deepest first.
fn b_message(vm: &mut VM, argc: u8) -> Value {
    let mut parts = Vec::with_capacity(argc as usize);
    for _ in 0..argc {
        parts.push(render(&vm.pop()));
    }
    parts.reverse();
    MESSAGES.with(|m| m.borrow_mut().push(parts.concat()));
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
    MESSAGES.with(|m| m.borrow_mut().clear());
    let mut vm = VM::new(chunk);
    vm.register_builtin(ops::MESSAGE, b_message);
    match vm.run() {
        VMResult::Ok(_) | VMResult::Halted => Ok(MESSAGES.with(|m| m.borrow().clone())),
        VMResult::Error(e) => Err(e),
    }
}
