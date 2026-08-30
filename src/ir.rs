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
    /// `\rustcall <name> <args>\endrust` — a call into a compiled `\rust{ … }`
    /// block. It is a `Num` rather than a command of its own because that is
    /// where a value is useful: anywhere TeX reads a number, which is a register
    /// assignment, an arithmetic operand, a conditional, or a `\message` body.
    Rust {
        name: String,
        args: Vec<Num>,
    },
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
    /// A number computed and thrown away, for a call made for its effect —
    /// `\rustcall` in running text rather than inside a message body. It rides
    /// the message machinery because that is where a run-time value already has
    /// somewhere to be evaluated; nothing is appended.
    Discard(Num),
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
    /// `\rustcompile <base64>\endrust` — compile and register the functions a
    /// `\rust{ … }` block exported. It is a run-time command, not a compile-time
    /// one: compiling calls `rustc`, and doing that while LOWERING would mean a
    /// document could not be lowered on a machine that only has to run it.
    RustCompile(String),
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
    /// A run of the document's own text.
    ///
    /// Ordinary characters -- the words of the document, as opposed to what
    /// `\message` prints -- used to be dropped while lowering, because an
    /// engine with no stomach has nowhere to put them. That made a 15,000-line
    /// book compile to a program that printed nothing: it "ran" and produced 66
    /// bytes. This carries them instead, so `--text` can emit what the document
    /// says. It is not typesetting: no line breaking, no pages, no fonts. It is
    /// the text, in order.
    Text(String),
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
    /// A tail-recursive macro, as a run-time loop.
    ///
    /// `\def\r{BODY \ifnum A<B \r \fi}` is TeX's loop idiom: run the body,
    /// test, and invoke yourself again while the test holds. Inlining that is
    /// what used to run the lowerer out of stack, because the copy contains the
    /// call that gets copied. As a loop it is finite bytecode AND faster than a
    /// call would be -- a backward `Jump` with no frame to push, which is a
    /// shape the JIT already recognises.
    ///
    /// The test runs AFTER the body, matching the idiom: the body always
    /// executes once, exactly as the first `\r` does before reaching its own
    /// conditional.
    Loop {
        body: Vec<Cmd>,
        left: Num,
        rel: Rel,
        right: Num,
    },
    /// `\ifodd<num>` … `\else` … `\fi`
    IfOdd {
        value: Num,
        then_branch: Vec<Cmd>,
        else_branch: Vec<Cmd>,
    },
}

/// The command stream as an indented tree, for `--dump-ast`.
///
/// `{:#?}` would also print it, but as Rust: field names, `Some(...)`, a line
/// per brace. This prints the shape a reader of the DOCUMENT is looking for --
/// one node per line, children indented under the construct that owns them --
/// so a `\ifnum` shows its two branches and a tail-recursive macro shows the
/// loop it lowered to rather than the call it was written as.
pub fn render(cmds: &[Cmd]) -> String {
    let mut out = String::new();
    render_into(cmds, 0, &mut out);
    out
}

fn render_into(cmds: &[Cmd], depth: usize, out: &mut String) {
    for cmd in cmds {
        let pad = "  ".repeat(depth);
        match cmd {
            Cmd::RustCompile(_) => out.push_str(&format!("{pad}RustCompile <block>\n")),
            Cmd::Line(n) => out.push_str(&format!("{pad}Line {n}\n")),
            Cmd::SetCount(reg, num) => {
                out.push_str(&format!("{pad}SetCount \\count{reg} = {}\n", num_text(num)))
            }
            Cmd::Arith(op, reg, num) => {
                out.push_str(&format!("{pad}{op:?} \\count{reg} by {}\n", num_text(num)))
            }
            Cmd::Text(t) => out.push_str(&format!("{pad}Text {t:?}\n")),
            Cmd::Message(ops) => {
                out.push_str(&format!("{pad}Message\n"));
                render_msg(ops, depth + 1, out);
            }
            Cmd::Group { saves, body } => {
                out.push_str(&format!("{pad}Group saves={saves:?}\n"));
                render_into(body, depth + 1, out);
            }
            Cmd::IfNum {
                left,
                rel,
                right,
                then_branch,
                else_branch,
            } => {
                out.push_str(&format!(
                    "{pad}IfNum {} {} {}\n",
                    num_text(left),
                    rel_text(*rel),
                    num_text(right)
                ));
                render_branches(then_branch, else_branch, depth, out);
            }
            Cmd::IfOdd {
                value,
                then_branch,
                else_branch,
            } => {
                out.push_str(&format!("{pad}IfOdd {}\n", num_text(value)));
                render_branches(then_branch, else_branch, depth, out);
            }
            Cmd::Loop {
                body,
                left,
                rel,
                right,
            } => {
                // The test prints after the body because that is when it runs:
                // the body always executes once, as the first call does before
                // reaching its own conditional.
                out.push_str(&format!("{pad}Loop\n"));
                render_into(body, depth + 1, out);
                out.push_str(&format!(
                    "{pad}  while {} {} {}\n",
                    num_text(left),
                    rel_text(*rel),
                    num_text(right)
                ));
            }
        }
    }
}

/// The two arms of a conditional, each named, and an empty one said to be empty
/// rather than left out -- `\ifnum` with no `\else` and `\ifnum` whose `\else`
/// lowered to nothing are different documents.
fn render_branches(then_branch: &[Cmd], else_branch: &[Cmd], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth + 1);
    out.push_str(&format!("{pad}then\n"));
    render_into(then_branch, depth + 2, out);
    out.push_str(&format!("{pad}else\n"));
    render_into(else_branch, depth + 2, out);
}

fn render_msg(ops: &[MsgOp], depth: usize, out: &mut String) {
    for op in ops {
        let pad = "  ".repeat(depth);
        match op {
            MsgOp::Text(t) => out.push_str(&format!("{pad}Text {t:?}\n")),
            MsgOp::Number(n) => out.push_str(&format!("{pad}Number {}\n", num_text(n))),
            MsgOp::Discard(n) => out.push_str(&format!("{pad}Discard {}\n", num_text(n))),
            MsgOp::If {
                left,
                rel,
                right,
                then_ops,
                else_ops,
            } => {
                out.push_str(&format!(
                    "{pad}IfNum {} {} {}\n",
                    num_text(left),
                    rel_text(*rel),
                    num_text(right)
                ));
                out.push_str(&format!("{pad}  then\n"));
                render_msg(then_ops, depth + 2, out);
                out.push_str(&format!("{pad}  else\n"));
                render_msg(else_ops, depth + 2, out);
            }
            MsgOp::IfOdd {
                value,
                then_ops,
                else_ops,
            } => {
                out.push_str(&format!("{pad}IfOdd {}\n", num_text(value)));
                out.push_str(&format!("{pad}  then\n"));
                render_msg(then_ops, depth + 2, out);
                out.push_str(&format!("{pad}  else\n"));
                render_msg(else_ops, depth + 2, out);
            }
        }
    }
}

/// A number written the way the document wrote it: a literal as itself, a
/// register as `\count<n>`, a call as its name and arguments.
fn num_text(num: &Num) -> String {
    match num {
        Num::Literal(v) => v.to_string(),
        Num::Count(n) => format!("\\count{n}"),
        Num::Rust { name, args } => {
            let args: Vec<String> = args.iter().map(num_text).collect();
            format!("\\rustcall {name} {}", args.join(" "))
        }
    }
}

fn rel_text(rel: Rel) -> &'static str {
    match rel {
        Rel::Less => "<",
        Rel::Equal => "=",
        Rel::Greater => ">",
    }
}
