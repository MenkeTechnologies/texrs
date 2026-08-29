//! The command stream a TeX file compiles to.
//!
//! TeX has no expression grammar to parse into a tree — the mouth and the
//! expander hand the stomach a flat run of primitive commands. That run IS this
//! frontend's AST, and it is what `crate::compiler` lowers to fusevm bytecode.
//!
//! Macro expansion happens BEFORE this, in the expander, exactly as a
//! conventional frontend runs its parser before lowering. What survives to here
//! is only what has to happen at run time: register writes, arithmetic,
//! branches, and output.

/// Where a number comes from at run time.
#[derive(Clone, Debug)]
pub enum Num {
    Literal(i64),
    /// `\count<n>` — a register read.
    Count(i64),
}

#[derive(Clone, Copy, Debug)]
pub enum Rel {
    Less,
    Equal,
    Greater,
}

#[derive(Clone, Copy, Debug)]
pub enum Arith {
    Add,
    Mul,
    Div,
}

/// One step of building a `\message`.
///
/// A message is assembled at RUN time, not folded to a string while lowering,
/// because a conditional inside one tests registers the VM holds. `\ifnum`
/// there becomes a real branch around the pieces it selects, exactly as it does
/// in running text.
#[derive(Clone, Debug)]
pub enum MsgOp {
    Text(String),
    Number(Num),
    If {
        left: Num,
        rel: Rel,
        right: Num,
        then_ops: Vec<MsgOp>,
        else_ops: Vec<MsgOp>,
    },
    IfOdd {
        value: Num,
        then_ops: Vec<MsgOp>,
        else_ops: Vec<MsgOp>,
    },
}

#[derive(Clone, Debug)]
pub enum Cmd {
    /// The source line the commands after this one came from.
    ///
    /// A line directive, the way any compiler carries one: the lowerer emits it
    /// when the line changes, and the code generator stamps it onto every op it
    /// emits afterwards. Without it every op in the chunk reports line 0, which
    /// makes a disassembly unreadable and a source-line debugger impossible.
    /// It generates no code.
    Line(u32),
    /// `\count<n>=<num>`
    SetCount(i64, Num),
    /// `\advance`/`\multiply`/`\divide` on a count register.
    Arith(Arith, i64, Num),
    /// `\message{...}` — built piece by piece at run time.
    Message(Vec<MsgOp>),
    /// A group: the listed count registers are saved on entry and restored on
    /// exit, which is what `{\count0=99}` needs and the macro table alone
    /// cannot give — a register lives in a VM slot, not in the frontend.
    Group { saves: Vec<i64>, body: Vec<Cmd> },
    /// `\ifnum<a><rel><b>` … `\else` … `\fi`
    IfNum {
        left: Num,
        rel: Rel,
        right: Num,
        then_branch: Vec<Cmd>,
        else_branch: Vec<Cmd>,
    },
    /// `\ifodd<num>` … `\else` … `\fi`
    IfOdd {
        value: Num,
        then_branch: Vec<Cmd>,
        else_branch: Vec<Cmd>,
    },
}
