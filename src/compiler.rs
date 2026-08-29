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

use crate::ir::{Arith, Cmd, MsgOp, Num, Rel};
use fusevm::{Chunk, ChunkBuilder, Op, Value};

/// Builtin ids this frontend registers on the VM.
pub mod ops {
    /// Append one rendered piece to the message being built.
    pub const MSG_APPEND: u16 = 4000;
    /// Finish the message being built and record it.
    pub const MSG_FLUSH: u16 = 4001;
}

/// TeX has exactly 256 count registers (`tex.web` §236).
pub const COUNT_SLOTS: u16 = 256;

pub struct Compiler {
    b: ChunkBuilder,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            b: ChunkBuilder::new(),
        }
    }

    pub fn compile(mut self, cmds: &[Cmd]) -> Chunk {
        // Every count register starts at zero, as INITEX leaves them; the slots
        // have to be written before they are read or a read finds `Undef`.
        for reg in 0..COUNT_SLOTS {
            self.b.emit(Op::LoadInt(0), 0);
            self.b.emit(Op::SetSlot(reg), 0);
        }
        self.block(cmds);
        self.b.build()
    }

    fn block(&mut self, cmds: &[Cmd]) {
        for c in cmds {
            self.cmd(c);
        }
    }

    fn cmd(&mut self, c: &Cmd) {
        match c {
            Cmd::SetCount(reg, n) => {
                self.num(n);
                self.b.emit(Op::SetSlot(slot(*reg)), 0);
            }
            Cmd::Arith(op, reg, n) => {
                self.b.emit(Op::GetSlot(slot(*reg)), 0);
                self.num(n);
                let native = match op {
                    Arith::Add => Op::Add,
                    Arith::Mul => Op::Mul,
                    Arith::Div => Op::Div,
                };
                self.b.emit(native, 0);
                // TeX's `\divide` is INTEGER division truncating toward zero
                // (tex.web §1236); fusevm's `Div` is the numeric one and yields a
                // float, which printed `count=Float(18.0)` where tex prints 18.
                if matches!(op, Arith::Div) {
                    self.b.emit(Op::TruncInt, 0);
                }
                self.b.emit(Op::SetSlot(slot(*reg)), 0);
            }
            Cmd::Message(msg) => {
                self.msg_ops(msg);
                self.b.emit(Op::CallBuiltin(ops::MSG_FLUSH, 0), 0);
                self.b.emit(Op::Pop, 0);
            }
            Cmd::Group { saves, body } => {
                // The saved values sit on the VM stack under the group's own
                // work, which stays balanced, and are written back in reverse.
                for reg in saves {
                    self.b.emit(Op::GetSlot(slot(*reg)), 0);
                }
                self.block(body);
                for reg in saves.iter().rev() {
                    self.b.emit(Op::SetSlot(slot(*reg)), 0);
                }
            }
            Cmd::IfNum {
                left,
                rel,
                right,
                then_branch,
                else_branch,
            } => {
                self.num(left);
                self.num(right);
                self.b.emit(
                    match rel {
                        Rel::Less => Op::NumLt,
                        Rel::Equal => Op::NumEq,
                        Rel::Greater => Op::NumGt,
                    },
                    0,
                );
                self.branch(then_branch, else_branch);
            }
            Cmd::IfOdd {
                value,
                then_branch,
                else_branch,
            } => {
                // `\ifodd n` is `n mod 2 = 1`, and a negative odd number is odd
                // too -- so compare against zero rather than against one.
                self.num(value);
                self.b.emit(Op::LoadInt(2), 0);
                self.b.emit(Op::Mod, 0);
                self.b.emit(Op::LoadInt(0), 0);
                self.b.emit(Op::NumEq, 0);
                self.b.emit(Op::LogNot, 0);
                self.branch(then_branch, else_branch);
            }
        }
    }

    /// Emit the steps that build a message, appending each piece as it goes.
    fn msg_ops(&mut self, msg: &[MsgOp]) {
        for m in msg {
            match m {
                MsgOp::Text(t) => {
                    let k = self.b.add_constant(Value::Str(t.clone().into()));
                    self.b.emit(Op::LoadConst(k), 0);
                    self.b.emit(Op::CallBuiltin(ops::MSG_APPEND, 1), 0);
                    self.b.emit(Op::Pop, 0);
                }
                MsgOp::Number(n) => {
                    self.num(n);
                    self.b.emit(Op::CallBuiltin(ops::MSG_APPEND, 1), 0);
                    self.b.emit(Op::Pop, 0);
                }
                MsgOp::If {
                    left,
                    rel,
                    right,
                    then_ops,
                    else_ops,
                } => {
                    self.num(left);
                    self.num(right);
                    self.b.emit(
                        match rel {
                            Rel::Less => Op::NumLt,
                            Rel::Equal => Op::NumEq,
                            Rel::Greater => Op::NumGt,
                        },
                        0,
                    );
                    self.msg_branch(then_ops, else_ops);
                }
                MsgOp::IfOdd {
                    value,
                    then_ops,
                    else_ops,
                } => {
                    self.num(value);
                    self.b.emit(Op::LoadInt(2), 0);
                    self.b.emit(Op::Mod, 0);
                    self.b.emit(Op::LoadInt(0), 0);
                    self.b.emit(Op::NumEq, 0);
                    self.b.emit(Op::LogNot, 0);
                    self.msg_branch(then_ops, else_ops);
                }
            }
        }
    }

    fn msg_branch(&mut self, then_ops: &[MsgOp], else_ops: &[MsgOp]) {
        let to_else = self.b.emit(Op::JumpIfFalse(0), 0);
        self.msg_ops(then_ops);
        let over = self.b.emit(Op::Jump(0), 0);
        let at = self.b.current_pos();
        self.b.patch_jump(to_else, at);
        self.msg_ops(else_ops);
        let end = self.b.current_pos();
        self.b.patch_jump(over, end);
    }

    /// The condition is on the stack; emit the two arms around it.
    fn branch(&mut self, then_branch: &[Cmd], else_branch: &[Cmd]) {
        let to_else = self.b.emit(Op::JumpIfFalse(0), 0);
        self.block(then_branch);
        let over_else = self.b.emit(Op::Jump(0), 0);
        let else_at = self.b.current_pos();
        self.b.patch_jump(to_else, else_at);
        self.block(else_branch);
        let end = self.b.current_pos();
        self.b.patch_jump(over_else, end);
    }

    fn num(&mut self, n: &Num) {
        match n {
            Num::Literal(v) => {
                self.b.emit(Op::LoadInt(*v), 0);
            }
            Num::Count(reg) => {
                self.b.emit(Op::GetSlot(slot(*reg)), 0);
            }
        }
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
