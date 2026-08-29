//! TeX's expander: the half of the engine that turns tokens into other tokens.
//!
//! Everything here is "mouth" work in Knuth's sense — `\def`, `\csname`, the
//! conditionals and `\the` all produce token lists and never build a box. That
//! separation is the reason this is the tractable half to implement first, and
//! it is also the half where the time goes in a real document: a macro-heavy
//! preamble expands far more tokens than it typesets.

use crate::catcode::{cat_from_i64, Cat, CatTable};
use crate::lexer::Lexer;
use crate::token::Token;
use std::collections::HashMap;

/// A `\def`'d macro: the parameter text as written, and the body.
#[derive(Clone)]
pub struct Macro {
    /// The parameter text between the name and the body — `#1#2`, or a
    /// delimited form like `#1,#2.`. Stored as tokens because the delimiters
    /// are arbitrary token sequences, not just characters.
    pub params: Vec<Token>,
    pub body: Vec<Token>,
}

pub struct Engine {
    pub cats: CatTable,
    pub macros: HashMap<String, Macro>,
    pub count: HashMap<i64, i64>,
    /// What `\message` has written, in order. The terminal contract is
    /// space-separated (`tex.web` §1279 adds a space before a message when the
    /// line is not empty), which the driver reproduces.
    pub messages: Vec<String>,
    pub escape: char,
}

#[derive(Debug)]
pub struct TexError(pub String);

type R<T> = Result<T, TexError>;

impl Engine {
    pub fn new() -> Self {
        Self {
            cats: CatTable::new(),
            macros: HashMap::new(),
            count: HashMap::new(),
            messages: Vec::new(),
            escape: '\\',
        }
    }

    /// Run a source file to `\end` or exhaustion.
    pub fn run(&mut self, src: &str) -> R<()> {
        let mut lx = Lexer::new(src);
        while let Some(tok) = lx.next_token(&self.cats) {
            if self.step(&mut lx, tok)? {
                break;
            }
        }
        Ok(())
    }

    /// One token of execution. `Ok(true)` means `\end` was seen.
    fn step(&mut self, lx: &mut Lexer, tok: Token) -> R<bool> {
        let Token::Cs(name) = &tok else {
            return Ok(false);
        };
        match name.as_str() {
            "end" => return Ok(true),
            "def" => self.do_def(lx)?,
            "catcode" => self.do_catcode(lx)?,
            "count" => self.do_count_assign(lx)?,
            "advance" => self.do_arith(lx, Arith::Add)?,
            "multiply" => self.do_arith(lx, Arith::Mul)?,
            "divide" => self.do_arith(lx, Arith::Div)?,
            "message" => {
                let text = self.read_group_text(lx)?;
                self.messages.push(text);
            }
            "relax" | "par" => {}
            other => {
                if self.macros.contains_key(other) {
                    self.expand_macro(lx, other)?;
                } else {
                    return Err(TexError(format!("Undefined control sequence \\{other}")));
                }
            }
        }
        Ok(false)
    }

    // ── definitions ──────────────────────────────────────────────────────

    /// `\def\name<params>{<body>}`.
    fn do_def(&mut self, lx: &mut Lexer) -> R<()> {
        let Some(Token::Cs(name)) = lx.next_token(&self.cats) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        let mut params = Vec::new();
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError("Runaway definition".into()));
            };
            if matches!(t, Token::Char(_, Cat::BeginGroup)) {
                break;
            }
            params.push(t);
        }
        let body = self.read_balanced(lx)?;
        self.macros.insert(name, Macro { params, body });
        Ok(())
    }

    /// Read to the matching end-group, the opening brace already consumed.
    fn read_balanced(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        let mut depth = 1usize;
        let mut out = Vec::new();
        while let Some(t) = lx.next_token(&self.cats) {
            match &t {
                Token::Char(_, Cat::BeginGroup) => depth += 1,
                Token::Char(_, Cat::EndGroup) => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                }
                _ => {}
            }
            out.push(t);
        }
        Err(TexError("Runaway argument".into()))
    }

    // ── macro calls ──────────────────────────────────────────────────────

    /// Bind the arguments a macro's parameter text calls for, then push the
    /// substituted body back in front of the input.
    ///
    /// Undelimited (`#1`) takes one balanced group or a single token; delimited
    /// (`#1,`) takes everything up to the next occurrence of the delimiter. Both
    /// are `tex.web` §392's `macro_call`, and both are common enough in real
    /// documents that supporting only the first would be useless.
    fn expand_macro(&mut self, lx: &mut Lexer, name: &str) -> R<()> {
        let m = self.macros[name].clone();
        let args = self.match_params(lx, &m.params)?;
        let mut out = Vec::with_capacity(m.body.len());
        let mut i = 0;
        while i < m.body.len() {
            match &m.body[i] {
                Token::Char(_, Cat::Param) => {
                    // `#1`..`#9` substitutes; `##` is a literal `#`.
                    match m.body.get(i + 1) {
                        Some(Token::Char(d, _)) if d.is_ascii_digit() && *d != '0' => {
                            let idx = (*d as u8 - b'1') as usize;
                            if let Some(a) = args.get(idx) {
                                out.extend(a.iter().cloned());
                            }
                            i += 2;
                            continue;
                        }
                        Some(Token::Char(_, Cat::Param)) => {
                            out.push(m.body[i + 1].clone());
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                    out.push(m.body[i].clone());
                }
                t => out.push(t.clone()),
            }
            i += 1;
        }
        lx.push_back(&out);
        Ok(())
    }

    fn match_params(&mut self, lx: &mut Lexer, params: &[Token]) -> R<Vec<Vec<Token>>> {
        let mut args: Vec<Vec<Token>> = Vec::new();
        let mut i = 0;
        while i < params.len() {
            let is_param = matches!(params[i], Token::Char(_, Cat::Param));
            if !is_param {
                // Literal text in the parameter text must match the input.
                let want = params[i].clone();
                let got = lx.next_token(&self.cats);
                if got.as_ref() != Some(&want) {
                    return Err(TexError("Use of macro doesn't match its definition".into()));
                }
                i += 1;
                continue;
            }
            // `#n` — look at what follows to decide delimited vs undelimited.
            let delim: Vec<Token> = params[i + 2..]
                .iter()
                .take_while(|t| !matches!(t, Token::Char(_, Cat::Param)))
                .cloned()
                .collect();
            let arg = match delim.is_empty() {
                true => self.read_undelimited(lx)?,
                false => self.read_delimited(lx, &delim)?,
            };
            args.push(arg);
            i += 2 + delim.len();
        }
        Ok(args)
    }

    /// One balanced group (braces stripped) or one token.
    fn read_undelimited(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError(
                    "Paragraph ended before argument was complete".into(),
                ));
            };
            if t.is_space() {
                continue; // Leading spaces before an argument are skipped.
            }
            return match t {
                Token::Char(_, Cat::BeginGroup) => self.read_balanced(lx),
                other => Ok(vec![other]),
            };
        }
    }

    /// Everything up to the next occurrence of `delim`, brace-aware. A single
    /// group wrapping the whole argument has its braces stripped, as TeX does.
    fn read_delimited(&mut self, lx: &mut Lexer, delim: &[Token]) -> R<Vec<Token>> {
        let mut out: Vec<Token> = Vec::new();
        let mut depth = 0usize;
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError(
                    "Paragraph ended before argument was complete".into(),
                ));
            };
            match &t {
                Token::Char(_, Cat::BeginGroup) => depth += 1,
                Token::Char(_, Cat::EndGroup) => depth = depth.saturating_sub(1),
                _ => {}
            }
            out.push(t);
            if depth == 0 && out.len() >= delim.len() && out[out.len() - delim.len()..] == *delim {
                out.truncate(out.len() - delim.len());
                let stripped = out.len() >= 2
                    && matches!(out[0], Token::Char(_, Cat::BeginGroup))
                    && matches!(out[out.len() - 1], Token::Char(_, Cat::EndGroup));
                if stripped {
                    out.remove(0);
                    out.pop();
                }
                return Ok(out);
            }
        }
    }
    // ── assignments ──────────────────────────────────────────────────────

    /// `\catcode<number>=<number>`.
    fn do_catcode(&mut self, lx: &mut Lexer) -> R<()> {
        let ch = self.read_number(lx)?;
        self.skip_equals(lx)?;
        let val = self.read_number(lx)?;
        let (Some(c), Some(cat)) = (char::from_u32(ch as u32), cat_from_i64(val)) else {
            return Err(TexError("Invalid code".into()));
        };
        self.cats.set(c, cat);
        Ok(())
    }

    /// `\count<n>=<number>`.
    fn do_count_assign(&mut self, lx: &mut Lexer) -> R<()> {
        let reg = self.read_number(lx)?;
        self.skip_equals(lx)?;
        let val = self.read_number(lx)?;
        self.count.insert(reg, val);
        Ok(())
    }

    fn do_arith(&mut self, lx: &mut Lexer, op: Arith) -> R<()> {
        let Some(Token::Cs(what)) = lx.next_token(&self.cats) else {
            return Err(TexError("You can't use this after \\advance".into()));
        };
        if what != "count" {
            return Err(TexError(format!("Unsupported register \\{what}")));
        }
        let reg = self.read_number(lx)?;
        self.skip_by(lx)?;
        let val = self.read_number(lx)?;
        let cur = *self.count.get(&reg).unwrap_or(&0);
        let next = match op {
            Arith::Add => cur + val,
            Arith::Mul => cur * val,
            // TeX truncates toward zero and refuses a zero divisor.
            Arith::Div => match val {
                0 => return Err(TexError("Arithmetic overflow".into())),
                d => cur / d,
            },
        };
        self.count.insert(reg, next);
        Ok(())
    }

    /// The optional `by` keyword before an arithmetic operand.
    ///
    /// `\advance\count0 by 5` puts a space before the keyword, and TeX's number
    /// scanner skips spaces wherever it looks for one — so the keyword match has
    /// to skip them too, or `by` is read as the number and the assignment fails.
    fn skip_by(&mut self, lx: &mut Lexer) -> R<()> {
        while let Some(t) = lx.next_token(&self.cats) {
            if !t.is_space() {
                lx.push_back(&[t]);
                break;
            }
        }
        let save = lx.pending.clone();
        for want in ['b', 'y'] {
            match lx.next_token(&self.cats) {
                Some(Token::Char(c, _)) if c.eq_ignore_ascii_case(&want) => {}
                other => {
                    lx.pending = save;
                    if let Some(t) = other {
                        lx.push_back(&[t]);
                    }
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn skip_equals(&mut self, lx: &mut Lexer) -> R<()> {
        while let Some(t) = lx.next_token(&self.cats) {
            match &t {
                t if t.is_space() => continue,
                Token::Char('=', _) => return Ok(()),
                other => {
                    lx.push_back(&[other.clone()]);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// An integer constant, optionally signed, optionally a backtick character
    /// code (`` `\{ ``), optionally `\count<n>`.
    fn read_number(&mut self, lx: &mut Lexer) -> R<i64> {
        self.scan_number(lx, false)
    }

    /// The same scanner, restricted to already-expanded tokens.
    ///
    /// `\the\count0` inside a `\message` is read from the pushed-back token
    /// list, and the digit is the LAST of them. A scanner that then asks the
    /// mouth for one more token to test for a further digit pulls the newline
    /// after the closing brace out of the FILE and renders it into the message
    /// — `a=7 ` where tex writes `a=7`. During expansion the token list is the
    /// whole world, so running out of it ends the number.
    fn read_number_pending(&mut self, lx: &mut Lexer) -> R<i64> {
        self.scan_number(lx, true)
    }

    fn scan_number(&mut self, lx: &mut Lexer, pending_only: bool) -> R<i64> {
        let take = |lx: &mut Lexer, cats: &CatTable| match pending_only {
            true => lx.pending.pop(),
            false => lx.next_token(cats),
        };
        let mut sign = 1i64;
        let mut cur = loop {
            let Some(t) = take(lx, &self.cats) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            match &t {
                t if t.is_space() => continue,
                Token::Char('-', _) => {
                    sign = -sign;
                    continue;
                }
                Token::Char('+', _) => continue,
                other => break other.clone(),
            }
        };
        // `` `x `` and `` `\x `` are the character code of x.
        if matches!(cur, Token::Char('`', _)) {
            let Some(t) = take(lx, &self.cats) else {
                return Err(TexError("Missing number".into()));
            };
            let code = match t {
                Token::Char(c, _) => u32::from(c) as i64,
                Token::Cs(name) => name
                    .chars()
                    .next()
                    .map(|c| u32::from(c) as i64)
                    .unwrap_or(0),
            };
            return Ok(sign * code);
        }
        if let Token::Cs(name) = &cur {
            if name == "count" {
                let reg = self.scan_number(lx, pending_only)?;
                return Ok(sign * *self.count.get(&reg).unwrap_or(&0));
            }
            return Err(TexError(format!("Missing number, found \\{name}")));
        }
        let mut digits = String::new();
        loop {
            match &cur {
                Token::Char(c, _) if c.is_ascii_digit() => digits.push(*c),
                other => {
                    lx.push_back(&[other.clone()]);
                    break;
                }
            }
            match take(lx, &self.cats) {
                Some(t) => cur = t,
                None => break,
            }
        }
        if digits.is_empty() {
            return Err(TexError("Missing number, treated as zero".into()));
        }
        digits
            .parse::<i64>()
            .map(|n| sign * n)
            .map_err(|_| TexError("Number too big".into()))
    }

    // ── \message ─────────────────────────────────────────────────────────

    /// Read `{...}` and expand it to the text `\message` prints.
    fn read_group_text(&mut self, lx: &mut Lexer) -> R<String> {
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError("Missing { inserted".into()));
            };
            if t.is_space() {
                continue;
            }
            if !matches!(t, Token::Char(_, Cat::BeginGroup)) {
                return Err(TexError("Missing { inserted".into()));
            }
            break;
        }
        let body = self.read_balanced(lx)?;
        self.expand_to_text(lx, &body)
    }

    /// Fully expand a token list and render it, which is what `\message` and
    /// `\edef` both need.
    fn expand_to_text(&mut self, lx: &mut Lexer, toks: &[Token]) -> R<String> {
        let saved: Vec<Token> = std::mem::take(&mut lx.pending);
        lx.push_back(toks);
        let mut out = String::new();
        let depth_guard = 100_000usize;
        let mut steps = 0usize;
        while let Some(t) = lx.pending.pop() {
            steps += 1;
            if steps > depth_guard {
                return Err(TexError("TeX capacity exceeded".into()));
            }
            match &t {
                Token::Cs(name) if self.macros.contains_key(name) => {
                    self.expand_macro(lx, name)?;
                }
                Token::Cs(name) if name == "the" => {
                    let n = self.read_the(lx)?;
                    out.push_str(&n);
                }
                Token::Cs(name) if name == "string" => {
                    // `\string` is `sprint_cs` (tex.web §262), NOT `print_cs`:
                    // it writes the escape and the name and stops. `print_cs`'s
                    // trailing space after a multi-letter name is for showing a
                    // token, so `\string\foo` is `\foo` and not `\foo `.
                    if let Some(next) = lx.pending.pop() {
                        out.push_str(&match &next {
                            Token::Cs(n) => format!("{}{n}", self.escape),
                            other => other.to_text(self.escape),
                        });
                    }
                }
                Token::Cs(name) if name == "csname" => {
                    let built = self.read_csname(lx)?;
                    lx.push_back(&[Token::Cs(built)]);
                }
                other => out.push_str(&other.to_text(self.escape)),
            }
        }
        lx.pending = saved;
        Ok(out)
    }

    /// `\the\count<n>` — the only `\the` this milestone answers.
    fn read_the(&mut self, lx: &mut Lexer) -> R<String> {
        let Some(Token::Cs(what)) = lx.pending.pop() else {
            return Err(TexError("You can't use \\the here".into()));
        };
        if what != "count" {
            return Err(TexError(format!("Unsupported \\the\\{what}")));
        }
        let reg = self.read_number_pending(lx)?;
        Ok(self.count.get(&reg).unwrap_or(&0).to_string())
    }

    /// `\csname ... \endcsname` builds a control sequence name from expanded text.
    fn read_csname(&mut self, lx: &mut Lexer) -> R<String> {
        let mut name = String::new();
        loop {
            let Some(t) = lx.pending.pop() else {
                return Err(TexError("Missing \\endcsname inserted".into()));
            };
            match &t {
                Token::Cs(n) if n == "endcsname" => return Ok(name),
                Token::Cs(n) if self.macros.contains_key(n) => self.expand_macro(lx, n)?,
                Token::Char(c, _) => name.push(*c),
                Token::Cs(n) => return Err(TexError(format!("Missing \\endcsname before \\{n}"))),
            }
        }
    }
}

enum Arith {
    Add,
    Mul,
    Div,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
