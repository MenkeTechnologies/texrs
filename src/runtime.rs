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

thread_local! {
    /// A fault raised inside a builtin, for [`run_with`] to surface after the VM
    /// returns.
    ///
    /// A builtin can only return a `Value`, not an error, so a failed `rustc`
    /// or a missing FFI function has nowhere to go but here: the builtin
    /// records the reason and halts the VM, and the runner turns the halt into
    /// the error the caller expects. Without it a broken `\rust{ … }` block
    /// would look like a document that simply printed nothing.
    static FAULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Record `msg` and stop the run.
fn fault(vm: &mut VM, msg: impl Into<String>) -> Value {
    FAULT.with(|f| *f.borrow_mut() = Some(msg.into()));
    vm.request_halt();
    Value::Undef
}

/// Compile and register a `\rust{ … }` block. One argument: the base64 body.
///
/// The error is raised on the VM rather than swallowed: a block that does not
/// compile is a broken document, and a run that quietly continued would fail
/// later at the call with a message about a missing function instead of the
/// rustc diagnostic that explains it.
fn b_ffi_compile(vm: &mut VM, _argc: u8) -> Value {
    let b64 = render(&vm.pop());
    match fusevm::ffi::compile_and_register(&b64) {
        Ok(()) => Value::Undef,
        Err(e) => fault(vm, e),
    }
}

/// Call a function a `\rust{ … }` block exported. The name is pushed first and
/// the arguments after it, so the stack is popped back to front.
fn b_ffi_call(vm: &mut VM, argc: u8) -> Value {
    let mut args = Vec::with_capacity(argc.saturating_sub(1) as usize);
    for _ in 1..argc {
        args.push(vm.pop());
    }
    args.reverse();
    let name = render(&vm.pop());
    match fusevm::ffi::try_call(&name, &args) {
        Some(Ok(v)) => v,
        Some(Err(e)) => fault(vm, e),
        // Not registered: either the block that exports it did not run, or the
        // name is misspelled. Both read the same way from here, so say both.
        None => fault(
            vm,
            format!("Undefined rust function {name} -- is its \\rust block above the call?"),
        ),
    }
}

/// Install the `\message` builtins on `vm`.
///
/// Shared by the interpreted path and the AOT runtime hook: a compiled document
/// must call the same two functions the interpreted one does, or its output
/// would be a different program's.
pub fn register_message_builtins(vm: &mut VM) {
    vm.register_builtin(ops::MSG_APPEND, b_msg_append);
    vm.register_builtin(ops::MSG_FLUSH, b_msg_flush);
    vm.register_builtin(ops::FFI_COMPILE, b_ffi_compile);
    vm.register_builtin(ops::FFI_CALL, b_ffi_call);
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
    FAULT.with(|f| *f.borrow_mut() = None);
    let mut vm = VM::new(chunk);
    register_message_builtins(&mut vm);
    if let Some(f) = on_line {
        vm.register_builtin(ops::DBG_LINE, f);
    }
    let result = vm.run();
    // A builtin that faulted halted the VM, so the halt has to be read as the
    // error it stands for rather than as a clean finish.
    if let Some(f) = FAULT.with(|f| f.borrow_mut().take()) {
        return Err(f);
    }
    match result {
        VMResult::Ok(_) | VMResult::Halted => Ok(MESSAGES.with(|m| m.borrow().clone())),
        VMResult::Error(e) => Err(e),
    }
}
