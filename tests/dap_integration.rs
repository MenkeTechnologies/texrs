//! End-to-end DAP integration test for `texrs --dap`.
//!
//! Spawns the built binary over stdio pipes and drives a real Debug Adapter
//! Protocol session — initialize → setBreakpoints → launch → configurationDone —
//! then asserts the run stops on the breakpoint line, reports the right frame
//! and count registers, single-steps to the next line, and terminates on
//! `continue`. Headless and dependency-free beyond the built binary and
//! serde_json, so it runs in CI with no editor and no TeX installation.
//!
//! What is debuggable is what survives lowering: macros expand at compile time,
//! so the lines a breakpoint can stop on are the ones that left run-time work
//! behind. The document below is written so each such line is a statement of
//! its own.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

/// Line 2 assigns, line 3 advances, line 4 multiplies, line 5 prints — one
/// run-time statement per line, so a line breakpoint maps to exactly one stop.
const PROGRAM: &str = "\\catcode`\\{=1 \\catcode`\\}=2\n\\count1=7\n\\advance\\count1 by 5\n\\multiply\\count1 by 3\n\\message{count=\\the\\count1}\n\\end\n";

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    seq: i64,
    output: String,
    stops_seen: usize,
}

impl Session {
    fn start(path: &str, lines: &[u32]) -> Self {
        let mut s = Self::spawn();
        s.send("initialize", json!({}));
        let bps: Vec<Value> = lines.iter().map(|l| json!({ "line": l })).collect();
        s.send(
            "setBreakpoints",
            json!({ "source": { "path": path }, "breakpoints": bps }),
        );
        s.send("launch", json!({ "program": path }));
        s.send("configurationDone", json!({}));
        s
    }

    fn spawn() -> Self {
        let bin = env!("CARGO_BIN_EXE_texrs");
        let mut child = Command::new(bin)
            .arg("--dap")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn texrs --dap");
        // Watchdog: a wedged debugger must fail the test, not hang CI.
        let id = child.id();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(20));
            let _ = Command::new("kill").arg("-9").arg(id.to_string()).status();
        });
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
            seq: 0,
            output: String::new(),
            stops_seen: 0,
        }
    }

    fn send(&mut self, command: &str, arguments: Value) {
        self.seq += 1;
        let msg = json!({
            "seq": self.seq, "type": "request", "command": command, "arguments": arguments,
        });
        let body = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).expect("write req");
        self.stdin.flush().expect("flush req");
    }

    fn read_msg(&mut self) -> Option<Value> {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let t = line.trim_end();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.strip_prefix("Content-Length:") {
                len = v.trim().parse().ok()?;
            }
        }
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).ok()?;
        let msg: Value = serde_json::from_slice(&buf).ok()?;
        if msg["type"] == "event" && msg["event"] == "output" {
            self.output
                .push_str(msg["body"]["output"].as_str().unwrap_or(""));
        }
        if msg["type"] == "event" && msg["event"] == "stopped" {
            self.stops_seen += 1;
        }
        Some(msg)
    }

    fn read_until(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..200 {
            match self.read_msg() {
                Some(m) if pred(&m) => return m,
                Some(_) => continue,
                None => break,
            }
        }
        panic!("did not receive {what} before EOF");
    }

    fn stopped(&mut self) -> Value {
        self.read_until("stopped event", |m| {
            m["type"] == "event" && m["event"] == "stopped"
        })
    }

    fn response(&mut self, command: &str) -> Value {
        self.read_until(&format!("{command} response"), |m| {
            m["type"] == "response" && m["command"] == command
        })
    }

    fn stack_line(&mut self) -> u64 {
        self.send("stackTrace", json!({ "threadId": 1 }));
        let r = self.response("stackTrace");
        r["body"]["stackFrames"][0]["line"].as_u64().unwrap()
    }

    /// The `\count` registers as the adapter reports them at the current stop.
    fn registers(&mut self) -> Vec<(String, String)> {
        self.send("variables", json!({ "variablesReference": 1 }));
        let r = self.response("variables");
        r["body"]["variables"]
            .as_array()
            .expect("variables array")
            .iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap_or("").to_string(),
                    v["value"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect()
    }

    fn run_to_end(&mut self) -> (String, bool) {
        for _ in 0..100 {
            match self.read_msg() {
                Some(m) if m["type"] == "event" && m["event"] == "terminated" => {
                    return (self.output.clone(), true);
                }
                Some(_) => continue,
                None => break,
            }
        }
        (self.output.clone(), false)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_program() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.tex");
    std::fs::write(&path, PROGRAM).expect("write program");
    let s = path.to_string_lossy().into_owned();
    (dir, s)
}

#[test]
fn a_breakpoint_stops_on_its_line_and_the_registers_read_back() {
    let (_dir, path) = write_program();
    // Line 4 multiplies, so at the stop line 2 and 3 have run: 7 + 5 = 12.
    let mut s = Session::start(&path, &[4]);
    let stop = s.stopped();
    assert_eq!(stop["body"]["reason"], "breakpoint", "{stop}");
    assert_eq!(s.stack_line(), 4, "stopped on the wrong line");

    let regs = s.registers();
    let one = regs
        .iter()
        .find(|(n, _)| n == "\\count1")
        .unwrap_or_else(|| panic!("\\count1 not reported: {regs:?}"));
    assert_eq!(
        one.1, "12",
        "the register should hold 7+5 before the multiply on this line: {regs:?}"
    );

    s.send("continue", json!({ "threadId": 1 }));
    let (out, terminated) = s.run_to_end();
    assert!(terminated, "no terminated event");
    assert!(
        out.contains("count=36"),
        "the document's message stream did not reach the client: {out:?}"
    );
}

#[test]
fn stepping_advances_one_statement_at_a_time() {
    let (_dir, path) = write_program();
    let mut s = Session::start(&path, &[3]);
    s.stopped();
    assert_eq!(s.stack_line(), 3);

    s.send("next", json!({ "threadId": 1 }));
    s.stopped();
    assert_eq!(s.stack_line(), 4, "a step did not advance exactly one line");

    // The advance on line 3 has run by now, the multiply on line 4 has not.
    let regs = s.registers();
    assert!(
        regs.iter().any(|(n, v)| n == "\\count1" && v == "12"),
        "registers after the step: {regs:?}"
    );

    s.send("continue", json!({ "threadId": 1 }));
    let (_out, terminated) = s.run_to_end();
    assert!(terminated);
}

#[test]
fn a_breakpoint_on_a_line_with_no_run_time_work_is_reported_unverified() {
    let (_dir, path) = write_program();
    let mut s = Session::spawn();
    s.send("initialize", json!({}));
    // Line 6 is `\end`, which stops lowering and leaves no run-time statement,
    // so a breakpoint there can never fire and must not be reported verified.
    s.send(
        "setBreakpoints",
        json!({ "source": { "path": path }, "breakpoints": [{ "line": 2 }, { "line": 6 }] }),
    );
    let r = s.response("setBreakpoints");
    let bps = r["body"]["breakpoints"].as_array().expect("breakpoints");
    assert_eq!(bps.len(), 2, "{r}");
    assert_eq!(bps[0]["verified"], true, "line 2 carries a statement: {r}");
    assert_eq!(bps[1]["verified"], false, "line 6 is \\end: {r}");
}

#[test]
fn a_document_with_no_breakpoints_runs_straight_through() {
    let (_dir, path) = write_program();
    let mut s = Session::start(&path, &[]);
    let (out, terminated) = s.run_to_end();
    assert!(terminated, "no terminated event");
    assert_eq!(s.stops_seen, 0, "stopped without a breakpoint: {out:?}");
    assert!(out.contains("count=36"), "output was {out:?}");
}
