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

/// One piece of a `\message`: either text fixed at expansion time, or a number
/// that is only known when the program runs.
#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    Number(Num),
}

#[derive(Clone, Debug)]
pub enum Cmd {
    /// `\count<n>=<num>`
    SetCount(i64, Num),
    /// `\advance`/`\multiply`/`\divide` on a count register.
    Arith(Arith, i64, Num),
    /// `\message{...}`
    Message(Vec<Part>),
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
