//! Lower the command stream to fusevm bytecode.
//!
//! This is the whole reason texrs is a fusevm frontend rather than another
//! interpreter. A count register becomes a VM SLOT, so `\advance\count0 by 5` is
//! `GetSlot / LoadInt / Add / SetSlot` — native ops the JIT can compile — rather
//! than a hash lookup and a match in a tree-walker. A conditional becomes a real
//! branch (`Lt` + `JumpIfFalse`), not a Rust `if` deciding which subtree to walk.
//!
//! There are 256 count registers and they map onto slots 0..255 directly, which
//! keeps the mapping trivial to read and means a register read is an array index.

use std::collections::HashMap;

use crate::expand::TexError;
use crate::ir::{Arith, Cmd, MsgOp, Num, Rel};
use fusevm::{Chunk, ChunkBuilder, Op, Value};

/// Builtin ids this frontend registers on the VM.
pub mod ops {
    /// Append a run of the document's own text.
    pub const TEXT: u16 = 4005;
    /// Append one rendered piece to the message being built.
    pub const MSG_APPEND: u16 = 4000;
    /// Finish the message being built and record it.
    pub const MSG_FLUSH: u16 = 4001;
    /// Compile and register a `\rust{ … }` block: one argument, the base64 body.
    pub const FFI_COMPILE: u16 = 4003;
    /// Call a function a block exported: the name, then its arguments.
    pub const FFI_CALL: u16 = 4004;
    /// A statement boundary, emitted only under `--dap`. The debug adapter
    /// stops here; an ordinary run carries none of these ops.
    pub const DBG_LINE: u16 = 4002;
}

/// TeX has exactly 256 count registers (`tex.web` §236).
pub const COUNT_SLOTS: u16 = 256;

pub struct Compiler {
    b: ChunkBuilder,
    /// Emit a `DBG_LINE` marker at every statement boundary. Off for an
    /// ordinary run, so nothing pays for the debugger that is not using it.
    debug: bool,
    /// The source line the commands being lowered came from, updated by
    /// [`Cmd::Line`] and stamped onto every op emitted after it.
    line: u32,
    /// Pool index of every string already added, so a repeated one costs a
    /// lookup rather than a slot.
    ///
    /// A `LoadConst` operand is a u16, so the pool holds 65,536 entries and a
    /// frontend that adds one per emission is bounded by how much text a
    /// document has -- which for a book is not a bound at all. Coalescing text
    /// runs across line directives took the books under it; a 4 MB reference
    /// still went past. Text repeats -- the same words, the same spacing -- so
    /// interning bounds the pool by what a document SAYS rather than by how
    /// often it says it.
    strings: HashMap<String, u16>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            b: ChunkBuilder::new(),
            debug: false,
            line: 0,
            strings: HashMap::new(),
        }
    }

    /// A compiler that also emits the `--dap` statement markers.
    pub fn new_debug() -> Self {
        Self {
            debug: true,
            ..Self::new()
        }
    }

    /// The bytecode for `cmds`, or the reason there is none.
    ///
    /// Compiling FAILS rather than panics when the chunk cannot hold the
    /// document: fusevm's own guidance for a frontend compiling input it did
    /// not write is to see the full pool and report it, and a `\message`
    /// naming the limit is a better answer than an abort inside the VM.
    pub fn compile(mut self, cmds: &[Cmd]) -> Result<Chunk, TexError> {
        // Every count register starts at zero, as INITEX leaves them; the slots
        // have to be written before they are read or a read finds `Undef`.
        for reg in 0..COUNT_SLOTS {
            self.b.emit(Op::LoadInt(0), self.line);
            self.b.emit(Op::SetSlot(reg), self.line);
        }
        self.block(cmds)?;
        Ok(self.b.build())
    }

    /// The pool index for `s`, added once however often it is emitted.
    fn str_const(&mut self, s: &str) -> Result<u16, TexError> {
        if let Some(&k) = self.strings.get(s) {
            return Ok(k);
        }
        let Some(k) = self.b.try_add_constant(Value::Str(s.to_string().into())) else {
            return Err(TexError(format!(
                "This document has more than {} distinct strings, which is \
                 every constant a chunk can address",
                ChunkBuilder::MAX_POOL
            )));
        };
        self.strings.insert(s.to_string(), k);
        Ok(k)
    }

    fn block(&mut self, cmds: &[Cmd]) -> Result<(), TexError> {
        for c in cmds {
            self.cmd(c)?;
        }
        Ok(())
    }

    fn cmd(&mut self, c: &Cmd) -> Result<(), TexError> {
        match c {
            // A line directive: no code of its own, just the stamp every
            // following op carries -- unless this is a debug build, where it is
            // also where the debugger is allowed to stop.
            Cmd::Line(n) => {
                self.line = *n;
                if self.debug {
                    self.b.emit(Op::CallBuiltin(ops::DBG_LINE, 0), self.line);
                    self.b.emit(Op::Pop, self.line);
                }
            }
            Cmd::RustCompile(b64) => {
                let k = self.str_const(b64)?;
                self.b.emit(Op::LoadConst(k), self.line);
                self.b.emit(Op::CallBuiltin(ops::FFI_COMPILE, 1), self.line);
                self.b.emit(Op::Pop, self.line);
            }
            Cmd::SetCount(reg, n) => {
                self.num(n)?;
                self.b.emit(Op::SetSlot(slot(*reg)), self.line);
            }
            Cmd::Arith(op, reg, n) => {
                self.b.emit(Op::GetSlot(slot(*reg)), self.line);
                self.num(n)?;
                let native = match op {
                    Arith::Add => Op::Add,
                    Arith::Mul => Op::Mul,
                    Arith::Div => Op::Div,
                };
                self.b.emit(native, self.line);
                // TeX's `\divide` is INTEGER division truncating toward zero
                // (tex.web §1236); fusevm's `Div` is the numeric one and yields a
                // float, which printed `count=Float(18.0)` where tex prints 18.
                if matches!(op, Arith::Div) {
                    self.b.emit(Op::TruncInt, self.line);
                }
                self.b.emit(Op::SetSlot(slot(*reg)), self.line);
            }
            Cmd::Text(t) => {
                let k = self.str_const(t)?;
                self.b.emit(Op::LoadConst(k), self.line);
                self.b.emit(Op::CallBuiltin(ops::TEXT, 1), self.line);
                self.b.emit(Op::Pop, self.line);
            }
            Cmd::Message(msg) => {
                self.msg_ops(msg)?;
                self.b.emit(Op::CallBuiltin(ops::MSG_FLUSH, 0), self.line);
                self.b.emit(Op::Pop, self.line);
            }
            Cmd::Group { saves, body } => {
                // The saved values sit on the VM stack under the group's own
                // work, which stays balanced, and are written back in reverse.
                for reg in saves {
                    self.b.emit(Op::GetSlot(slot(*reg)), self.line);
                }
                self.block(body)?;
                for reg in saves.iter().rev() {
                    self.b.emit(Op::SetSlot(slot(*reg)), self.line);
                }
            }
            Cmd::IfNum {
                left,
                rel,
                right,
                then_branch,
                else_branch,
            } => {
                self.num(left)?;
                self.num(right)?;
                self.b.emit(
                    match rel {
                        Rel::Less => Op::NumLt,
                        Rel::Equal => Op::NumEq,
                        Rel::Greater => Op::NumGt,
                    },
                    self.line,
                );
                self.branch(then_branch, else_branch)?;
            }
            Cmd::Loop {
                body,
                left,
                rel,
                right,
            } => {
                // do-while: the body runs, then the test decides whether to go
                // round again. `patch_jump` takes an absolute target, so the
                // back edge is just the position the body started at.
                let start = self.b.current_pos();
                self.block(body)?;
                self.num(left)?;
                self.num(right)?;
                self.b.emit(
                    match rel {
                        Rel::Less => Op::NumLt,
                        Rel::Equal => Op::NumEq,
                        Rel::Greater => Op::NumGt,
                    },
                    self.line,
                );
                let back = self.b.emit(Op::JumpIfTrue(0), self.line);
                self.b.patch_jump(back, start);
            }
            Cmd::IfOdd {
                value,
                then_branch,
                else_branch,
            } => {
                // `\ifodd n` is `n mod 2 = 1`, and a negative odd number is odd
                // too -- so compare against zero rather than against one.
                self.num(value)?;
                self.b.emit(Op::LoadInt(2), self.line);
                self.b.emit(Op::Mod, self.line);
                self.b.emit(Op::LoadInt(0), self.line);
                self.b.emit(Op::NumEq, self.line);
                self.b.emit(Op::LogNot, self.line);
                self.branch(then_branch, else_branch)?;
            }
        }
        Ok(())
    }

    /// Emit the steps that build a message, appending each piece as it goes.
    fn msg_ops(&mut self, msg: &[MsgOp]) -> Result<(), TexError> {
        for m in msg {
            match m {
                MsgOp::Text(t) => {
                    let k = self.str_const(t)?;
                    self.b.emit(Op::LoadConst(k), self.line);
                    self.b.emit(Op::CallBuiltin(ops::MSG_APPEND, 1), self.line);
                    self.b.emit(Op::Pop, self.line);
                }
                MsgOp::Discard(n) => {
                    self.num(n)?;
                    self.b.emit(Op::Pop, self.line);
                }
                MsgOp::Number(n) => {
                    self.num(n)?;
                    self.b.emit(Op::CallBuiltin(ops::MSG_APPEND, 1), self.line);
                    self.b.emit(Op::Pop, self.line);
                }
                MsgOp::If {
                    left,
                    rel,
                    right,
                    then_ops,
                    else_ops,
                } => {
                    self.num(left)?;
                    self.num(right)?;
                    self.b.emit(
                        match rel {
                            Rel::Less => Op::NumLt,
                            Rel::Equal => Op::NumEq,
                            Rel::Greater => Op::NumGt,
                        },
                        self.line,
                    );
                    self.msg_branch(then_ops, else_ops)?;
                }
                MsgOp::IfOdd {
                    value,
                    then_ops,
                    else_ops,
                } => {
                    self.num(value)?;
                    self.b.emit(Op::LoadInt(2), self.line);
                    self.b.emit(Op::Mod, self.line);
                    self.b.emit(Op::LoadInt(0), self.line);
                    self.b.emit(Op::NumEq, self.line);
                    self.b.emit(Op::LogNot, self.line);
                    self.msg_branch(then_ops, else_ops)?;
                }
            }
        }
        Ok(())
    }

    fn msg_branch(&mut self, then_ops: &[MsgOp], else_ops: &[MsgOp]) -> Result<(), TexError> {
        let to_else = self.b.emit(Op::JumpIfFalse(0), self.line);
        self.msg_ops(then_ops)?;
        let over = self.b.emit(Op::Jump(0), self.line);
        let at = self.b.current_pos();
        self.b.patch_jump(to_else, at);
        self.msg_ops(else_ops)?;
        let end = self.b.current_pos();
        self.b.patch_jump(over, end);
        Ok(())
    }

    /// The condition is on the stack; emit the two arms around it.
    fn branch(&mut self, then_branch: &[Cmd], else_branch: &[Cmd]) -> Result<(), TexError> {
        let to_else = self.b.emit(Op::JumpIfFalse(0), self.line);
        self.block(then_branch)?;
        let over_else = self.b.emit(Op::Jump(0), self.line);
        let else_at = self.b.current_pos();
        self.b.patch_jump(to_else, else_at);
        self.block(else_branch)?;
        let end = self.b.current_pos();
        self.b.patch_jump(over_else, end);
        Ok(())
    }

    fn num(&mut self, n: &Num) -> Result<(), TexError> {
        match n {
            Num::Literal(v) => {
                self.b.emit(Op::LoadInt(*v), self.line);
            }
            Num::Count(reg) => {
                self.b.emit(Op::GetSlot(slot(*reg)), self.line);
            }
            Num::Rust { name, args } => {
                // The name first, then the arguments: the builtin pops the
                // whole run and the name is what it dispatches on.
                let k = self.str_const(name)?;
                self.b.emit(Op::LoadConst(k), self.line);
                for a in args {
                    self.num(a)?;
                }
                let argc = u8::try_from(args.len() + 1).unwrap_or(u8::MAX);
                self.b.emit(Op::CallBuiltin(ops::FFI_CALL, argc), self.line);
            }
        }
        Ok(())
    }
}

/// A register number as a slot, clamped to the 256 TeX provides.
fn slot(reg: i64) -> u16 {
    u16::try_from(reg).unwrap_or(0).min(COUNT_SLOTS - 1)
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}
