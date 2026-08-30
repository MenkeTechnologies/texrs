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

thread_local! {
    /// The document's own text, in the order it was read.
    static TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Append a run of the document's text.
fn b_text(vm: &mut VM, _argc: u8) -> Value {
    let piece = render(&vm.pop());
    TEXT.with(|t| t.borrow_mut().push_str(&piece));
    Value::Undef
}

/// Open a colour. Recorded in the text stream as a marker the typesetter
/// turns into a DVI `\special`, because the text is what carries order --
/// a colour that arrived out of order would paint the wrong words.
fn b_color_push(vm: &mut VM, _argc: u8) -> Value {
    let b = render(&vm.pop());
    let g = render(&vm.pop());
    let r = render(&vm.pop());
    TEXT.with(|t| t.borrow_mut().push_str(&format!("\u{1}{r},{g},{b}\u{2}")));
    Value::Undef
}

/// Close the innermost colour.
fn b_color_pop(_vm: &mut VM, _argc: u8) -> Value {
    TEXT.with(|t| t.borrow_mut().push('\u{3}'));
    Value::Undef
}

/// Everything the document said, and clear it for the next run.
pub fn take_text() -> String {
    TEXT.with(|t| std::mem::take(&mut *t.borrow_mut()))
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

/// `\multiply` and `\divide`, under TeX's rule rather than the machine's.
///
/// `tex.web` §1236 checks both: a product or quotient outside the 32-bit range
/// raises `Arithmetic overflow` and LEAVES THE REGISTER ALONE, and so does a
/// division by zero. Measured against tex 3.141592653: `\count1=2000000000
/// \multiply\count1 by 2` raises and the register still reads 2000000000.
///
/// texrs stops the run where tex reports and carries on -- its error model, and
/// the one difference recorded in BUGS.md rather than papered over.
fn b_arith_checked(vm: &mut VM, _argc: u8) -> Value {
    let which = vm.pop().to_int();
    let operand = vm.pop().to_int();
    let old = vm.pop().to_int();
    let result = match which {
        0 => old.checked_mul(operand),
        _ => match operand {
            0 => None,
            d => old.checked_div(d),
        },
    };
    match result {
        Some(v) if (i32::MIN as i64..=i32::MAX as i64).contains(&v) => Value::Int(v),
        _ => fault(vm, "Arithmetic overflow"),
    }
}

/// Close an `\input` file: append `)` to the message already written.
///
/// Not a message of its own, because the stream is joined with spaces and tex
/// writes the paren hard against what came before it. With nothing written yet
/// the open paren IS the last message, so an empty file closes correctly too.
fn b_msg_close(_vm: &mut VM, _argc: u8) -> Value {
    MESSAGES.with(|m| {
        let mut m = m.borrow_mut();
        match m.last_mut() {
            Some(last) => last.push(')'),
            None => m.push(")".to_string()),
        }
    });
    Value::Int(0)
}

/// Append a dimension the way TeX writes one: `print_scaled`, then `pt`.
fn b_msg_dimen(vm: &mut VM, _argc: u8) -> Value {
    let sp = vm.pop().to_int();
    let text = format!("{}pt", crate::dimen::print_scaled(sp));
    BUILDING.with(|b| b.borrow_mut().push_str(&text));
    Value::Int(0)
}

/// Install the `\message` builtins on `vm`.
///
/// Shared by the interpreted path and the AOT runtime hook: a compiled document
/// must call the same two functions the interpreted one does, or its output
/// would be a different program's.
pub fn register_message_builtins(vm: &mut VM) {
    vm.register_builtin(ops::TEXT, b_text);
    vm.register_builtin(ops::COLOR_PUSH, b_color_push);
    vm.register_builtin(ops::COLOR_POP, b_color_pop);
    vm.register_builtin(ops::MSG_APPEND, b_msg_append);
    vm.register_builtin(ops::MSG_FLUSH, b_msg_flush);
    vm.register_builtin(ops::MSG_CLOSE, b_msg_close);
    vm.register_builtin(ops::MSG_DIMEN, b_msg_dimen);
    vm.register_builtin(ops::ARITH_CHECKED, b_arith_checked);
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
    // The decision needs the ops, and `VM::new` takes the chunk by value.
    let chunk_for_jit = chunk.clone();
    let mut vm = VM::new(chunk);
    register_message_builtins(&mut vm);
    match on_line {
        // A debug run stays interpreted. The tracing JIT compiles a hot loop
        // into native code that does not call the `DBG_LINE` marker, so a
        // debugger under it would silently stop stopping -- which is why
        // `--dap` asks for the marker builtin and gets the interpreter with it.
        Some(f) => vm.register_builtin(ops::DBG_LINE, f),
        // Everything else gets the JIT the crate has been compiling in and
        // never switching on. A TeX loop lowers to a rotated conditional back
        // edge, which is the shape fusevm's trace compiler accepts; without
        // this it recorded the trace and had nowhere to install it.
        // ... but ONLY for a chunk with a loop in it. A tracing JIT has
        // nothing to offer a straight-line program, and switching it on for one
        // is not free: fusevm 0.26.0 SEGVs in native code when three GROWING
        // loop-free documents run in one thread, which is exactly what the REPL
        // does at every prompt. `tests/jit_reentry.rs` holds the reproducer and
        // BUGS.md records it. Gating on the shape the JIT is for keeps every
        // loop at full speed and takes the crash off the path that could not
        // have used it anyway.
        None if crate::tiers::has_loop(&chunk_for_jit) => vm.enable_tracing_jit(),
        None => {}
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
