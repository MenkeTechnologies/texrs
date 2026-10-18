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
    /// How much of [`BUILDING`] is reports rather than the message's own
    /// text: the reports a message opens with. An error the run raises while
    /// a message is being assembled goes at the end of these, because tex
    /// expands the whole text -- reporting what goes wrong -- before it prints
    /// any of it (§1279).
    static REPORTS_END: RefCell<usize> = const { RefCell::new(0) };
}

thread_local! {
    /// Whether the message being built OPENS with a report. §82's
    /// `print_err` begins with `print_nl`, which starts a fresh line rather
    /// than writing the space `\message` puts between two messages -- so such
    /// a message is written hard against the one before it, not after a space.
    static GLUED: RefCell<bool> = const { RefCell::new(false) };
}

thread_local! {
    /// Whether the next message is glued to the one before it because that
    /// one was an `\immediate\write`: §1370 ends a write with `print_ln`, which
    /// leaves `term_offset` at zero, and §1280 writes the space in front of a
    /// `\message` only when it is not.
    static GLUE_NEXT: RefCell<bool> = const { RefCell::new(false) };
}

thread_local! {
    /// The document's own text, in the order it was read.
    static TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

thread_local! {
    /// `tex.web` §311's context display for the command about to run, put here
    /// by [`b_err_site`] and read by whatever reports.
    ///
    /// A compiled engine has no input stack when a run-time error happens, so
    /// the display cannot be derived then; the lowerer takes it off the mouth
    /// and the chunk carries it.
    static ERROR_SITE: RefCell<String> = const { RefCell::new(String::new()) };
    /// Whether anything was REPORTED during the run, which is what §1335
    /// consults before printing `(see the transcript file …)`.
    static REPORTED: RefCell<bool> = const { RefCell::new(false) };
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

/// Record a finished message: as a message of its own, or -- when it opens
/// with a report -- appended to the last one, with nothing between.
fn record(done: String) {
    let glued = GLUED.with(|g| std::mem::take(&mut *g.borrow_mut()))
        | GLUE_NEXT.with(|g| std::mem::take(&mut *g.borrow_mut()));
    MESSAGES.with(|m| {
        let mut m = m.borrow_mut();
        match (glued, m.last_mut()) {
            (true, Some(last)) => last.push_str(&done),
            _ => m.push(done),
        }
    });
}

/// What the stomach said during one list action, into the terminal stream
/// in order (see `crate::stomach::Said`). `online` is `\tracingonline>0`.
pub(crate) fn tell(said: Vec<crate::stomach::Said>, online: bool) {
    use crate::stomach::Said;
    for s in said {
        match s {
            Said::Error(msg) => report(&msg),
            // §245: on the terminal only when `\tracingonline` is positive;
            // in the log alone otherwise, which the terminal learns of as
            // `history=warning_issued`.
            Said::Diagnostic(text) => match online {
                true => push_report(&text.replace('\n', "")),
                false => REPORTED.with(|r| *r.borrow_mut() = true),
            },
            // texrs writes its terminal reports without the line ends tex
            // puts in them, as `report` does with an error's context.
            Said::Report(text) => push_report(&text.replace('\n', "")),
            Said::Terminal(text) => {
                // A report ends with `print_ln` (§663), so what §638 prints
                // after one starts its line with no space before it.
                let after_report = BUILDING.with(|b| {
                    let len = b.borrow().len();
                    len > 0 && REPORTS_END.with(|end| *end.borrow() == len)
                });
                let held = take_building();
                if after_report {
                    record(held + &text);
                    continue;
                }
                if !held.is_empty() {
                    record(held);
                }
                record(text);
            }
        }
    }
}

/// Append text a builtin rendered to the message being built.
pub(crate) fn append_to_message(text: &str) {
    BUILDING.with(|b| b.borrow_mut().push_str(text));
}

/// Append a report to the message being built; see [`GLUED`].
fn push_report(text: &str) {
    BUILDING.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_empty() {
            GLUED.with(|g| *g.borrow_mut() = true);
        }
        REPORTS_END.with(|end| {
            let mut end = end.borrow_mut();
            if *end == b.len() {
                *end += text.len();
            }
        });
        b.push_str(text);
    });
}

/// Put a report in front of the text of the message being built, after the
/// reports it already opens with (see [`REPORTS_END`]).
fn report_before_text(text: &str) {
    REPORTED.with(|r| *r.borrow_mut() = true);
    BUILDING.with(|b| {
        let mut b = b.borrow_mut();
        REPORTS_END.with(|end| {
            let mut end = end.borrow_mut();
            let at = (*end).min(b.len());
            if at == 0 {
                GLUED.with(|g| *g.borrow_mut() = true);
            }
            b.insert_str(at, text);
            *end = at + text.len();
        });
    });
}

/// The message being built, taken: what it held, and no reports in front.
fn take_building() -> String {
    REPORTS_END.with(|end| *end.borrow_mut() = 0);
    BUILDING.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

/// One step of an e-TeX expression: the operands, the operation's code and
/// the bound (`crate::etex_arith::apply`).
fn b_expr_step(vm: &mut VM, argc: u8) -> Value {
    let max = vm.pop().to_int();
    let code = vm.pop().to_int();
    let mut args: Vec<i64> = (2..argc).map(|_| vm.pop().to_int()).collect();
    args.reverse();
    match crate::etex_arith::Op::from_code(code) {
        Some(op) => Value::Int(crate::etex_arith::apply(op, max, &args)),
        None => Value::Int(crate::etex_arith::ERROR),
    }
}

/// The end of an expression (the last lines of luatex's `scan_expr`): a step
/// that went out of range makes the whole value zero, reported as
/// `Arithmetic overflow` with the context the lowerer took where the
/// expression ended. Two arguments: the value, then that report.
fn b_expr_end(vm: &mut VM, _argc: u8) -> Value {
    let site = render(&vm.pop());
    match vm.pop().to_int() {
        crate::etex_arith::ERROR => {
            report_before_text(&site);
            Value::Int(0)
        }
        v => Value::Int(v),
    }
}

/// A report the mouth or the lowerer made, carried to its place in the run.
fn b_msg_report(vm: &mut VM, _argc: u8) -> Value {
    let text = render(&vm.pop());
    // Tracing output is printed but is no error; `warning_issued` prints
    // nothing and only makes the run end with the transcript notice.
    match text.strip_prefix(crate::expand::DIAGNOSTIC) {
        Some(shown) => push_report(shown),
        None if text == crate::expand::WARNING_ISSUED => {
            REPORTED.with(|r| *r.borrow_mut() = true);
        }
        None => {
            REPORTED.with(|r| *r.borrow_mut() = true);
            push_report(&text);
        }
    }
    Value::Undef
}

/// Append one piece to the message being built.
fn b_msg_append(vm: &mut VM, _argc: u8) -> Value {
    let piece = render(&vm.pop());
    BUILDING.with(|b| b.borrow_mut().push_str(&piece));
    Value::Undef
}

/// Finish the message and record it.
fn b_msg_flush(_vm: &mut VM, _argc: u8) -> Value {
    let done = take_building();
    record(done);
    Value::Undef
}

/// Finish an `\immediate\write` and record it on a line of its own (see
/// [`GLUE_NEXT`]): §1370's `print_nl` glues it to what came before, and its
/// `print_ln` glues what comes after to it.
fn b_write_flush(_vm: &mut VM, _argc: u8) -> Value {
    let done = take_building();
    GLUED.with(|g| *g.borrow_mut() = true);
    record(done);
    GLUE_NEXT.with(|g| *g.borrow_mut() = true);
    Value::Undef
}

/// Finish a `\write` to an open stream: §1370's `write_out` once the text is
/// built, with the selector on the file. One argument, the stream. The text
/// was made for the file while lowering (`crate::write::file_ops`).
fn b_out_write(vm: &mut VM, _argc: u8) -> Value {
    let stream = vm.pop().to_int();
    let done = BUILDING.with(|b| std::mem::take(&mut *b.borrow_mut()));
    crate::write::write_line(stream, &done);
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

/// [`fault`], for a builtin that lives outside this module: `crate::write`'s
/// `\openout` on a file that cannot be created.
pub(crate) fn fault_now(vm: &mut VM, msg: impl Into<String>) -> Value {
    fault(vm, msg)
}

/// Report `msg` and CARRY ON — `tex.web` §82's `print_err` followed by `error`,
/// which is what all but §93's fatal conditions do.
///
/// The report is pushed onto the message being BUILT rather than recorded as a
/// message of its own, and that is the whole formatting rule: `print_ln` ends
/// the error's context display and adds no character, so the `\message` that
/// follows begins hard against it on the same terminal line. It is the same
/// arrangement `lower.rs`'s `reports` makes for an error the mouth reported;
/// this is the run-time half, so that a `\multiply` overflow and a `\count`
/// constant that is too big read identically.
fn report(msg: &str) {
    let site = ERROR_SITE.with(|s| s.borrow().clone());
    REPORTED.with(|r| *r.borrow_mut() = true);
    push_report(&format!("! {msg}.{}", site.replace('\n', "")));
}

/// Record where the command about to run would be reported from. One argument.
fn b_err_site(vm: &mut VM, _argc: u8) -> Value {
    let site = render(&vm.pop());
    ERROR_SITE.with(|s| *s.borrow_mut() = site);
    Value::Int(0)
}

/// §1335: what tex writes as a run closes. One argument: the `(\end occurred
/// ...)` lines, already parenthesised, or an empty string.
///
/// The `)` in front is the document's own closing paren, which tex writes when
/// the file ends and which is otherwise the last character of the run, so a
/// clean run with nothing to say writes nothing at all. When the run reported
/// something `(see the transcript file ...)` follows; its own closing paren is
/// left off, as the comparison against tex's output leaves it off.
fn b_transcript_notice(vm: &mut VM, _argc: u8) -> Value {
    let glued = vm.pop().to_int() != 0;
    let notes = render(&vm.pop());
    let reported = REPORTED.with(|r| *r.borrow());
    if !reported && notes.is_empty() {
        return Value::Int(0);
    }
    // What was held is glued (`record`); the notice is not -- tex writes the
    // closing paren after a space.
    let held = take_building();
    if !held.is_empty() {
        record(held);
    }
    let mut text = format!("){notes}");
    match reported {
        true => text.push_str("(see the transcript file for additional information"),
        // The stream ends where the last paren the run printed does, as the
        // comparison reads a terminal (`parity::messages_of`): the last note's
        // own `)` -- or, after luatex's group listing, the `)` that closes
        // the listing's last `(...)`.
        false => {
            if let Some(at) = text.rfind(')') {
                text.truncate(at);
            }
        }
    }
    // LuaTeX's paren is written hard against the last message.
    MESSAGES.with(|m| {
        let mut m = m.borrow_mut();
        match (glued, m.last_mut()) {
            (true, Some(last)) => last.push_str(&text),
            _ => m.push(text),
        }
    });
    Value::Int(0)
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
/// The recovery is tex's too: §1236 reports through `error` rather than
/// `fatal_error`, so the run CONTINUES with the register untouched. Returning
/// the old value is what leaves it untouched -- the caller's `SetSlot` writes
/// whatever comes back, and writing the old value is a no-op.
///
/// `which` is 0 for an integer product, 1 for a quotient, and 2 for a product
/// of a dimension or glue component, which §1240 bounds at 2^30-1.
fn b_arith_checked(vm: &mut VM, _argc: u8) -> Value {
    let which = vm.pop().to_int();
    let operand = vm.pop().to_int();
    let old = vm.pop().to_int();
    let result = match which {
        0 => old.checked_mul(operand),
        2 => old
            .checked_mul(operand)
            .filter(|v| v.abs() <= crate::dimen::MAX_DIMEN),
        _ => match operand {
            0 => None,
            d => old.checked_div(d),
        },
    };
    match result {
        Some(v) if (i32::MIN as i64..=i32::MAX as i64).contains(&v) => Value::Int(v),
        _ => {
            report("Arithmetic overflow");
            Value::Int(old)
        }
    }
}

/// `tex.web` §453's `<factor><internal unit>`: the internal dimension, the
/// factor's integer part, and its fraction in 65536ths.
///
/// §455's `attach_sign` reports a product outside the dimension range as
/// `Dimension too large` and carries on with the largest dimension.
fn b_scale_dimen(vm: &mut VM, _argc: u8) -> Value {
    let frac = vm.pop().to_int();
    let int = vm.pop().to_int();
    let v = vm.pop().to_int();
    match crate::dimen::scale_by_factor_checked(int, frac, v) {
        Ok(product) => Value::Int(product),
        Err(clamped) => {
            report("Dimension too large");
            Value::Int(clamped)
        }
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

/// Append a number as `\romannumeral` writes it (`tex.web` §69).
fn b_msg_roman(vm: &mut VM, _argc: u8) -> Value {
    let n = vm.pop().to_int();
    let text = crate::expand::roman_int(n);
    BUILDING.with(|b| b.borrow_mut().push_str(&text));
    Value::Int(0)
}

/// Append a glue the way TeX writes one.
fn b_msg_glue(vm: &mut VM, _argc: u8) -> Value {
    append_glue(vm, "pt")
}

/// The same in MATH units (`tex.web` §1060's `print_spec(p,"mu")`).
fn b_msg_muglue(vm: &mut VM, _argc: u8) -> Value {
    append_glue(vm, "mu")
}

fn append_glue(vm: &mut VM, unit: &str) -> Value {
    // Pushed natural, stretch, shrink, orders -- so they come back reversed.
    let orders = vm.pop().to_int();
    let shrink = vm.pop().to_int();
    let stretch = vm.pop().to_int();
    let natural = vm.pop().to_int();
    let text = crate::glue::print_glue_in(unit, natural, stretch, orders / 4, shrink, orders % 4);
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
    vm.register_builtin(ops::WRITE_FLUSH, b_write_flush);
    vm.register_builtin(ops::MSG_REPORT, b_msg_report);
    vm.register_builtin(ops::MSG_CLOSE, b_msg_close);
    vm.register_builtin(ops::MSG_DIMEN, b_msg_dimen);
    vm.register_builtin(ops::MSG_ROMAN, b_msg_roman);
    vm.register_builtin(ops::MSG_GLUE, b_msg_glue);
    vm.register_builtin(ops::ARITH_CHECKED, b_arith_checked);
    vm.register_builtin(ops::SCALE_DIMEN, b_scale_dimen);
    vm.register_builtin(ops::MSG_MUGLUE, b_msg_muglue);
    vm.register_builtin(ops::ERR_SITE, b_err_site);
    vm.register_builtin(ops::EXPR_STEP, b_expr_step);
    vm.register_builtin(ops::EXPR_END, b_expr_end);
    vm.register_builtin(ops::TRANSCRIPT_NOTICE, b_transcript_notice);
    vm.register_builtin(ops::FFI_COMPILE, b_ffi_compile);
    vm.register_builtin(ops::FFI_CALL, b_ffi_call);
    vm.register_builtin(ops::LIST, crate::stomach::run::b_list);
    vm.register_builtin(ops::LIST_SHOWBOX, crate::stomach::run::b_list_showbox);
    vm.register_builtin(ops::OUT_OPEN, crate::write::b_out_open);
    vm.register_builtin(ops::OUT_WRITE, b_out_write);
    vm.register_builtin(ops::OUT_CLOSE, crate::write::b_out_close);
    vm.register_builtin(ops::OUT_IS_OPEN, crate::write::b_out_is_open);
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
    take_building();
    GLUED.with(|g| *g.borrow_mut() = false);
    GLUE_NEXT.with(|g| *g.borrow_mut() = false);
    FAULT.with(|f| *f.borrow_mut() = None);
    ERROR_SITE.with(|s| s.borrow_mut().clear());
    REPORTED.with(|r| *r.borrow_mut() = false);
    crate::stomach::run::reset();
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
    finish_run()?;
    match result {
        VMResult::Ok(_) | VMResult::Halted => Ok(MESSAGES.with(|m| m.borrow().clone())),
        VMResult::Error(e) => Err(e),
    }
}

/// What a run owes once its VM has returned: the report still being built, and
/// the fault a builtin halted on, as the error it stands for.
///
/// Shared with the `--aot` entry (`src/aot_runtime.rs`), whose VM fusevm runs
/// and returns from without passing through [`run_with`].
pub(crate) fn finish_run() -> Result<(), String> {
    // A report with nothing printed after it still has to be printed. In an
    // ordinary run the buffer is empty here -- every `\message` flushes its
    // own -- so this only ever carries an error §82 reported after the last
    // one, which tex writes on the terminal as it happens.
    let held = take_building();
    if !held.is_empty() {
        record(held);
    }
    // §1333's `close_files_and_terminate`: every stream still open is closed,
    // so what was written reaches the disk before anything reads it.
    crate::write::close_all();
    // A builtin that faulted halted the VM, so the halt has to be read as the
    // error it stands for rather than as a clean finish.
    match FAULT.with(|f| f.borrow_mut().take()) {
        Some(f) => Err(f),
        None => Ok(()),
    }
}
