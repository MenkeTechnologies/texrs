//! `\openout`, `\write`, `\closeout` and `\special`: `tex.web` §1341-§1378 as
//! LuaTeX extends them in `extensions.c` and `texfileio.c`.
//!
//! Each of the four is a WHATSIT: a node put into the current list whose
//! effect happens when the page holding it is shipped out (§1373's `out_what`),
//! unless `\immediate` made it happen at once. This file holds the parts that
//! do not depend on where a whatsit sits in a list:
//!
//! - what the four whatsits carry ([`Whatsit`]), which is what the node list a
//!   page is built from holds and what ship-out hands back to the lowerer;
//! - how a stream number is read and what it means ([`Stream`]);
//! - the name an `\openout` file is given ([`out_file_name`]) and where it is
//!   put ([`out_path`]);
//! - the open files themselves, held by the run ([`b_out_open`],
//!   [`b_out_write`], [`b_out_close`]), because a `\write`'s text carries
//!   register reads that only the run can answer;
//! - and what the LOWERING has written ([`Written`]), because texrs lowers the
//!   whole document before it runs it, and a file written with
//!   `\immediate\write` and then read back with `\input` -- `filecontents`,
//!   or LaTeX reading its `.aux` again at `\end{document}` -- has to be read as
//!   it now stands and not as the run before left it.
//!
//! What is not here is the placement of a deferred whatsit in a list: that is
//! the node-list stomach's, which appends one wherever `\write` stood, with
//! what the lowerer's `ship_whatsit` made of it compiled to run when the
//! page holding it ships (`crate::stomach::run::out_what`).

use crate::token::Token;
use fusevm::{Value, VM};
use std::cell::RefCell;
use std::io::Write as _;

/// LuaTeX's `last_file_selector` (`printing.h`): 128 write streams, 0..=127,
/// where `tex.web` §1342 has sixteen.
pub const WRITE_STREAMS: usize = 128;

/// Where a `\write` to stream `n` goes when that stream is not open.
///
/// §1350 (`new_write_whatsit` in `extensions.c`): a negative number is
/// `term_only` and a number past the last stream is `term_and_log`, and
/// §1370's `write_out` then sends the first to the log alone and the second to
/// the terminal and the log -- as it does a number naming a stream that is
/// closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// One of the 128 streams `\openout` can open.
    File(usize),
    /// A negative number: the log alone.
    LogOnly,
    /// A number past the last stream: the terminal and the log.
    Terminal,
}

impl Stream {
    /// What a `\write` number means (`new_write_whatsit` with `check` false).
    pub fn of(n: i64) -> Stream {
        match n {
            n if n < 0 => Stream::LogOnly,
            n if n >= WRITE_STREAMS as i64 => Stream::Terminal,
            n => Stream::File(n as usize),
        }
    }

    /// The number the run is told: the stream itself, or -1 and 128 for the
    /// two that never name a file.
    pub fn code(self) -> i64 {
        match self {
            Stream::File(n) => n as i64,
            Stream::LogOnly => -1,
            Stream::Terminal => WRITE_STREAMS as i64,
        }
    }
}

/// One of the whatsits of §1341, as the list a page is built from holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Whatsit {
    /// `\openout<n>=<file>`: the stream, already checked to be in range
    /// (`scan_limited_int(last_file_selector)`), and the file name as it was
    /// scanned -- `.tex` is supplied when it is opened, not when it is read.
    Open { stream: usize, name: String },
    /// `\write<n>{<text>}`: the text UNEXPANDED (§1352), since §1370 expands it
    /// when it is written.
    Write { stream: Stream, tokens: Vec<Token> },
    /// `\closeout<n>`.
    Close { stream: usize },
    /// `\special{<text>}`, expanded when it was read as `\xdef` expands
    /// (§1354), or kept unexpanded for ship-out under `shipout` (LuaTeX's
    /// `late_special_node`).
    Special { tokens: Vec<Token>, late: bool },
    /// One of LuaTeX's PDF backend whatsits (`crate::backend`).
    Pdf(crate::backend::PdfWhatsit),
}

/// §1374 (`wrapup_leader` in `texfileio.c`): the name an `\openout` file is
/// written under, `.tex` supplied when the name has no extension of its own.
///
/// Measured against luatex 1.24.0: `\openout3=foo` writes `foo.tex`,
/// `\openout5=q.aux` writes `q.aux` and `\openout4={bar baz}` writes
/// `bar baz.tex`.
pub fn out_file_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.contains('.') {
        true => name.to_string(),
        false => format!("{name}.tex"),
    }
}

/// `-output-directory`, which a relative output name is written under. One
/// for the process rather than a thread, because a batch sets its documents on
/// a pool and the option is the command line's.
static OUTPUT_DIRECTORY: std::sync::RwLock<Option<std::path::PathBuf>> =
    std::sync::RwLock::new(None);

/// Set the directory the run writes into, as `-output-directory` asks.
pub fn set_output_directory(dir: Option<std::path::PathBuf>) {
    if let Ok(mut d) = OUTPUT_DIRECTORY.write() {
        *d = dir;
    }
}

/// Where a file named `name` is written: under the output directory when one
/// was given -- by `-output-directory`, or by `TEXMF_OUTPUT_DIRECTORY` as
/// web2c's `open_out_or_pipe` reads it -- and the name is relative, and where
/// it says otherwise.
pub fn out_path(name: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(name);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let dir = OUTPUT_DIRECTORY
        .read()
        .ok()
        .and_then(|d| d.clone())
        .or_else(|| std::env::var_os("TEXMF_OUTPUT_DIRECTORY").map(Into::into));
    match dir {
        Some(dir) => dir.join(path),
        None => path.to_path_buf(),
    }
}

/// How a `\write`'s text goes into a FILE: `lprint` in `extensions.c`, which
/// ends the line at `\newlinechar` and writes every other character as it is.
///
/// The terminal is different -- §49's `^^` notation for what it cannot show,
/// which is `crate::ir::printed` -- and measured, luatex writes `^^A` as the one
/// byte 1 into the file and `é` as its two UTF-8 bytes.
pub fn printed_to_file(text: &str, new_line_char: i64) -> String {
    let nl = u32::try_from(new_line_char).ok().and_then(char::from_u32);
    text.chars()
        .map(|c| match Some(c) == nl {
            true => '\n',
            false => c,
        })
        .collect()
}

/// [`printed_to_file`] over every piece of text a `\write` writes, as
/// `crate::ir::printed_ops` is over a message's for the terminal.
pub fn file_ops(ops: Vec<crate::ir::MsgOp>, new_line_char: i64) -> Vec<crate::ir::MsgOp> {
    use crate::ir::MsgOp;
    ops.into_iter()
        .map(|op| match op {
            MsgOp::Text(t) => MsgOp::Text(printed_to_file(&t, new_line_char)),
            MsgOp::If {
                left,
                rel,
                right,
                then_ops,
                else_ops,
            } => MsgOp::If {
                left,
                rel,
                right,
                then_ops: file_ops(then_ops, new_line_char),
                else_ops: file_ops(else_ops, new_line_char),
            },
            MsgOp::IfOdd {
                value,
                then_ops,
                else_ops,
            } => MsgOp::IfOdd {
                value,
                then_ops: file_ops(then_ops, new_line_char),
                else_ops: file_ops(else_ops, new_line_char),
            },
            other => other,
        })
        .collect()
}

/// The files the run has open, one slot a stream.
struct Outs {
    files: Vec<Option<std::io::BufWriter<std::fs::File>>>,
}

thread_local! {
    static OUTS: RefCell<Outs> = RefCell::new(Outs {
        files: (0..WRITE_STREAMS).map(|_| None).collect(),
    });
}

/// Close every stream: §1378's `final_cleanup` closing what the document left
/// open, and the state a fresh run starts in.
pub fn close_all() {
    OUTS.with(|o| {
        for file in o.borrow_mut().files.iter_mut() {
            if let Some(mut f) = file.take() {
                let _ = f.flush();
            }
        }
    });
}

/// Whether stream `n` is open in this run.
pub fn is_open(n: usize) -> bool {
    OUTS.with(|o| o.borrow().files.get(n).is_some_and(Option::is_some))
}

/// The `OUT_OPEN` builtin: the stream, then the file name. §1374 closes the
/// stream first and opens it again on the file, created empty.
///
/// A file that cannot be created is §530's `prompt_file_name` in a run that
/// cannot prompt, which is fatal: `I can't write on file`.
pub fn b_out_open(vm: &mut VM, _argc: u8) -> Value {
    let name = match vm.pop() {
        Value::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    };
    let stream = vm.pop().to_int();
    let Ok(n) = usize::try_from(stream) else {
        return Value::Undef;
    };
    if n >= WRITE_STREAMS {
        return Value::Undef;
    }
    let path = out_path(&name);
    let opened = std::fs::File::create(&path);
    OUTS.with(|o| {
        let mut o = o.borrow_mut();
        if let Some(mut old) = o.files[n].take() {
            let _ = old.flush();
        }
        if let Ok(file) = opened {
            o.files[n] = Some(std::io::BufWriter::new(file));
        }
    });
    if !is_open(n) {
        return crate::runtime::fault_now(vm, format!("I can't write on file `{name}'"));
    }
    Value::Undef
}

/// The `OUT_IS_OPEN` builtin: the stream; 1 when the run has it open.
pub fn b_out_is_open(vm: &mut VM, _argc: u8) -> Value {
    let stream = vm.pop().to_int();
    Value::Int(i64::from(usize::try_from(stream).is_ok_and(is_open)))
}

/// The `OUT_CLOSE` builtin: the stream (§1374's `close_write_file`).
pub fn b_out_close(vm: &mut VM, _argc: u8) -> Value {
    let stream = vm.pop().to_int();
    if let Ok(n) = usize::try_from(stream) {
        OUTS.with(|o| {
            if let Some(Some(mut f)) = o.borrow_mut().files.get_mut(n).map(Option::take) {
                let _ = f.flush();
            }
        });
    }
    Value::Undef
}

/// Write `line` and a line end to stream `n` if it is open, saying whether it
/// was.
pub fn write_line(n: i64, line: &str) -> bool {
    let Ok(n) = usize::try_from(n) else {
        return false;
    };
    OUTS.with(|o| match o.borrow_mut().files.get_mut(n) {
        Some(Some(f)) => {
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(b"\n");
            true
        }
        _ => false,
    })
}

/// What the lowering has written into each file it has opened, so a file read
/// back later in the same lowering is read as it now stands.
///
/// The run writes the files; this is the lowerer's copy of the same text, made
/// with the register values the lowerer knows, and consulted by `\input` and
/// `\openin` before the disk is. A file is entered when its stream is opened
/// and stays readable after it is closed, as a file on disk does.
#[derive(Default, Debug)]
pub struct Written {
    /// The file each open stream is writing, by name as `out_file_name` gave it.
    streams: std::collections::HashMap<usize, String>,
    /// What each file holds so far.
    files: std::collections::HashMap<String, String>,
}

impl Written {
    pub fn open(&mut self, stream: usize, name: &str) {
        self.streams.insert(stream, name.to_string());
        self.files.insert(name.to_string(), String::new());
    }

    /// Append a line to the file stream `n` is writing, saying whether one is
    /// open.
    pub fn write(&mut self, stream: Stream, line: &str) -> bool {
        let Stream::File(n) = stream else {
            return false;
        };
        let Some(name) = self.streams.get(&n) else {
            return false;
        };
        let file = self.files.entry(name.clone()).or_default();
        file.push_str(line);
        file.push('\n');
        true
    }

    pub fn close(&mut self, stream: usize) {
        self.streams.remove(&stream);
    }

    /// Whether the lowering believes stream `n` is open.
    pub fn is_open(&self, stream: usize) -> bool {
        self.streams.contains_key(&stream)
    }

    /// The text of `name` if this lowering wrote it. `name` is matched as the
    /// writer named it, with a leading `./` taken off.
    pub fn text(&self, name: &str) -> Option<&str> {
        let name = name.strip_prefix("./").unwrap_or(name);
        self.files.get(name).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// luatex's naming, measured: `.tex` only when the name carries no
    /// extension, and an extension is looked for in the last component only.
    #[test]
    fn an_output_name_gets_tex_only_when_it_has_no_extension() {
        assert_eq!(out_file_name("foo"), "foo.tex");
        assert_eq!(out_file_name("q.aux"), "q.aux");
        assert_eq!(out_file_name("bar baz"), "bar baz.tex");
        assert_eq!(out_file_name("dir.d/x"), "dir.d/x.tex");
        assert_eq!(out_file_name("case.out"), "case.out");
    }

    /// `new_write_whatsit`: below zero is the log, past 127 the terminal.
    #[test]
    fn a_write_number_names_a_stream_the_log_or_the_terminal() {
        assert_eq!(Stream::of(-1), Stream::LogOnly);
        assert_eq!(Stream::of(0), Stream::File(0));
        assert_eq!(Stream::of(127), Stream::File(127));
        assert_eq!(Stream::of(128), Stream::Terminal);
        assert_eq!(Stream::of(16), Stream::File(16));
    }

    /// A file gets its bytes raw; only `\newlinechar` is turned into a line end.
    #[test]
    fn a_file_line_is_raw_but_for_the_newline_character() {
        assert_eq!(printed_to_file("a\u{1}é", -1), "a\u{1}é");
        assert_eq!(printed_to_file("a^b", '^' as i64), "a\nb");
    }

    /// What the lowering wrote is readable as it now stands, after the stream
    /// is closed, and a write to a stream it never opened goes nowhere.
    #[test]
    fn the_lowering_reads_back_what_it_wrote() {
        let mut w = Written::default();
        assert!(!w.write(Stream::File(3), "lost"));
        w.open(3, "foo.tex");
        assert!(w.write(Stream::File(3), "one"));
        assert!(w.write(Stream::File(3), "two"));
        w.close(3);
        assert!(!w.write(Stream::File(3), "after"));
        assert_eq!(w.text("./foo.tex"), Some("one\ntwo\n"));
        // Opening it again starts it empty, as `File::create` does.
        w.open(4, "foo.tex");
        assert_eq!(w.text("foo.tex"), Some(""));
    }
}
