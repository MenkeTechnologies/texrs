//! Debug Adapter Protocol over stdio (`texrs --dap`).
//!
//! A single-threaded source-line debugger for a TeX document. The document is
//! compiled with per-statement line markers (`Op::CallBuiltin(DBG_LINE, 0)`,
//! emitted only in this mode — an ordinary run carries zero extra ops) and run
//! on the pure interpreter: the tracing JIT would compile hot code whose
//! compiled form does not call the marker, so a debugger under it would silently
//! stop stopping. The `DBG_LINE` builtin fires synchronously at each marker;
//! when it lands on a breakpoint or a step target it pauses IN PLACE and
//! services DAP requests (`stackTrace`/`scopes`/`variables`/`continue`/`next`/
//! `stepIn`/`stepOut`) from stdin until a resume command, then returns control
//! to the VM.
//!
//! What is debuggable is what survives lowering. Macro expansion happens at
//! COMPILE time, so a `\def` is not a line the debugger stops on — by the time
//! the VM runs, the macro is gone and its expansion is the code around it. What
//! remains is what has run-time state: register assignment and arithmetic,
//! conditionals, groups, and `\message`. The variables scope is therefore the
//! `\count` registers, read out of the VM's slots.
//!
//! There is no stdout redirection here, unlike the sibling frontends'. A TeX
//! document writes nothing as it runs: `\message` accumulates and texrs prints
//! the stream when the run ends, so the messages are forwarded as one `output`
//! event at exit and the protocol channel on stdout is never at risk.

use serde_json::{json, Value as J};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};

use fusevm::{Op, Value, VM};

/// How the debuggee should proceed from a stop.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Continue,
    Step,
}

struct DebugState {
    breakpoints: HashSet<u32>,
    /// Lines that actually carry a marker (so a breakpoint on them can fire).
    verified: HashSet<u32>,
    mode: Mode,
    /// The source line of the current stop (for the stack frame).
    cur_line: u32,
    /// Real stdout, saved at startup; all DAP protocol is written here.
    proto_fd: RawFd,
    /// Source path reported in stack frames.
    program: String,
    seq: i64,
    /// True once `launch` has redirected stdout and the debuggee is running.
    active: bool,
}

thread_local! {
    static DBG: RefCell<DebugState> = RefCell::new(DebugState {
        breakpoints: HashSet::new(),
        verified: HashSet::new(),
        mode: Mode::Continue,
        cur_line: 0,
        proto_fd: 1,
        program: String::new(),
        seq: 1,
        active: false,
    });
}

/// Entry point for `texrs --dap`.
pub fn run() -> Result<(), String> {
    // Save the real stdout up front, and write every protocol frame to that
    // duplicate: whatever else touches fd 1 later cannot corrupt the channel.
    let proto = unsafe { libc::dup(1) };
    DBG.with(|d| d.borrow_mut().proto_fd = proto);

    let mut input = std::io::stdin();
    while let Some(msg) = read_message(&mut input)? {
        let command = msg.get("command").and_then(|c| c.as_str()).unwrap_or("");
        let req_seq = msg.get("seq").and_then(|s| s.as_i64()).unwrap_or(0);
        match command {
            "initialize" => {
                respond(
                    req_seq,
                    command,
                    json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsEvaluateForHovers": true,
                        "supportsTerminateRequest": true,
                    }),
                );
                event("initialized", json!({}));
            }
            "setBreakpoints" => set_breakpoints(&msg, req_seq),
            "setExceptionBreakpoints" => {
                // Accepted so clients that always send it proceed; the
                // single-threaded adapter does not stop on exceptions.
                respond(req_seq, command, json!({ "breakpoints": [] }));
            }
            "evaluate" => {
                respond(
                    req_seq,
                    command,
                    json!({ "result": "", "variablesReference": 0 }),
                );
            }
            "pause" => respond(req_seq, command, json!({})),
            "configurationDone" => respond(req_seq, command, json!({})),
            "threads" => respond(
                req_seq,
                command,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
            ),
            "launch" => {
                let program = msg
                    .get("arguments")
                    .and_then(|a| a.get("program"))
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                respond(req_seq, command, json!({}));
                launch(&program);
            }
            "disconnect" | "terminate" => {
                respond(req_seq, command, json!({}));
                break;
            }
            _ => respond(req_seq, command, json!({})),
        }
    }
    unsafe {
        libc::close(proto);
    }
    Ok(())
}

/// `setBreakpoints`: store the requested lines and report each verified only if
/// the program actually emits a marker on that line (a blank/comment line with
/// no compiled statement is reported unverified — a breakpoint there would never
/// fire).
fn set_breakpoints(msg: &J, req_seq: i64) {
    let path = msg
        .get("arguments")
        .and_then(|a| a.get("source"))
        .and_then(|s| s.get("path"))
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let lines: Vec<u32> = msg
        .get("arguments")
        .and_then(|a| a.get("breakpoints"))
        .and_then(|b| b.as_array())
        .map(|bps| {
            bps.iter()
                .filter_map(|b| b.get("line").and_then(|l| l.as_u64()).map(|l| l as u32))
                .collect()
        })
        .unwrap_or_default();

    let markers = marker_lines(&path);
    DBG.with(|d| {
        let mut s = d.borrow_mut();
        if !path.is_empty() {
            s.program = path;
        }
        s.breakpoints = lines.iter().copied().collect();
        s.verified = markers;
    });
    let bps: Vec<J> = DBG.with(|d| {
        let s = d.borrow();
        lines
            .iter()
            .map(|l| json!({ "verified": s.verified.contains(l), "line": l }))
            .collect()
    });
    respond(req_seq, "setBreakpoints", json!({ "breakpoints": bps }));
}

/// The set of source lines that carry a `DBG_LINE` marker in the compiled
/// program — the lines on which a breakpoint can actually stop.
fn marker_lines(path: &str) -> HashSet<u32> {
    let mut set = HashSet::new();
    let Ok(src) = std::fs::read_to_string(path) else {
        return set;
    };
    let Ok(chunk) = crate::compile_debug(&src) else {
        return set;
    };
    for (i, op) in chunk.ops.iter().enumerate() {
        if let Op::CallBuiltin(id, _) = op {
            if *id == crate::compiler::ops::DBG_LINE {
                if let Some(l) = chunk.lines.get(i) {
                    set.insert(*l);
                }
            }
        }
    }
    set
}

/// Run the document under the debugger, then forward its `\message` stream as
/// an output event and report termination.
///
/// The sibling frontends redirect the debuggee's stdout to a pipe here, because
/// their programs print as they run. A TeX document does not: `\message`
/// accumulates and is printed once at the end, so there is nothing to interleave
/// and nothing to redirect.
fn launch(program: &str) {
    if program.is_empty() {
        return;
    }
    DBG.with(|d| {
        let mut s = d.borrow_mut();
        if s.program.is_empty() {
            s.program = program.to_string();
        }
        s.mode = Mode::Continue;
        s.active = true;
    });

    let outcome = match std::fs::read_to_string(program) {
        Ok(src) => crate::run_messages_debug(&src).map_err(|e| format!("! {}.", e.0)),
        Err(e) => Err(format!("texrs: {program}: {e}")),
    };
    DBG.with(|d| d.borrow_mut().active = false);

    match outcome {
        Ok(msgs) => {
            let body = match msgs.is_empty() {
                true => String::new(),
                false => format!(" {msgs}"),
            };
            emit_output("stdout", &format!("(./{program}{body} )\n"));
        }
        Err(e) => emit_output("stderr", &format!("{e}\n")),
    }
    event("terminated", json!({}));
}

/// Called by the VM at each statement marker (via the `DBG_LINE` builtin). Reads
/// the marker's source line; if it is a breakpoint or a step target, pauses and
/// services DAP requests until a resume command.
pub fn on_debug_line(vm: &mut VM, _argc: u8) -> Value {
    let line = *vm.chunk.lines.get(vm.ip.saturating_sub(1)).unwrap_or(&0);
    if line == 0 {
        return Value::Undef;
    }
    let (stop, reason) = DBG.with(|d| {
        let mut s = d.borrow_mut();
        s.cur_line = line;
        if !s.active {
            return (false, "");
        }
        let bp = s.breakpoints.contains(&line) && s.verified.contains(&line);
        let step = s.mode == Mode::Step;
        let reason = if bp { "breakpoint" } else { "step" };
        (bp || step, reason)
    });
    if !stop {
        return Value::Undef;
    }
    event(
        "stopped",
        json!({
            "reason": reason,
            "threadId": 1,
            "allThreadsStopped": true,
        }),
    );

    // Service requests until a resume command returns control to the VM.
    let mut stdin = std::io::stdin();
    loop {
        match read_message(&mut stdin) {
            Ok(Some(msg)) => {
                if handle_stopped(vm, &msg) {
                    break;
                }
            }
            _ => {
                // EOF / read error: let the document run to completion.
                DBG.with(|d| d.borrow_mut().mode = Mode::Continue);
                break;
            }
        }
    }
    Value::Undef
}

/// The slots of the frame the VM is executing, which is where the count
/// registers live: a document compiles to one frame, so this is the base one.
fn current_slots(vm: &VM) -> &[Value] {
    match vm.frames.last() {
        Some(f) => &f.slots,
        None => &[],
    }
}

/// The count registers, as `(\count<n>, value)` pairs.
///
/// All 256 exist from the first instruction — the chunk's prologue writes zero
/// into every slot, because a read of an unwritten slot finds `Undef` — so
/// listing all of them would bury the two a document actually uses in 254 zeros.
/// Only the registers the document ASSIGNS are shown, which the chunk names by
/// its `SetSlot` ops, plus any that are non-zero at the stop.
fn count_registers(vm: &VM) -> Vec<(String, String)> {
    let mut interesting: Vec<u16> = vm
        .chunk
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::SetSlot(n) => Some(*n),
            _ => None,
        })
        .collect();
    for (i, v) in current_slots(vm).iter().enumerate() {
        if !matches!(v, Value::Int(0) | Value::Undef) {
            interesting.push(i as u16);
        }
    }
    interesting.sort_unstable();
    interesting.dedup();

    interesting
        .into_iter()
        .map(|n| {
            let value = match current_slots(vm).get(n as usize) {
                Some(Value::Int(i)) => i.to_string(),
                Some(Value::Undef) | None => "0".to_string(),
                Some(other) => format!("{other:?}"),
            };
            (format!("\\count{n}"), value)
        })
        .collect()
}

/// Handle one request while stopped. Returns true when a resume command
/// (`continue`/`next`/`stepIn`/`stepOut`) was processed and the VM should run on.
fn handle_stopped(vm: &VM, msg: &J) -> bool {
    let command = msg.get("command").and_then(|c| c.as_str()).unwrap_or("");
    let req_seq = msg.get("seq").and_then(|s| s.as_i64()).unwrap_or(0);
    match command {
        "threads" => {
            respond(
                req_seq,
                command,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
            );
            false
        }
        "stackTrace" => {
            let (program, line) = DBG.with(|d| {
                let s = d.borrow();
                (s.program.clone(), s.cur_line)
            });
            let frames = json!([{
                "id": 0,
                "name": "main",
                "line": line,
                "column": 1,
                "source": { "path": program },
            }]);
            respond(
                req_seq,
                command,
                json!({ "stackFrames": frames, "totalFrames": 1 }),
            );
            false
        }
        "scopes" => {
            respond(
                req_seq,
                command,
                json!({ "scopes": [{ "name": "Count registers", "variablesReference": 1, "expensive": false }] }),
            );
            false
        }
        "variables" => {
            let vars: Vec<J> = count_registers(vm)
                .into_iter()
                .map(|(n, v)| json!({ "name": n, "value": v, "variablesReference": 0 }))
                .collect();
            respond(req_seq, command, json!({ "variables": vars }));
            false
        }
        "setBreakpoints" => {
            set_breakpoints(msg, req_seq);
            false
        }
        "setExceptionBreakpoints" => {
            respond(req_seq, command, json!({ "breakpoints": [] }));
            false
        }
        "evaluate" => {
            let expr = msg
                .get("arguments")
                .and_then(|a| a.get("expression"))
                .and_then(|e| e.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let result = count_registers(vm)
                .into_iter()
                .find(|(n, _)| *n == expr)
                .map(|(_, v)| v)
                .unwrap_or_else(|| {
                    if expr.is_empty() {
                        String::new()
                    } else {
                        format!("<cannot evaluate `{expr}`>")
                    }
                });
            respond(
                req_seq,
                command,
                json!({ "result": result, "variablesReference": 0 }),
            );
            false
        }
        "pause" => {
            respond(req_seq, command, json!({}));
            false
        }
        "continue" => {
            DBG.with(|d| d.borrow_mut().mode = Mode::Continue);
            respond(req_seq, command, json!({ "allThreadsContinued": true }));
            true
        }
        "next" | "stepIn" | "stepOut" => {
            DBG.with(|d| d.borrow_mut().mode = Mode::Step);
            respond(req_seq, command, json!({}));
            true
        }
        "disconnect" | "terminate" => {
            DBG.with(|d| d.borrow_mut().mode = Mode::Continue);
            respond(req_seq, command, json!({}));
            true
        }
        _ => {
            respond(req_seq, command, json!({}));
            false
        }
    }
}

/// Forward text the document produced as an `output` event.
fn emit_output(category: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    event("output", json!({ "category": category, "output": text }));
}

// ---- wire protocol --------------------------------------------------------

/// Read one `Content-Length`-framed JSON message; `None` at EOF.
fn read_message(input: &mut std::io::Stdin) -> Result<Option<J>, String> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => {
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(e) => return Err(format!("dap read: {e}")),
        }
    }
    let header = String::from_utf8_lossy(&header);
    let len: usize = header
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length:"))
        .and_then(|v| v.trim().parse().ok())
        .ok_or("dap: missing Content-Length")?;
    let mut body = vec![0u8; len];
    input
        .read_exact(&mut body)
        .map_err(|e| format!("dap body: {e}"))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| format!("dap json: {e}"))
}

/// Write a framed JSON message to the saved protocol fd (never to fd 1, which is
/// the program's redirected stdout during a run).
fn send(msg: &J) {
    let body = msg.to_string();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let fd = DBG.with(|d| d.borrow().proto_fd);
    // SAFETY: `fd` is a valid duplicated stdout fd owned by this process; wrapped
    // in ManuallyDrop so the File does not close it on drop.
    unsafe {
        let mut f = std::mem::ManuallyDrop::new(std::fs::File::from_raw_fd(fd));
        let _ = f.write_all(frame.as_bytes());
        let _ = f.flush();
    }
}

fn next_seq() -> i64 {
    DBG.with(|d| {
        let mut s = d.borrow_mut();
        let n = s.seq;
        s.seq += 1;
        n
    })
}

fn respond(req_seq: i64, command: &str, body: J) {
    send(&json!({
        "seq": next_seq(),
        "type": "response",
        "request_seq": req_seq,
        "success": true,
        "command": command,
        "body": body,
    }));
}

fn event(ev: &str, body: J) {
    send(&json!({ "seq": next_seq(), "type": "event", "event": ev, "body": body }));
}
