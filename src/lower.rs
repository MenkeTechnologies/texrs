//! Turn a TeX source into the command stream `crate::compiler` lowers.
//!
//! This is the frontend half: the mouth tokenises, macros expand, `\catcode` and
//! `\def` take effect HERE at compile time — they change how the rest of the
//! file reads, so they cannot be deferred to run time — and everything whose
//! value is only known when the program runs becomes a `Cmd`.
//!
//! A conditional is the interesting case. A tree-walker decides the branch as it
//! goes; a compiler cannot, because the registers it tests are runtime state. So
//! both arms are COLLECTED as token runs, lowered recursively, and emitted as a
//! real branch. That is what makes `\ifnum` compile to `NumLt` + `JumpIfFalse`
//! instead of a Rust `if`.

use crate::catcode::Cat;
use crate::expand::{Engine, Macro, Meaning, TexError};
use crate::ir::{Arith, Cmd, MsgOp, Num, Rel};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};

type R<T> = Result<T, TexError>;

pub struct Lowerer {
    pub eng: Engine,
    /// Whether an `\end` in the source stopped the run.
    ///
    /// tex closes the file's paren differently depending on this: `\end` inside
    /// the file prints `(./doc.tex MSGS )` and stops reading, while a file that
    /// merely runs out prints `(./doc.tex MSGS)` and keeps reading — from the
    /// command line, if there is more there. `src/main.rs` needs to know which
    /// happened; nothing else does.
    pub ended: bool,
    /// Next hidden count register. `\edef` freezing `\the\count0` needs somewhere
    /// to put the value NOW, and a register is the only run-time store there is;
    /// TeX reserves the high registers for exactly this kind of scratch use.
    next_scratch: i64,
    /// How deep `block` is currently nested.
    ///
    /// Lowering inlines a macro into the stream and lowers through its body, and
    /// it lowers BOTH arms of a run-time conditional because neither is decided
    /// yet. A macro whose body names itself therefore inlines into its own arm
    /// without end -- `\def\r{\ifnum\count0<3 \r \fi}` never terminates while
    /// lowering, whichever way the test would go at run time, and the Rust stack
    /// runs out before anything is emitted. Real TeX never meets this because it
    /// interprets: `pass_text` skips the arm it did not take without expanding
    /// it. Until a recursive macro lowers to a run-time call rather than an
    /// inline copy, this bounds the nesting so the failure is TeX's own
    /// "capacity exceeded" rather than a segfault.
    depth: usize,
}

/// The nesting `block` refuses to go past.
///
/// Sits far above any real document -- a hand-written file nests groups and
/// conditionals a few dozen deep at the very most -- and below what the stack
/// can take. The ceiling is MEASURED, not guessed, and measured on a SPAWNED
/// thread rather than main: a spawned stack is a fraction of main's, and both
/// the test harness and any future worker pool run there. In a debug build on
/// such a thread 128 levels still lower and 192 abort, so the bound is set
/// under the smaller of those with room to spare. Matches the spirit of the
/// expander's 200_000-step ceiling: bound the runaway, name it as TeX names it.
const MAX_LOWER_DEPTH: usize = 100;

impl Lowerer {
    pub fn new() -> Self {
        Self {
            eng: Engine::new(),
            ended: false,
            next_scratch: 255,
            depth: 0,
        }
    }

    /// Where the hidden scratch registers have got to.
    ///
    /// A format (`crate::format`) captures the engine after a preamble and
    /// applies it to a later run, and the scratch counter has to travel with
    /// it: a body that started again from 255 would write over a value the
    /// preamble's `\edef` had already frozen there.
    pub fn scratch_mark(&self) -> i64 {
        self.next_scratch
    }

    /// Resume the scratch counter where a captured format left it.
    pub fn set_scratch_mark(&mut self, at: i64) {
        self.next_scratch = at;
    }

    /// Compile a whole source to a command stream.
    pub fn lower(&mut self, src: &str) -> R<Vec<Cmd>> {
        self.lower_located(src).map_err(|(e, _line)| e)
    }

    /// Run the LaTeX prelude through this lowerer, keeping its definitions and
    /// discarding whatever commands it emitted.
    ///
    /// The prelude is all definitions, so there is nothing to keep: what
    /// matters is the macro table it leaves behind on `self.eng`, which the
    /// document is then lowered against.
    pub fn preload(&mut self, src: &str) -> R<()> {
        self.lower_located(src).map(|_| ()).map_err(|(e, _)| e)
    }

    /// The same, reporting the line the mouth had reached when it stopped.
    ///
    /// A `TexError` carries a reason and no position, which is all a terminal
    /// message needs (`! Undefined control sequence.`) and not enough for an
    /// editor: a diagnostic has to land on a line. The lexer knows where it is,
    /// so the position is taken from it at the point the error escapes rather
    /// than threaded through every `?` in the expander.
    pub fn lower_located(&mut self, src: &str) -> Result<Vec<Cmd>, (TexError, u32)> {
        let mut lx = Lexer::new(src);
        match self.block(&mut lx, None) {
            Ok(cmds) => Ok(cmds),
            Err(e) => Err((e, lx.line())),
        }
    }

    /// Lower commands until the input ends, `\end` is seen, or one of `stop` is
    /// reached at nesting depth zero (used for a conditional's arms).
    /// Drop a line directive no command follows.
    ///
    /// A line whose tokens all vanish at compile time — a `\def`, a `\catcode`,
    /// a comment, `\end` — leaves no run-time work behind. A marker there is not
    /// merely useless: `--dap` verifies a breakpoint against the marker set, so
    /// one would let a client set a breakpoint on a line that can never be
    /// reached and report it verified.
    fn drop_empty_line_directives(cmds: Vec<Cmd>) -> Vec<Cmd> {
        let mut out: Vec<Cmd> = Vec::with_capacity(cmds.len());
        for cmd in cmds {
            if matches!(cmd, Cmd::Line(_)) && matches!(out.last(), Some(Cmd::Line(_))) {
                out.pop();
            }
            out.push(cmd);
        }
        if matches!(out.last(), Some(Cmd::Line(_))) {
            out.pop();
        }
        out
    }

    fn block(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        self.depth += 1;
        if self.depth > MAX_LOWER_DEPTH {
            self.depth -= 1;
            return Err(TexError(
                "TeX capacity exceeded, sorry [input stack size=100]".into(),
            ));
        }
        let out = self.block_inner(lx, stop);
        self.depth -= 1;
        out
    }

    fn block_inner(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        let mut out = Vec::new();
        // The line the last directive named, so one is emitted per line rather
        // than per command: a `\count` assignment and the `\message` beside it
        // share a line and need only one.
        let mut marked = 0u32;
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            let line = lx.line();
            if line != marked {
                out.push(Cmd::Line(line));
                marked = line;
            }
            let Token::Cs(name) = &tok else {
                // Braces group the macro table while lowering, so a `\def`
                // inside them is undone at the `}` exactly as TeX undoes it.
                match &tok {
                    Token::Char(_, Cat::BeginGroup) => {
                        // A group scopes the macro table AND the registers it
                        // writes; the latter is run-time state, so the body is
                        // lowered and wrapped in save/restore.
                        self.eng.compile_time_begin_group();
                        let body = self.block(lx, Some(&["\u{0}endgroup"]))?;
                        self.eng.compile_time_end_group()?;
                        let saves = assigned_counts(&body);
                        out.push(Cmd::Group { saves, body });
                    }
                    Token::Char(_, Cat::EndGroup) => {
                        return Ok(Self::drop_empty_line_directives(out))
                    }
                    _ => {}
                }
                continue;
            };
            let name = *name;
            if let Some(stops) = stop {
                if stops.contains(&name.name()) {
                    lx.push_back(&[Token::Cs(name)]);
                    return Ok(Self::drop_empty_line_directives(out));
                }
            }
            match name.name() {
                "end" => {
                    self.ended = true;
                    break;
                }
                // Compile-time: these change how the REST of the file reads.
                "def" | "gdef" => self.eng.compile_time_def(lx, name.name())?,
                "catcode" => self.eng.compile_time_catcode(lx)?,
                "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_equals_file(lx)?;
                    let v = self.number(lx)?;
                    out.push(Cmd::SetCount(reg, v));
                }
                "advance" | "multiply" | "divide" => {
                    let op = match name.name() {
                        "advance" => Arith::Add,
                        "multiply" => Arith::Mul,
                        _ => Arith::Div,
                    };
                    let Some(Token::Cs(what)) = lx.next_token(&self.eng.cats) else {
                        return Err(TexError("You can't use this after \\advance".into()));
                    };
                    if what.name() != "count" {
                        return Err(TexError(format!("Unsupported register \\{}", what.name())));
                    }
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_by_file(lx)?;
                    let v = self.number(lx)?;
                    out.push(Cmd::Arith(op, reg, v));
                }
                "message" => {
                    let parts = self.message_parts(lx)?;
                    out.push(Cmd::Message(parts));
                }
                // `\rustcompile <base64>\endrust`, which is what a `\rust{ … }`
                // block desugared to before the mouth ever read the file.
                n if n == crate::rust_ffi::COMPILE_CS => {
                    let b64 = self.rust_blob(lx)?;
                    out.push(Cmd::RustCompile(b64));
                }
                // A bare `\rustcall` in running text: the value is discarded,
                // which is how a document calls a Rust function for its effect.
                n if n == crate::rust_ffi::CALL_CS => {
                    let call = self.rust_call(lx, false)?;
                    out.push(Cmd::Message(vec![]));
                    // The call has to reach the VM, and a message with no
                    // pieces emits nothing but the flush -- so put the call in
                    // it and drop the rendered text by flushing an empty build.
                    if let Some(Cmd::Message(parts)) = out.last_mut() {
                        parts.push(MsgOp::Discard(call));
                    }
                }
                "ifnum" => {
                    let left = self.number(lx)?;
                    let rel = match self.eng.read_relation_file(lx)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.number(lx)?;
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfNum {
                        left,
                        rel,
                        right,
                        then_branch,
                        else_branch,
                    });
                }
                "ifodd" => {
                    let value = self.number(lx)?;
                    let (then_branch, else_branch) = self.arms(lx)?;
                    out.push(Cmd::IfOdd {
                        value,
                        then_branch,
                        else_branch,
                    });
                }
                // Decidable while lowering: the truth depends on the macro
                // table, which is a frontend fact. Emitting a branch for it
                // would be dishonest bytecode -- there is nothing to test.
                "iftrue" | "iffalse" => {
                    let taken = name.name() == "iftrue";
                    let (t, e) = self.arms(lx)?;
                    out.extend(if taken { t } else { e });
                }
                "ifx" => {
                    let same = self.eng.ifx_equal(lx)?;
                    let (t, e) = self.arms(lx)?;
                    out.extend(if same { t } else { e });
                }
                "let" => self.eng.compile_time_let(lx)?,
                "newcommand" | "renewcommand" | "providecommand" | "DeclareRobustCommand" => {
                    self.eng.compile_time_newcommand(lx, name.name())?
                }
                // Preamble directives naming files texrs cannot load. Their
                // arguments are consumed so the body of the document is still
                // read; see compile_time_preamble_directive.
                "documentclass" | "usepackage" | "RequirePackage" => {
                    self.eng.compile_time_preamble_directive(lx, 1)?
                }
                "PassOptionsToPackage" | "PassOptionsToClass" => {
                    self.eng.compile_time_preamble_directive(lx, 2)?
                }
                "makeatletter" => self.eng.compile_time_set_at_letter(true),
                "makeatother" => self.eng.compile_time_set_at_letter(false),
                "futurelet" => self.eng.compile_time_futurelet(lx)?,
                // Advice registration is a compile-time act: it changes what
                // the macros after it expand to.
                "intercept" => self.eng.compile_time_intercept(lx)?,
                "edef" | "xdef" => {
                    if let Some(cmd) = self.edef_snapshot(lx)? {
                        out.push(cmd);
                    }
                }
                "begingroup" => self.eng.compile_time_begin_group(),
                "endgroup" => self.eng.compile_time_end_group()?,
                "global" => self.eng.set_global_prefix(true),
                "relax" | "par" => {}
                other => {
                    // TeX's loop idiom -- a macro whose last act is to call
                    // itself under a test -- becomes a real loop rather than an
                    // inline copy. Inlining it cannot terminate: the copy holds
                    // the call that gets copied.
                    if let Some(parts) = self.tail_loop(name) {
                        let cmd = self.lower_tail_loop(parts)?;
                        out.push(cmd);
                        continue;
                    }
                    // A macro expands into the stream and lowering continues
                    // through its body -- expansion is a frontend concern.
                    if matches!(self.eng.meanings.get(&name), Some(Meaning::Macro(_))) {
                        self.eng.expand_macro_file(lx, name)?;
                        continue;
                    }
                    // Expandable primitives reach the top level too: TeX's
                    // expander handles `\expandafter`, `\csname` and friends
                    // wherever they occur, not only inside a macro body. Whatever
                    // they leave behind goes back through this loop.
                    if self.eng.expand_in_text(lx, name)? {
                        continue;
                    }
                    // `\let\g=\message` makes `\g` MEAN the primitive, and a
                    // primitive is dispatched by name here, so the alias has to
                    // resolve to the name before the match runs or `\g` reads as
                    // undefined while `\message` works.
                    if let Some(Meaning::Primitive(p)) = self.eng.meanings.get(&name) {
                        let p = *p;
                        if p != name {
                            lx.push_back(&[Token::Cs(p)]);
                            continue;
                        }
                    }
                    return Err(TexError(format!("Undefined control sequence \\{other}")));
                }
            }
        }
        Ok(Self::drop_empty_line_directives(out))
    }

    /// `\edef\x{...\the\count<n>...}` freezes the register's CURRENT value.
    ///
    /// The value lives in a VM slot, so "now" is run time. The snapshot is
    /// written to a scratch register at this point in the program and the macro
    /// is defined to read THAT, which is what makes a later use see the frozen
    /// value rather than the live one.
    fn edef_snapshot(&mut self, lx: &mut Lexer) -> R<Option<Cmd>> {
        let Some(Token::Cs(name)) = lx.next_token(&self.eng.cats) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        // Parameter text is not supported for the snapshot form.
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Runaway definition".into()));
            };
            if matches!(t, Token::Char(_, Cat::BeginGroup)) {
                break;
            }
        }
        let body = self.eng.read_balanced_pub(lx)?;
        // Find `\the\count<n>` in the body; anything else stays literal.
        let mut work = Lexer::new("");
        work.push_back(&body);
        let mut new_body: Vec<Token> = Vec::new();
        let mut cmd = None;
        while let Some(t) = work.pending.pop() {
            match &t {
                Token::Cs(n) if n.name() == "the" => {
                    match work.pending.pop() {
                        Some(Token::Cs(w)) if w.name() == "count" => {}
                        _ => return Err(TexError("Unsupported \\edef body".into())),
                    }
                    let reg = self.eng.scan_number_pending(&mut work)?;
                    let scratch = self.next_scratch;
                    self.next_scratch -= 1;
                    cmd = Some(Cmd::SetCount(scratch, Num::Count(reg)));
                    new_body.push(Token::cs("the"));
                    new_body.push(Token::cs("count"));
                    for ch in scratch.to_string().chars() {
                        new_body.push(Token::Char(ch, Cat::Other));
                    }
                }
                other => new_body.push(*other),
            }
        }
        self.eng.define_macro(name, new_body);
        Ok(cmd)
    }

    /// The `\else` and `\fi` arms of a conditional, each lowered.
    /// Recognise `\def\r{BODY \ifnum A<B \r \fi}` -- TeX's loop.
    ///
    /// Returns the body tokens and the condition tokens when `name` has exactly
    /// that shape, and `None` otherwise. Deliberately narrow: the tail call must
    /// be the last thing before the closing `\fi`, the macro must take no
    /// arguments, and it must not name itself anywhere else. Anything less
    /// certain is left to inlining, where the depth bound still catches it --
    /// a recogniser that guesses would silently compile a DIFFERENT program.
    fn tail_loop(&self, name: CsId) -> Option<TailLoop> {
        let Some(Meaning::Macro(m)) = self.eng.meanings.get(&name) else {
            return None;
        };
        if !m.params.is_empty() {
            return None;
        }
        let is_self = |t: &Token| matches!(t, Token::Cs(n) if *n == name);
        // The tail call sits at the end, before an optional space and `\fi`.
        let mut end = m.body.len();
        while end > 0 && m.body[end - 1].is_space() {
            end -= 1;
        }
        if end == 0 || !matches!(&m.body[end - 1], Token::Cs(n) if n.name() == "fi") {
            return None;
        }
        end -= 1;
        while end > 0 && m.body[end - 1].is_space() {
            end -= 1;
        }
        if end == 0 || !is_self(&m.body[end - 1]) {
            return None;
        }
        end -= 1;
        // Everything before it splits at the `\ifnum` that guards the call.
        let guard = m.body[..end]
            .iter()
            .rposition(|t| matches!(t, Token::Cs(n) if n.name() == "ifnum"))?;
        let body = m.body[..guard].to_vec();
        let cond = m.body[guard + 1..end].to_vec();
        // One self-reference only: another one elsewhere is a different program
        // than a loop, and inlining it is the honest answer.
        if body.iter().chain(cond.iter()).any(is_self) {
            return None;
        }
        Some(TailLoop { body, cond })
    }

    /// Lower a recognised tail loop: the body as a block, the guard as a test.
    fn lower_tail_loop(&mut self, parts: TailLoop) -> R<Cmd> {
        let mut body_lx = Lexer::new("");
        body_lx.push_back(&parts.body);
        let body = self.block(&mut body_lx, None)?;

        let mut cond_lx = Lexer::new("");
        cond_lx.push_back(&parts.cond);
        let left = self.number(&mut cond_lx)?;
        let rel = match self.eng.read_relation_file(&mut cond_lx)? {
            '<' => Rel::Less,
            '>' => Rel::Greater,
            _ => Rel::Equal,
        };
        let right = self.number(&mut cond_lx)?;
        Ok(Cmd::Loop {
            body,
            left,
            rel,
            right,
        })
    }

    fn arms(&mut self, lx: &mut Lexer) -> R<(Vec<Cmd>, Vec<Cmd>)> {
        let then_branch = self.block(lx, Some(&["else", "fi"]))?;
        let mut else_branch = Vec::new();
        match lx.next_token(&self.eng.cats) {
            Some(Token::Cs(n)) if n.name() == "else" => {
                else_branch = self.block(lx, Some(&["fi"]))?;
                // Consume the `\fi`.
                let _ = lx.next_token(&self.eng.cats);
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            other => {
                if let Some(t) = other {
                    lx.push_back(&[t]);
                }
                return Err(TexError("Incomplete \\ifnum; missing \\fi".into()));
            }
        }
        Ok((then_branch, else_branch))
    }

    /// The base64 body of a `\rustcompile <base64>\endrust`.
    ///
    /// Every character up to the terminating control sequence, spaces skipped:
    /// base64's alphabet is letters, digits, `+`, `/` and `=`, none of which the
    /// mouth can turn into anything but a character token whatever the catcodes
    /// are.
    fn rust_blob(&mut self, lx: &mut Lexer) -> R<String> {
        let mut b64 = String::new();
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Runaway \\rustcompile: missing \\endrust".into()));
            };
            match &t {
                _ if t.is_space() => continue,
                Token::Char(c, _) => b64.push(*c),
                Token::Cs(n) if n.name() == crate::rust_ffi::END_CS => break,
                Token::Cs(n) => {
                    return Err(TexError(format!(
                        "Unexpected \\{} inside a \\rust block body",
                        n.name()
                    )))
                }
            }
        }
        Ok(b64)
    }

    /// One token, from the file or from the pending list.
    ///
    /// The lowerer reads from two places -- running text (`\rustcall` in a
    /// statement) and an already-expanded token run (`\rustcall` inside a
    /// `\message` body) -- and the FFI call parser is the same either way, so
    /// the difference is a flag rather than two copies of the parser.
    fn take_token(&mut self, lx: &mut Lexer, pending: bool) -> Option<Token> {
        match pending {
            true => lx.pending.pop(),
            false => lx.next_token(&self.eng.cats),
        }
    }

    /// `\rustcall <name> <numbers…>\endrust`.
    ///
    /// The name is characters up to the first space, and the arguments are
    /// numbers up to `\endrust`. Both ends are catcode-independent: a control
    /// sequence terminates the list, and the only category the form depends on
    /// is the escape character.
    fn rust_call(&mut self, lx: &mut Lexer, pending: bool) -> R<Num> {
        let mut name = String::new();
        loop {
            let Some(t) = self.take_token(lx, pending) else {
                return Err(TexError("Runaway \\rustcall: no name".into()));
            };
            match &t {
                _ if t.is_space() && name.is_empty() => continue,
                _ if t.is_space() => break,
                Token::Char(c, _) => name.push(*c),
                // A control sequence ends the name; it is the argument list, or
                // the terminator for a call that takes none.
                Token::Cs(_) => {
                    lx.push_back(&[t]);
                    break;
                }
            }
        }
        if name.is_empty() {
            return Err(TexError("Missing function name after \\rustcall".into()));
        }

        let mut args = Vec::new();
        loop {
            let Some(t) = self.take_token(lx, pending) else {
                return Err(TexError(format!(
                    "Runaway \\rustcall {name}: missing \\endrust"
                )));
            };
            if t.is_space() {
                continue;
            }
            if let Token::Cs(n) = &t {
                if n.name() == crate::rust_ffi::END_CS {
                    break;
                }
            }
            lx.push_back(&[t]);
            args.push(match pending {
                true => self.msg_number(lx)?,
                false => self.number(lx)?,
            });
        }
        Ok(Num::Rust { name, args })
    }

    /// A number operand: a literal, `\count<n>` read at run time, or a call into
    /// a compiled `\rust{ … }` block.
    fn number(&mut self, lx: &mut Lexer) -> R<Num> {
        // Peek for `\count`, which becomes a slot read rather than a constant.
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    return Ok(Num::Count(reg));
                }
                Token::Cs(n) if n.name() == crate::rust_ffi::CALL_CS => {
                    return self.rust_call(lx, false);
                }
                _ => {
                    lx.push_back(&[t]);
                    return Ok(Num::Literal(self.eng.scan_number_file(lx)?));
                }
            }
        }
    }

    /// `\message{...}` lowered to the steps that build it at run time.
    ///
    /// The body is walked as a token list. Macros and `\csname` resolve here --
    /// they depend on the macro table, which is a frontend fact. `\the\count`
    /// and conditionals do not: they read VM slots, so they become a slot read
    /// and a real branch.
    fn message_parts(&mut self, lx: &mut Lexer) -> R<Vec<MsgOp>> {
        let body = self.eng.read_message_body(lx)?;
        let mut work = Lexer::new("");
        work.push_back(&body);
        self.msg_ops(&mut work, &[])
    }

    /// Walk a message token list into build steps, stopping at `stop`.
    fn msg_ops(&mut self, work: &mut Lexer, stop: &[&str]) -> R<Vec<MsgOp>> {
        let mut out: Vec<MsgOp> = Vec::new();
        let mut text = String::new();
        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    out.push(MsgOp::Text(std::mem::take(&mut text)));
                }
            };
        }
        while let Some(t) = work.pending.pop() {
            let Token::Cs(n) = &t else {
                text.push_str(&t.to_text(self.eng.escape));
                continue;
            };
            let n = *n;
            if stop.contains(&n.name()) {
                work.push_back(&[Token::Cs(n)]);
                break;
            }
            match n.name() {
                "the" | "number" => {
                    flush!();
                    if n.name() == "the" {
                        match work.pending.pop() {
                            Some(Token::Cs(w)) if w.name() == "count" => {}
                            _ => return Err(TexError("Unsupported \\the".into())),
                        }
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(reg)));
                        continue;
                    }
                    // `\number` takes either a register or a literal.
                    let is_reg =
                        matches!(work.pending.last(), Some(Token::Cs(w)) if w.name() == "count");
                    if is_reg {
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(work)?;
                        out.push(MsgOp::Number(Num::Count(reg)));
                    } else {
                        let v = self.eng.scan_number_pending(work)?;
                        text.push_str(&v.to_string());
                    }
                }
                // An advice marker: it carries depth, not text.
                n if self.eng.advice_marker(n) => {}
                // `\rustcall <name> <numbers…>\endrust` inside a message: the
                // returned value is rendered where the call stands.
                n if n == crate::rust_ffi::CALL_CS => {
                    flush!();
                    let call = self.rust_call(work, true)?;
                    out.push(MsgOp::Number(call));
                }
                "string" => {
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&match &next {
                            Token::Cs(cs) => format!("{}{}", self.eng.escape, cs.name()),
                            other => other.to_text(self.eng.escape),
                        });
                    }
                }
                "csname" => {
                    // The name is built from text and macros, all compile-time.
                    let built = self.eng.read_csname_pending(work)?;
                    work.push_back(&[Token::cs(&built)]);
                }
                "expandafter" => {
                    let Some(held) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    let Some(next) = work.pending.pop() else {
                        return Err(TexError("Missing token after \\expandafter".into()));
                    };
                    match &next {
                        Token::Cs(m) if self.eng.is_macro(*m) => {
                            let m = *m;
                            self.eng.expand_macro_pending(work, m)?;
                        }
                        _ => work.push_back(&[next]),
                    }
                    work.push_back(&[held]);
                }
                "iftrue" | "iffalse" => {
                    let taken = n.name() == "iftrue";
                    let (t_ops, e_ops) = self.msg_arms(work)?;
                    flush!();
                    out.extend(if taken { t_ops } else { e_ops });
                }
                "ifx" => {
                    let a = work.pending.pop();
                    let b = work.pending.pop();
                    let same = self.eng.meanings_equal_pub(a.as_ref(), b.as_ref());
                    let (t_ops, e_ops) = self.msg_arms(work)?;
                    flush!();
                    out.extend(if same { t_ops } else { e_ops });
                }
                "ifnum" => {
                    let left = self.msg_number(work)?;
                    let rel = match self.eng.read_relation_pending(work)? {
                        '<' => Rel::Less,
                        '>' => Rel::Greater,
                        _ => Rel::Equal,
                    };
                    let right = self.msg_number(work)?;
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::If {
                        left,
                        rel,
                        right,
                        then_ops,
                        else_ops,
                    });
                }
                "ifodd" => {
                    let value = self.msg_number(work)?;
                    let (then_ops, else_ops) = self.msg_arms(work)?;
                    flush!();
                    out.push(MsgOp::IfOdd {
                        value,
                        then_ops,
                        else_ops,
                    });
                }
                "ifcase" => {
                    let value = self.msg_number(work)?;
                    let branches = self.msg_case_arms(work)?;
                    flush!();
                    out.push(self.case_chain(value, branches));
                }
                _ if self.eng.is_macro(n) => self.eng.expand_macro_pending(work, n)?,
                _ => text.push_str(&t.to_text(self.eng.escape)),
            }
        }
        if !text.is_empty() {
            out.push(MsgOp::Text(text));
        }
        Ok(out)
    }

    /// `\ifcase` becomes a chain of equality branches -- one per `\or` arm, with
    /// the `\else` arm as the tail. The VM has no jump table op, and a chain is
    /// what `\ifcase` means anyway: the nth branch for the value n.
    fn case_chain(&mut self, value: Num, mut branches: Vec<Vec<MsgOp>>) -> MsgOp {
        let default = match branches.len() {
            0 => Vec::new(),
            _ => branches.pop().unwrap_or_default(),
        };
        let mut chain = default;
        for (i, arm) in branches.into_iter().enumerate().rev() {
            chain = vec![MsgOp::If {
                left: value.clone(),
                rel: Rel::Equal,
                right: Num::Literal(i as i64),
                then_ops: arm,
                else_ops: chain,
            }];
        }
        match chain.len() {
            1 => chain.pop().unwrap_or(MsgOp::Text(String::new())),
            _ => MsgOp::Text(String::new()),
        }
    }

    /// The `\or`-separated arms of an `\ifcase`, the last being `\else`'s.
    fn msg_case_arms(&mut self, work: &mut Lexer) -> R<Vec<Vec<MsgOp>>> {
        let mut arms = Vec::new();
        loop {
            let arm = self.msg_ops(work, &["or", "else", "fi"])?;
            arms.push(arm);
            match work.pending.pop() {
                Some(Token::Cs(n)) if n.name() == "or" => continue,
                Some(Token::Cs(n)) if n.name() == "else" => {
                    let default = self.msg_ops(work, &["fi"])?;
                    arms.push(default);
                    let _ = work.pending.pop();
                    return Ok(arms);
                }
                Some(Token::Cs(n)) if n.name() == "fi" => {
                    // No `\else`: the default arm is empty.
                    arms.push(Vec::new());
                    return Ok(arms);
                }
                _ => return Err(TexError("Incomplete \\ifcase".into())),
            }
        }
    }

    /// The two arms of a conditional inside a message.
    fn msg_arms(&mut self, work: &mut Lexer) -> R<(Vec<MsgOp>, Vec<MsgOp>)> {
        let then_ops = self.msg_ops(work, &["else", "fi"])?;
        let mut else_ops = Vec::new();
        match work.pending.pop() {
            Some(Token::Cs(n)) if n.name() == "else" => {
                else_ops = self.msg_ops(work, &["fi"])?;
                let _ = work.pending.pop();
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            _ => return Err(TexError("Incomplete \\if; missing \\fi".into())),
        }
        Ok((then_ops, else_ops))
    }

    /// A number operand inside a message body.
    fn msg_number(&mut self, work: &mut Lexer) -> R<Num> {
        loop {
            let Some(t) = work.pending.pop() else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            if t.is_space() {
                continue;
            }
            match &t {
                Token::Cs(n) if n.name() == "count" => {
                    let reg = self.eng.scan_number_pending(work)?;
                    return Ok(Num::Count(reg));
                }
                Token::Cs(n) if n.name() == crate::rust_ffi::CALL_CS => {
                    return self.rust_call(work, true);
                }
                _ => {
                    work.push_back(&[t]);
                    return Ok(Num::Literal(self.eng.scan_number_pending(work)?));
                }
            }
        }
    }
}

/// The two halves of a recognised tail loop: what runs, and what decides.
struct TailLoop {
    body: Vec<Token>,
    cond: Vec<Token>,
}

/// Every count register a command block assigns, so a group knows what to save.
fn assigned_counts(cmds: &[Cmd]) -> Vec<i64> {
    let mut regs = Vec::new();
    fn walk(cmds: &[Cmd], regs: &mut Vec<i64>) {
        for c in cmds {
            match c {
                // Neither a line directive nor a `\rust{ … }` compile writes a
                // register.
                Cmd::Line(_) | Cmd::RustCompile(_) => {}
                Cmd::SetCount(r, _) | Cmd::Arith(_, r, _) => {
                    if !regs.contains(r) {
                        regs.push(*r);
                    }
                }
                Cmd::IfNum {
                    then_branch,
                    else_branch,
                    ..
                }
                | Cmd::IfOdd {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    walk(then_branch, regs);
                    walk(else_branch, regs);
                }
                // A loop's body assigns exactly what its commands assign; a
                // group around one still has to save those registers.
                Cmd::Loop { body, .. } | Cmd::Group { body, .. } => walk(body, regs),
                Cmd::Message(_) => {}
            }
        }
    }
    walk(cmds, &mut regs);
    regs
}

impl Default for Lowerer {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-exported so `lower` can build macros without reaching into the engine.
pub type MacroDef = Macro;
/// Kept for the catcode table's benefit.
pub type CatKind = Cat;
