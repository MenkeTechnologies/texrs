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
use crate::ir::{Arith, Cmd, Num, Part, Rel};
use crate::lexer::Lexer;
use crate::token::Token;

type R<T> = Result<T, TexError>;

pub struct Lowerer {
    pub eng: Engine,
}

impl Lowerer {
    pub fn new() -> Self {
        Self { eng: Engine::new() }
    }

    /// Compile a whole source to a command stream.
    pub fn lower(&mut self, src: &str) -> R<Vec<Cmd>> {
        let mut lx = Lexer::new(src);
        self.block(&mut lx, None)
    }

    /// Lower commands until the input ends, `\end` is seen, or one of `stop` is
    /// reached at nesting depth zero (used for a conditional's arms).
    fn block(&mut self, lx: &mut Lexer, stop: Option<&[&str]>) -> R<Vec<Cmd>> {
        let mut out = Vec::new();
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            let Token::Cs(name) = &tok else {
                // Braces group the macro table while lowering, so a `\def`
                // inside them is undone at the `}` exactly as TeX undoes it.
                match &tok {
                    Token::Char(_, Cat::BeginGroup) => self.eng.compile_time_begin_group(),
                    Token::Char(_, Cat::EndGroup) => self.eng.compile_time_end_group()?,
                    _ => {}
                }
                continue;
            };
            let name = name.clone();
            if let Some(stops) = stop {
                if stops.contains(&name.as_str()) {
                    lx.push_back(&[Token::Cs(name)]);
                    return Ok(out);
                }
            }
            match name.as_str() {
                "end" => break,
                // Compile-time: these change how the REST of the file reads.
                "def" | "gdef" => self.eng.compile_time_def(lx, &name)?,
                "catcode" => self.eng.compile_time_catcode(lx)?,
                "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    self.eng.skip_equals_file(lx)?;
                    let v = self.number(lx)?;
                    out.push(Cmd::SetCount(reg, v));
                }
                "advance" | "multiply" | "divide" => {
                    let op = match name.as_str() {
                        "advance" => Arith::Add,
                        "multiply" => Arith::Mul,
                        _ => Arith::Div,
                    };
                    let Some(Token::Cs(what)) = lx.next_token(&self.eng.cats) else {
                        return Err(TexError("You can't use this after \\advance".into()));
                    };
                    if what != "count" {
                        return Err(TexError(format!("Unsupported register \\{what}")));
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
                    let taken = name == "iftrue";
                    let (t, e) = self.arms(lx)?;
                    out.extend(if taken { t } else { e });
                }
                "ifx" => {
                    let same = self.eng.ifx_equal(lx)?;
                    let (t, e) = self.arms(lx)?;
                    out.extend(if same { t } else { e });
                }
                "let" => self.eng.compile_time_let(lx)?,
                "edef" | "xdef" => self.eng.compile_time_def(lx, &name)?,
                "begingroup" => self.eng.compile_time_begin_group(),
                "endgroup" => self.eng.compile_time_end_group()?,
                "global" => self.eng.set_global_prefix(true),
                "relax" | "par" => {}
                other => {
                    // A macro expands into the stream and lowering continues
                    // through its body -- expansion is a frontend concern.
                    if matches!(self.eng.meanings.get(other), Some(Meaning::Macro(_))) {
                        self.eng.expand_macro_file(lx, other)?;
                        continue;
                    }
                    return Err(TexError(format!("Undefined control sequence \\{other}")));
                }
            }
        }
        Ok(out)
    }

    /// The `\else` and `\fi` arms of a conditional, each lowered.
    fn arms(&mut self, lx: &mut Lexer) -> R<(Vec<Cmd>, Vec<Cmd>)> {
        let then_branch = self.block(lx, Some(&["else", "fi"]))?;
        let mut else_branch = Vec::new();
        match lx.next_token(&self.eng.cats) {
            Some(Token::Cs(n)) if n == "else" => {
                else_branch = self.block(lx, Some(&["fi"]))?;
                // Consume the `\fi`.
                let _ = lx.next_token(&self.eng.cats);
            }
            Some(Token::Cs(n)) if n == "fi" => {}
            other => {
                if let Some(t) = other {
                    lx.push_back(&[t]);
                }
                return Err(TexError("Incomplete \\ifnum; missing \\fi".into()));
            }
        }
        Ok((then_branch, else_branch))
    }

    /// A number operand: a literal, or `\count<n>` which must be read at run time.
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
                Token::Cs(n) if n == "count" => {
                    let reg = self.eng.scan_number_file(lx)?;
                    return Ok(Num::Count(reg));
                }
                _ => {
                    lx.push_back(&[t]);
                    return Ok(Num::Literal(self.eng.scan_number_file(lx)?));
                }
            }
        }
    }

    /// `\message{...}` split into fixed text and run-time numbers.
    fn message_parts(&mut self, lx: &mut Lexer) -> R<Vec<Part>> {
        let body = self.eng.read_message_body(lx)?;
        let mut parts: Vec<Part> = Vec::new();
        let mut text = String::new();
        let mut work = Lexer::new("");
        work.push_back(&body);
        while let Some(t) = work.pending.pop() {
            match &t {
                Token::Cs(n) if n == "the" || n == "number" => {
                    // `\the\count0` is a register READ, deferred to run time.
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    let bare = n == "number";
                    if bare {
                        // `\number<literal>` is known now; only a register read
                        // has to wait for the VM.
                        let peek = work.pending.last().cloned();
                        let is_reg = matches!(&peek, Some(Token::Cs(w)) if w == "count");
                        if !is_reg {
                            let v = self.eng.scan_number_pending(&mut work)?;
                            text.push_str(&v.to_string());
                            if !text.is_empty() {
                                parts.push(Part::Text(std::mem::take(&mut text)));
                            }
                            continue;
                        }
                        let _ = work.pending.pop();
                        let reg = self.eng.scan_number_pending(&mut work)?;
                        parts.push(Part::Number(Num::Count(reg)));
                        continue;
                    }
                    if !bare {
                        match work.pending.pop() {
                            Some(Token::Cs(w)) if w == "count" => {}
                            _ => return Err(TexError("Unsupported \\the".into())),
                        }
                    }
                    let reg = self.eng.scan_number_pending(&mut work)?;
                    parts.push(Part::Number(Num::Count(reg)));
                }
                Token::Cs(n) if n == "string" => {
                    // `\string` is `sprint_cs`: escape + name, no trailing space.
                    if let Some(next) = work.pending.pop() {
                        text.push_str(&match &next {
                            Token::Cs(cs) => format!("{}{cs}", self.eng.escape),
                            other => other.to_text(self.eng.escape),
                        });
                    }
                }
                Token::Cs(n) if matches!(self.eng.meanings.get(n), Some(Meaning::Macro(_))) => {
                    let n = n.clone();
                    self.eng.expand_macro_pending(&mut work, &n)?;
                }
                other => text.push_str(&other.to_text(self.eng.escape)),
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(parts)
    }
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
