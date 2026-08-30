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
    /// The typefaces the document asked for, by `\setmainfont` and its
    /// siblings.
    ///
    /// A document says `\setmainfont{Arimo}` and means it; setting the whole
    /// book in Computer Modern regardless is the complaint this exists to
    /// answer. Recorded while lowering because that is where the preamble is
    /// read, and handed to the typesetter, which decides what it can honour.
    pub fonts: crate::typeset::Families,
    /// Carry the document's own text into the program.
    ///
    /// Off by default: the terminal output of a `tex` run is its `\message`
    /// stream, and the differential suite compares against exactly that. `--text`
    /// turns it on for a caller who wants what the document SAYS rather than
    /// what it announced.
    text_output: bool,
    /// How many `\input` files are open above this one, so a file that inputs
    /// itself stops with a diagnostic instead of exhausting the host stack.
    input_depth: usize,
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
            input_depth: 0,
            eng: Engine::new(),
            ended: false,
            next_scratch: 255,
            fonts: crate::typeset::Families::default(),
            text_output: false,
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

    /// Emit the document's own text as well as its messages.
    pub fn with_text_output(mut self) -> Self {
        self.text_output = true;
        self
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
            // An ACTIVE character is a command, not text: it is looked up in
            // the same table a control sequence is. Rewriting it to that
            // control sequence here means everything below -- expansion, the
            // tail-loop recogniser, `\ifx` -- sees one kind of token and needs
            // no second case for it.
            let tok = match &tok {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(*c) {
                    Some(id) => Token::Cs(id),
                    None => tok,
                },
                _ => tok,
            };
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
                        // A group exists to save registers and to scope the
                        // macro table. The macro table is a compile-time fact
                        // and is already handled above, so a group that assigns
                        // no register has nothing left to do at run time --
                        // and keeping it breaks the text run either side of it.
                        // A document's braces are everywhere (every
                        // `\NormalTok{...}` is one), so each became its own
                        // constant and a 4 MB book exhausted the 65,536-entry
                        // pool. Flattening a group that only carries text keeps
                        // one constant per stretch.
                        let only_text = body
                            .iter()
                            .all(|c| matches!(c, Cmd::Text(_) | Cmd::Line(_)));
                        if saves.is_empty() && only_text {
                            for cmd in body {
                                match (&cmd, out.last_mut()) {
                                    (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                                    _ => out.push(cmd),
                                }
                            }
                        } else {
                            out.push(Cmd::Group { saves, body });
                        }
                    }
                    Token::Char(_, Cat::EndGroup) => {
                        return Ok(Self::drop_empty_line_directives(out))
                    }
                    // The document's own words. Dropping these is why a book
                    // used to compile to a program that printed nothing.
                    Token::Char(c, _) if self.text_output => {
                        // Append to the run in progress, looking PAST any line
                        // directives: they generate no code, but one is emitted
                        // per line, so treating them as breaks makes every line
                        // its own constant. A 4 MB book then exhausts fusevm's
                        // 65,536-entry constant pool -- which a u16 operand
                        // cannot address past -- and the compile panics.
                        // Coalescing keeps one constant per stretch of text
                        // between real commands.
                        let mut at = out.len();
                        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
                            at -= 1;
                        }
                        match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
                            Some(Cmd::Text(t)) => t.push(*c),
                            _ => out.push(Cmd::Text(c.to_string())),
                        }
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
            // A verbatim environment suspends the catcodes: everything up to
            // its \end is characters, not TeX. It has to be caught HERE,
            // before `\begin` expands, because expanding is exactly what must
            // not happen to the body -- a code listing is full of backslashes
            // that are not control sequences, and reading them as control
            // sequences is why a book of code samples could not be read.
            if name.name() == "begin" {
                if let Some(env) = self.peek_environment_name(lx) {
                    if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
                        // Consume the `{name}` that was only peeked at, or it
                        // lands in the output ahead of the body.
                        while let Some(t) = lx.next_token(&self.eng.cats) {
                            if matches!(t, Token::Char(_, Cat::EndGroup)) {
                                break;
                            }
                        }
                        let end = format!("\\end{{{env}}}");
                        let Some(body) = lx.read_raw_until(&end) else {
                            return Err(TexError(format!(
                                "Runaway argument: \\begin{{{env}}} never ends"
                            )));
                        };
                        if self.text_output {
                            out.push(Cmd::Text(body));
                        }
                        continue;
                    }
                }
            }
            // Colour, before the prelude's own \textcolor can swallow it. DVI
            // carries colour as a `\special` a driver reads, so it has to
            // survive lowering as structure rather than as text.
            if self.text_output
                && name.name() == "textcolor"
                && self.lower_textcolor(lx, &mut out)?
            {
                continue;
            }
            // A control sequence MEANS what it was last defined as. The
            // dispatch below is by NAME, so a document that redefines a
            // primitive was still getting the primitive. LaTeX redefines `\end`
            // to close an environment, so a LaTeX document stopped dead at its
            // first `\end{...}` -- which is why a whole book produced a page of
            // preamble text and nothing else.
            if matches!(self.eng.meanings.get(&name), Some(Meaning::Macro(_)))
                && self.meaning_wins(lx, name)
            {
                if let Some(parts) = self.tail_loop(name) {
                    out.push(self.lower_tail_loop(parts)?);
                    continue;
                }
                self.eng.expand_macro_file(lx, name)?;
                continue;
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
                // `\pageno=7`, where `\pageno` was `\countdef`'d: the name is
                // the register, so this is the `\count0=7` arm reached by
                // another spelling.
                _ if matches!(
                    self.eng.numeric_cs(name),
                    Some(crate::expand::NumericCs::Register(_))
                ) =>
                {
                    let Some(crate::expand::NumericCs::Register(reg)) = self.eng.numeric_cs(name)
                    else {
                        unreachable!("guarded by the arm's own pattern")
                    };
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
                    // `\advance\pageno by 1` names the register the same way
                    // `\advance\count0 by 1` does.
                    let reg = match self.eng.numeric_cs(what) {
                        Some(crate::expand::NumericCs::Register(r)) => r,
                        _ => {
                            if what.name() != "count" {
                                return Err(TexError(format!(
                                    "Unsupported register \\{}",
                                    what.name()
                                )));
                            }
                            self.eng.scan_number_file(lx)?
                        }
                    };
                    self.eng.skip_by_file(lx)?;
                    let v = self.number(lx)?;
                    out.push(Cmd::Arith(op, reg, v));
                }
                "message" => {
                    let parts = self.message_parts(lx)?;
                    out.push(Cmd::Message(parts));
                }
                // `\input FILE` reads another file HERE, sharing every piece of
                // state: a macro it defines is defined afterwards, a `\catcode`
                // it sets stays set. That is the whole point of it -- a real
                // document's first line loads a format or a package, and until
                // this existed no real document could run at all.
                "input" => {
                    let name = self.scan_file_name(lx)?;
                    let (shown, src) = self.open_input(&name)?;
                    out.push(Cmd::Message(vec![MsgOp::Text(format!("({shown}"))]));
                    self.input_depth += 1;
                    let inner = self.input_pass(&src);
                    self.input_depth -= 1;
                    out.extend(inner?);
                    out.push(Cmd::FileClose);
                    // `\end` inside the file stops the whole run, not just the
                    // file: tex closes every open paren and finishes.
                    if self.ended {
                        break;
                    }
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
                    let cmds = self.decided_arms(lx, taken)?;
                    out.extend(cmds);
                }
                "ifx" => {
                    let same = self.eng.ifx_equal(lx)?;
                    let cmds = self.decided_arms(lx, same)?;
                    out.extend(cmds);
                }
                "let" => self.eng.compile_time_let(lx)?,
                // Both define a control sequence that stands for a number, and
                // both are compile-time: what they define changes how the rest
                // of the file READS, exactly as `\def` does.
                "chardef" | "countdef" => self.eng.compile_time_numeric_def(lx, name.name())?,
                "newcommand" | "renewcommand" | "providecommand" | "DeclareRobustCommand" => {
                    self.eng.compile_time_newcommand(lx, name.name())?
                }
                // Preamble directives naming files texrs cannot load. Their
                // arguments are consumed so the body of the document is still
                // read; see compile_time_preamble_directive.
                // The font the document asked for. Its argument is consumed
                // either way; the difference is that the name is kept.
                k @ ("setmainfont" | "setsansfont" | "setmonofont" | "setromanfont") => {
                    let _ = self.eng.read_optional_bracket(lx)?;
                    let name = self.eng.read_group_text_pub(lx)?;
                    let _ = self.eng.read_optional_bracket(lx)?;
                    let slot = match k {
                        "setsansfont" => &mut self.fonts.sans,
                        "setmonofont" => &mut self.fonts.mono,
                        _ => &mut self.fonts.main,
                    };
                    *slot = Some(name.trim().to_string());
                }
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
                // The other two definition prefixes. Like `\global` they set a
                // flag the definition that follows reads and spends.
                "long" => self.eng.set_long_prefix(true),
                "outer" => self.eng.set_outer_prefix(true),
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
        // The parameter text, exactly as `\def` reads it. `\edef` differs from
        // `\def` only in WHEN the body is expanded; dropping the parameters
        // here left `\edef\pair#1,#2.{…}` matching nothing and its delimiters
        // landing in the output. Found by `parity-fuzz`.
        let mut params: Vec<Token> = Vec::new();
        loop {
            let Some(t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("Runaway definition".into()));
            };
            if matches!(t, Token::Char(_, Cat::BeginGroup)) {
                break;
            }
            params.push(t);
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
        self.eng.define_macro_with_params(name, params, new_body)?;
        Ok(cmd)
    }

    /// The `\else` and `\fi` arms of a conditional, each lowered.
    /// `\textcolor[model]{spec}{text}`, as markers INSIDE the text run.
    ///
    /// Not as a `Cmd::Color` wrapping a block, which is what this did first: a
    /// colour region splits the text either side of it into separate commands,
    /// each of which becomes its own string constant, and Pandoc's syntax
    /// highlighting emits thousands of `\textcolor` calls per book. That
    /// exhausted fusevm's 65,536-constant pool on five of the larger documents
    /// -- the same ceiling the braces hit before, reached a different way.
    ///
    /// Markers in the stream keep the text coalescing as it did, and the
    /// typesetter turns them into the DVI `\special` a driver reads. Only the
    /// `rgb` model is understood, which is what a Pandoc document writes;
    /// anything else falls through to the ordinary macro path rather than being
    /// coloured wrongly.
    fn lower_textcolor(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<bool> {
        let Some(model) = self.eng.read_optional_bracket(lx)? else {
            return Ok(false);
        };
        let model: String = model.iter().map(|t| t.to_text(self.eng.escape)).collect();
        if model.trim() != "rgb" {
            return Ok(false);
        }
        let spec = self.eng.read_group_text_pub(lx)?;
        let parts: Vec<f64> = spec
            .split(',')
            .filter_map(|p| p.trim().parse::<f64>().ok())
            .collect();
        if parts.len() != 3 {
            return Ok(false);
        }
        self.push_text(
            out,
            &format!("\u{1}{},{},{}\u{2}", parts[0], parts[1], parts[2]),
        );
        let raw = self.eng.read_balanced_group(lx)?;
        let mut inner = Lexer::new("");
        inner.push_back(&raw);
        for cmd in self.block(&mut inner, None)? {
            match (&cmd, out.last_mut()) {
                (Cmd::Text(t), Some(Cmd::Text(prev))) => prev.push_str(t),
                _ => out.push(cmd),
            }
        }
        self.push_text(out, "\u{3}");
        Ok(true)
    }

    /// Append text to the run in progress, looking past line directives.
    fn push_text(&self, out: &mut Vec<Cmd>, text: &str) {
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Cmd::Line(_)) {
            at -= 1;
        }
        match at.checked_sub(1).and_then(|i| out.get_mut(i)) {
            Some(Cmd::Text(t)) => t.push_str(text),
            _ => out.push(Cmd::Text(text.to_string())),
        }
    }

    /// The environment name after `\begin`, without consuming it.
    ///
    /// Read as raw characters rather than tokens: the name is needed BEFORE
    /// deciding whether the body is TeX at all, so tokenising it would be
    /// deciding the question by answering it.
    fn peek_environment_name(&self, lx: &Lexer) -> Option<String> {
        let chars = lx.chars();
        let mut i = lx.pos();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() || chars[i] != '{' {
            return None;
        }
        i += 1;
        let start = i;
        while i < chars.len() && chars[i] != '}' {
            i += 1;
        }
        match i < chars.len() {
            true => Some(chars[start..i].iter().collect()),
            false => None,
        }
    }

    /// Recognise `\def\r{BODY \ifnum A<B \r \fi}` -- TeX's loop.
    ///
    /// Returns the body tokens and the condition tokens when `name` has exactly
    /// that shape, and `None` otherwise. Deliberately narrow: the tail call must
    /// be the last thing before the closing `\fi`, the macro must take no
    /// arguments, and it must not name itself anywhere else. Anything less
    /// certain is left to inlining, where the depth bound still catches it --
    /// a recogniser that guesses would silently compile a DIFFERENT program.
    /// Whether a redefined control sequence's macro meaning should win over the
    /// primitive of the same name.
    ///
    /// It normally should: a document means what it last defined. `\end` is the
    /// one that cannot be decided by name alone, because it is two things at
    /// once. The LaTeX prelude defines `\end#1` so `\end{itemize}` runs
    /// `\enditemize`, and TeX's own `\end` is how EVERY document stops --
    /// including every LaTeX one, which stops at `\end` with nothing after it
    /// once `\end{document}` has been read. Letting the macro win outright made
    /// a bare `\end` scan for an argument that is not there and die with
    /// "Paragraph ended before argument was complete".
    ///
    /// So the group decides, which is what the two spellings already differ by:
    /// a `{` following means the environment closer, anything else means the
    /// terminator.
    fn meaning_wins(&mut self, lx: &mut Lexer, name: CsId) -> bool {
        if name.name() != "end" {
            return true;
        }
        let Some(next) = lx.next_token(&self.eng.cats) else {
            return false;
        };
        let is_group = matches!(next, Token::Char(_, Cat::BeginGroup));
        lx.push_back(&[next]);
        is_group
    }

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

    /// The one arm a DECIDED conditional keeps, with the other skipped rather
    /// than lowered.
    ///
    /// [`Lowerer::arms`] lowers both and throws one away, which is what
    /// `\ifnum` needs: its test is a register read, so both arms have to reach
    /// the bytecode. For a conditional the frontend has already decided,
    /// lowering the losing arm is not merely wasted work -- lowering EXECUTES
    /// the compile-time assignments in it. `\ifx\a\b\let\x\y\else\let\x\z\fi`
    /// ran both `\let`s and the second one won, whichever way the test went.
    /// That is what stopped LaTeX's own `\@ifnextchar` from working here, and
    /// with it every optional argument and every starred form in the language.
    fn decided_arms(&mut self, lx: &mut Lexer, taken: bool) -> R<Vec<Cmd>> {
        if !taken {
            // Nothing of the true arm is read; if it ends at `\else` the false
            // arm is the one that lowers.
            return match self.eng.compile_time_skip_arm(lx, true)? {
                false => Ok(Vec::new()),
                true => {
                    let out = self.block(lx, Some(&["fi"]))?;
                    let _ = lx.next_token(&self.eng.cats);
                    Ok(out)
                }
            };
        }
        let out = self.block(lx, Some(&["fi", "else"]))?;
        match lx.next_token(&self.eng.cats) {
            Some(Token::Cs(n)) if n.name() == "else" => {
                self.eng.compile_time_skip_arm(lx, false)?;
            }
            Some(Token::Cs(n)) if n.name() == "fi" => {}
            other => {
                if let Some(t) = other {
                    lx.push_back(&[t]);
                }
                return Err(TexError("Incomplete \\ifx; missing \\fi".into()));
            }
        }
        Ok(out)
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
            // Active characters are commands here too: `\message{~}` runs `~`.
            let t = match &t {
                Token::Char(c, Cat::Active) => match self.eng.active_meaning(*c) {
                    Some(id) => Token::Cs(id),
                    None => t,
                },
                _ => t,
            };
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
                            // `\the\pageno` reads the register the name stands
                            // for, and `\the\active` is the constant itself --
                            // known already, so it is rendered here rather than
                            // asked of the run.
                            Some(Token::Cs(w)) => match self.eng.numeric_cs(w) {
                                Some(crate::expand::NumericCs::Register(r)) => {
                                    out.push(MsgOp::Number(Num::Count(r)));
                                    continue;
                                }
                                Some(crate::expand::NumericCs::Value(v)) => {
                                    text.push_str(&v.to_string());
                                    continue;
                                }
                                None => return Err(TexError("Unsupported \\the".into())),
                            },
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

/// The environments whose bodies are characters rather than TeX.
///
/// LaTeX's own `verbatim` and `Verbatim`, the fancyvrb and listings families,
/// and `alltt`.
///
/// Pandoc's `Highlighting` and `Shaded` are deliberately NOT here. They look
/// like code environments and are not: Pandoc fills them with `\NormalTok{…}`
/// and friends, which are macros that have to expand for the code to come out
/// as code. Treating them as verbatim emits the markup instead of the program.
const VERBATIM_ENVIRONMENTS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "Verbatim*",
    "BVerbatim",
    "LVerbatim",
    "SaveVerbatim",
    "lstlisting",
    "minted",
    "alltt",
    "filecontents",
    "filecontents*",
];

impl Lowerer {
    /// The file name after `\input`, per `tex.web` §537: leading spaces are
    /// skipped and the name runs to the first space or end of line.
    ///
    /// A control sequence ends it too and is put back — `\input foo\relax` names
    /// `foo`.
    fn scan_file_name(&mut self, lx: &mut Lexer) -> R<String> {
        let mut name = String::new();
        while let Some(t) = lx.next_token(&self.eng.cats) {
            match &t {
                t if t.is_space() => {
                    if name.is_empty() {
                        continue;
                    }
                    break;
                }
                Token::Char(c, _) => name.push(*c),
                Token::Cs(_) => {
                    lx.push_back(std::slice::from_ref(&t));
                    break;
                }
            }
        }
        match name.is_empty() {
            true => Err(TexError("Missing file name".into())),
            false => Ok(name),
        }
    }

    /// Find `name`, read it, and say what to print for it.
    ///
    /// TeX resolves a file name through kpathsea; texrs searches the working
    /// directory and then `TEXINPUTS`, and deliberately does NOT shell out to
    /// `kpsewhich` — that would make running a document depend on a TeX Live
    /// installation. `.tex` is supplied when the name carries no extension, as
    /// tex does.
    fn open_input(&self, name: &str) -> R<(String, String)> {
        // tex's own bound, measured: a file that inputs itself opens 14 more
        // levels above the document's own and refuses the 15th with
        // `! TeX capacity exceeded, sorry [text input levels=15].`. Matching
        // the number AND the wording makes a runaway `\\input` agree with tex
        // rather than merely stopping, and it keeps the lowerer -- which
        // recurses on the host stack for a nested file -- inside a bound it can
        // survive.
        const MAX_LEVELS: usize = 15;
        if self.input_depth + 1 >= MAX_LEVELS {
            return Err(TexError(format!(
                "TeX capacity exceeded, sorry [text input levels={MAX_LEVELS}]"
            )));
        }
        let candidates = match std::path::Path::new(name).extension().is_some() {
            true => vec![name.to_string()],
            false => vec![format!("{name}.tex"), name.to_string()],
        };
        let mut dirs = vec![std::path::PathBuf::from(".")];
        if let Ok(paths) = std::env::var("TEXINPUTS") {
            dirs.extend(std::env::split_paths(&paths));
        }
        for dir in &dirs {
            for cand in &candidates {
                let full = dir.join(cand);
                if let Ok(src) = std::fs::read_to_string(&full) {
                    // tex prints the path it opened, and writes the working
                    // directory as `./` rather than as nothing.
                    let shown = match full.strip_prefix(".") {
                        Ok(rest) => format!("./{}", rest.display()),
                        Err(_) => full.display().to_string(),
                    };
                    return Ok((shown, src));
                }
            }
        }
        Err(TexError(format!("I can't find file `{name}'")))
    }

    /// Lower an `\input` file into the stream, sharing the Lowerer's state.
    ///
    /// The same nested pass `preload` runs for the LaTeX prelude, except that
    /// the commands are kept: a preload only wants the definitions, while an
    /// `\input` file can also print.
    fn input_pass(&mut self, src: &str) -> R<Vec<Cmd>> {
        let mut lx = Lexer::new(src);
        self.block(&mut lx, None)
    }
}

/// Every count register a command block assigns, so a group knows what to save.
fn assigned_counts(cmds: &[Cmd]) -> Vec<i64> {
    let mut regs = Vec::new();
    fn walk(cmds: &[Cmd], regs: &mut Vec<i64>) {
        for c in cmds {
            match c {
                // A line directive, a run of the document's text, a
                // `\rust{ … }` compile and a file's closing paren all write no
                // register.
                Cmd::Line(_) | Cmd::Text(_) | Cmd::RustCompile(_) | Cmd::FileClose => {}
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
                Cmd::Loop { body, .. } | Cmd::Color { body, .. } | Cmd::Group { body, .. } => {
                    walk(body, regs)
                }
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
