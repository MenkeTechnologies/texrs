//! TeX's expander: the half of the engine that turns tokens into other tokens.
//!
//! Everything here is "mouth" work in Knuth's sense — `\def`, `\csname`, the
//! conditionals and `\the` all produce token lists and never build a box.
//!
//! Two reading contexts exist and the difference is load-bearing. Executing a
//! file reads from the mouth, which falls through to the source when the
//! pushed-back list runs dry. Expanding a token list — a `\message` body, an
//! `\edef` body — must NOT: the list is the whole world, and a scanner that
//! reaches past it pulls the next line of the FILE into the result. Every
//! scanner below therefore takes `pending_only` rather than guessing.

use crate::catcode::{cat_from_i64, Cat, CatTable};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};
use std::collections::HashMap;

/// A `\def`'d macro: the parameter text as written, and the body.
#[derive(Clone, PartialEq)]
pub struct Macro {
    pub params: Vec<Token>,
    pub body: Vec<Token>,
}

/// What a control sequence means. `\let` is why this is a value and not just a
/// macro table: `\let\a=\relax` makes `\a` a primitive, not a macro.
#[derive(Clone, PartialEq)]
pub enum Meaning {
    Macro(Macro),
    /// A primitive, by its canonical name (`relax`, `par`, …).
    Primitive(CsId),
    /// `\let\a=b` — a character token.
    Char(char, Cat),
}

/// One undo record. TeX's save stack restores individual changes at group end
/// rather than snapshotting the whole state, which is what makes deep grouping
/// affordable; the same choice is made here.
enum Save {
    Cat(char, Cat),
    Count(i64, Option<i64>),
    Meaning(CsId, Option<Meaning>),
}

pub struct Engine {
    pub cats: CatTable,
    /// What each control sequence means, keyed by interned id.
    ///
    /// Keyed on `CsId` rather than the name: a lookup here happens on every
    /// control sequence the expander touches, and hashing a `u32` is what makes
    /// that cheap. `tex.web` reaches `eqtb` the same way, by the pointer the
    /// hash table already produced.
    pub meanings: HashMap<CsId, Meaning>,
    pub count: HashMap<i64, i64>,
    pub messages: Vec<String>,
    pub escape: char,
    /// One frame per open group; each holds the undo records for that group.
    groups: Vec<Vec<Save>>,
    /// Open conditionals, so `\else`/`\fi` know what they close.
    conds: Vec<CondState>,
    /// Set by `\global`, cleared by the assignment it prefixes.
    global: bool,
}

#[derive(PartialEq, Clone, Copy)]
enum CondState {
    /// The true branch is running; an `\else` here starts skipping.
    Taken,
    /// A branch was skipped to get here; `\else` must not re-enter.
    Done,
}

#[derive(Debug)]
pub struct TexError(pub String);

type R<T> = Result<T, TexError>;

/// Every conditional primitive, so the skipper can count nesting correctly.
/// Missing one here would make a skipped `\ifnum` eat its own `\fi` and unbalance
/// everything after it.
const CONDITIONALS: &[&str] = &[
    "if",
    "ifcat",
    "ifnum",
    "ifdim",
    "ifodd",
    "ifvmode",
    "ifhmode",
    "ifmmode",
    "ifinner",
    "ifvoid",
    "ifhbox",
    "ifvbox",
    "ifx",
    "ifeof",
    "iftrue",
    "iffalse",
    "ifcase",
    "ifdefined",
    "ifcsname",
];

impl Engine {
    pub fn new() -> Self {
        Self {
            cats: CatTable::new(),
            meanings: HashMap::new(),
            count: HashMap::new(),
            messages: Vec::new(),
            escape: '\\',
            groups: Vec::new(),
            conds: Vec::new(),
            global: false,
        }
    }

    pub fn run(&mut self, src: &str) -> R<()> {
        let mut lx = Lexer::new(src);
        while let Some(tok) = lx.next_token(&self.cats) {
            if self.step(&mut lx, tok)? {
                break;
            }
        }
        Ok(())
    }

    // ── grouping ─────────────────────────────────────────────────────────

    fn begin_group(&mut self) {
        self.groups.push(Vec::new());
    }

    /// Undo this group's assignments, newest first.
    fn end_group(&mut self) -> R<()> {
        let Some(frame) = self.groups.pop() else {
            return Err(TexError("Too many }'s".into()));
        };
        for save in frame.into_iter().rev() {
            match save {
                Save::Cat(c, cat) => self.cats.set(c, cat),
                Save::Count(reg, old) => match old {
                    Some(v) => {
                        self.count.insert(reg, v);
                    }
                    None => {
                        self.count.remove(&reg);
                    }
                },
                Save::Meaning(name, old) => match old {
                    Some(m) => {
                        self.meanings.insert(name, m);
                    }
                    None => {
                        self.meanings.remove(&name);
                    }
                },
            }
        }
        Ok(())
    }

    /// Record the current value so the enclosing group can put it back.
    ///
    /// `\global` skips every frame, which is what makes `\gdef` survive to the
    /// outermost level rather than only to the next `}`.
    fn save(&mut self, rec: Save) {
        if self.global || self.groups.is_empty() {
            return;
        }
        if let Some(frame) = self.groups.last_mut() {
            frame.push(rec);
        }
    }

    fn set_meaning(&mut self, name: CsId, m: Meaning) {
        let old = self.meanings.get(&name).cloned();
        self.save(Save::Meaning(name, old));
        // A global assignment wipes the saved values other groups hold, so no
        // `}` can restore the old meaning over it.
        if self.global {
            for frame in &mut self.groups {
                frame.retain(|s| !matches!(s, Save::Meaning(n, _) if *n == name));
            }
        }
        self.meanings.insert(name, m);
    }

    fn set_count(&mut self, reg: i64, val: i64) {
        let old = self.count.get(&reg).copied();
        self.save(Save::Count(reg, old));
        self.count.insert(reg, val);
    }

    fn set_cat(&mut self, c: char, cat: Cat) {
        self.save(Save::Cat(c, self.cats.get(c)));
        self.cats.set(c, cat);
    }

    // ── execution ────────────────────────────────────────────────────────

    /// One token of execution. `Ok(true)` means `\end` was seen.
    fn step(&mut self, lx: &mut Lexer, tok: Token) -> R<bool> {
        match &tok {
            Token::Char(_, Cat::BeginGroup) => {
                self.begin_group();
                return Ok(false);
            }
            Token::Char(_, Cat::EndGroup) => {
                self.end_group()?;
                return Ok(false);
            }
            Token::Char(..) => return Ok(false),
            Token::Cs(_) => {}
        }
        let Token::Cs(name) = &tok else {
            return Ok(false);
        };
        let name = *name;
        if self.try_expand(lx, name, false)? {
            return Ok(false);
        }
        match name.name() {
            "end" => return Ok(true),
            kind @ ("def" | "gdef" | "edef" | "xdef") => self.do_def(lx, kind)?,
            "let" => self.do_let(lx)?,
            "global" => {
                self.global = true;
                let Some(next) = lx.next_token(&self.cats) else {
                    return Err(TexError("Missing control sequence".into()));
                };
                let out = self.step(lx, next);
                self.global = false;
                return out;
            }
            "begingroup" => self.begin_group(),
            "endgroup" => self.end_group()?,
            "catcode" => self.do_catcode(lx)?,
            "count" => self.do_count_assign(lx)?,
            "advance" => self.do_arith(lx, Arith::Add)?,
            "multiply" => self.do_arith(lx, Arith::Mul)?,
            "divide" => self.do_arith(lx, Arith::Div)?,
            "message" => {
                let text = self.read_group_text(lx)?;
                self.messages.push(text);
            }
            "relax" | "par" | "ignorespaces" => {}
            other => {
                return Err(TexError(format!("Undefined control sequence \\{other}")));
            }
        }
        Ok(false)
    }

    // ── the expandable primitives ────────────────────────────────────────

    /// Handle `name` if it is expandable, returning whether it was.
    ///
    /// This is the gullet. Both the executor and `expand_to_text` route through
    /// it so a conditional behaves identically in a file and inside a `\message`
    /// body — in TeX they are the same machinery, and splitting them here would
    /// make `\ifnum` work in one place and not the other.
    fn try_expand(&mut self, lx: &mut Lexer, name: CsId, pending_only: bool) -> R<bool> {
        if let Some(Meaning::Macro(_)) = self.meanings.get(&name) {
            self.expand_macro(lx, name, pending_only)?;
            return Ok(true);
        }
        match name.name() {
            n if CONDITIONALS.contains(&n) => {
                self.do_conditional(lx, n, pending_only)?;
                Ok(true)
            }
            "else" => {
                // Reaching `\else` in running text means the true branch ran;
                // everything to the matching `\fi` is skipped.
                match self.conds.pop() {
                    Some(_) => {
                        self.skip_to(lx, false, pending_only)?;
                        Ok(true)
                    }
                    None => Err(TexError("Extra \\else".into())),
                }
            }
            "or" => match self.conds.pop() {
                Some(_) => {
                    self.skip_to(lx, false, pending_only)?;
                    Ok(true)
                }
                None => Err(TexError("Extra \\or".into())),
            },
            "fi" => match self.conds.pop() {
                Some(_) => Ok(true),
                None => Err(TexError("Extra \\fi".into())),
            },
            "expandafter" => {
                self.do_expandafter(lx, pending_only)?;
                Ok(true)
            }
            "noexpand" => {
                // The next token is passed through unexpanded. Reading it and
                // pushing it back is enough here because nothing re-examines it.
                if let Some(t) = self.take(lx, pending_only) {
                    lx.push_back(&[t]);
                }
                Ok(true)
            }
            "csname" => {
                let built = self.read_csname(lx, pending_only)?;
                // An undefined \csname name becomes \relax, per tex.web §372.
                let id = CsId::intern(&built);
                self.meanings
                    .entry(id)
                    .or_insert_with(|| Meaning::Primitive(CsId::intern("relax")));
                lx.push_back(&[Token::Cs(id)]);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Read one token from whichever source this context uses.
    fn take(&mut self, lx: &mut Lexer, pending_only: bool) -> Option<Token> {
        match pending_only {
            true => lx.pending.pop(),
            false => lx.next_token(&self.cats),
        }
    }

    /// `\expandafter\A\B` — hold `\A`, expand `\B` once, then put `\A` back.
    fn do_expandafter(&mut self, lx: &mut Lexer, pending_only: bool) -> R<()> {
        let Some(held) = self.take(lx, pending_only) else {
            return Err(TexError("Missing token after \\expandafter".into()));
        };
        let Some(next) = self.take(lx, pending_only) else {
            return Err(TexError("Missing token after \\expandafter".into()));
        };
        match &next {
            Token::Cs(n) => {
                let n = *n;
                if !self.try_expand(lx, n, true)? {
                    lx.push_back(&[next]);
                }
            }
            _ => lx.push_back(&[next]),
        }
        lx.push_back(&[held]);
        Ok(())
    }

    // ── conditionals ─────────────────────────────────────────────────────

    fn do_conditional(&mut self, lx: &mut Lexer, name: &str, pending_only: bool) -> R<()> {
        let truth = match name {
            "iftrue" => true,
            "iffalse" => false,
            "ifnum" => {
                let a = self.scan_number(lx, pending_only)?;
                let rel = self.read_relation(lx, pending_only)?;
                let b = self.scan_number(lx, pending_only)?;
                match rel {
                    '<' => a < b,
                    '>' => a > b,
                    _ => a == b,
                }
            }
            "ifodd" => self.scan_number(lx, pending_only)? % 2 != 0,
            "ifdefined" => match self.take(lx, pending_only) {
                Some(Token::Cs(n)) => self.meanings.contains_key(&n),
                _ => false,
            },
            "ifx" => {
                let a = self.take(lx, pending_only);
                let b = self.take(lx, pending_only);
                self.meanings_equal(a.as_ref(), b.as_ref())
            }
            "if" => {
                // `\if` compares CHARACTER CODES after expansion; a control
                // sequence compares equal to any other control sequence.
                let a = self.expand_one_char(lx, pending_only)?;
                let b = self.expand_one_char(lx, pending_only)?;
                a == b
            }
            "ifcase" => {
                let n = self.scan_number(lx, pending_only)?;
                return self.do_ifcase(lx, n, pending_only);
            }
            other => return Err(TexError(format!("Unsupported conditional \\{other}"))),
        };
        match truth {
            true => self.conds.push(CondState::Taken),
            false => {
                // Skip to `\else` (run that branch) or `\fi` (run nothing).
                let hit_else = self.skip_to(lx, true, pending_only)?;
                if hit_else {
                    self.conds.push(CondState::Done);
                }
            }
        }
        Ok(())
    }

    /// `\ifcase<n>` selects the nth `\or` branch, `\else` being the default.
    fn do_ifcase(&mut self, lx: &mut Lexer, n: i64, pending_only: bool) -> R<()> {
        let mut remaining = n;
        while remaining > 0 {
            // Skip one branch; landing on `\fi` means no branch matched.
            let hit_else = self.skip_to(lx, true, pending_only)?;
            if !hit_else {
                return Ok(());
            }
            remaining -= 1;
        }
        self.conds.push(CondState::Taken);
        Ok(())
    }

    /// Skip tokens to the matching `\else`/`\or` (when `stop_at_else`) or `\fi`.
    ///
    /// Returns whether it stopped at an `\else`/`\or` rather than the `\fi`.
    /// Nested conditionals are counted so a skipped branch containing its own
    /// `\if` does not mistake that one's `\fi` for the outer one's.
    fn skip_to(&mut self, lx: &mut Lexer, stop_at_else: bool, pending_only: bool) -> R<bool> {
        let mut depth = 0usize;
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Incomplete \\if; all text was ignored".into()));
            };
            let Token::Cs(n) = &t else { continue };
            match n.name() {
                n if CONDITIONALS.contains(&n) => depth += 1,
                "fi" => match depth {
                    0 => return Ok(false),
                    _ => depth -= 1,
                },
                "else" | "or" if depth == 0 && stop_at_else => return Ok(true),
                _ => {}
            }
        }
    }

    fn read_relation(&mut self, lx: &mut Lexer, pending_only: bool) -> R<char> {
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing = inserted for \\ifnum".into()));
            };
            match &t {
                t if t.is_space() => continue,
                Token::Char(c, _) if *c == '<' || *c == '>' || *c == '=' => return Ok(*c),
                _ => return Err(TexError("Missing = inserted for \\ifnum".into())),
            }
        }
    }

    /// `\ifx` equality: same meaning, or the same character token.
    fn meanings_equal(&self, a: Option<&Token>, b: Option<&Token>) -> bool {
        match (a, b) {
            (Some(Token::Cs(x)), Some(Token::Cs(y))) => {
                match (self.meanings.get(x), self.meanings.get(y)) {
                    (None, None) => true,
                    (Some(mx), Some(my)) => mx == my,
                    _ => false,
                }
            }
            (Some(Token::Char(c1, k1)), Some(Token::Char(c2, k2))) => c1 == c2 && k1 == k2,
            _ => false,
        }
    }

    /// Expand until a character token appears, for `\if`'s code comparison.
    fn expand_one_char(&mut self, lx: &mut Lexer, pending_only: bool) -> R<char> {
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing token for \\if".into()));
            };
            match &t {
                Token::Char(c, _) => return Ok(*c),
                Token::Cs(n) => {
                    let n = *n;
                    if !self.try_expand(lx, n, pending_only)? {
                        // An unexpandable control sequence compares as itself.
                        return Ok('\u{0}');
                    }
                }
            }
        }
    }

    // ── definitions ──────────────────────────────────────────────────────

    /// `\def`, `\gdef`, `\edef`, `\xdef` — the last two expand the body now.
    fn do_def(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        let global = matches!(kind, "gdef" | "xdef") || self.global;
        let expand_body = matches!(kind, "edef" | "xdef");
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
        validate_params(&params)?;
        let raw = self.read_balanced(lx)?;
        let body = match expand_body {
            true => self.expand_to_tokens(lx, &raw)?,
            false => raw,
        };
        let was = std::mem::replace(&mut self.global, global);
        self.set_meaning(name, Meaning::Macro(Macro { params, body }));
        self.global = was;
        Ok(())
    }

    /// `\let\a=\b` — `\a` takes `\b`'s CURRENT meaning, not a reference to it.
    fn do_let(&mut self, lx: &mut Lexer) -> R<()> {
        let Some(Token::Cs(name)) = lx.next_token(&self.cats) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        // An optional `=` and one optional space, then the source token.
        let mut src = None;
        while let Some(t) = lx.next_token(&self.cats) {
            match &t {
                t if t.is_space() => continue,
                Token::Char('=', _) => continue,
                other => {
                    src = Some(*other);
                    break;
                }
            }
        }
        let Some(src) = src else {
            return Err(TexError("Missing token for \\let".into()));
        };
        let meaning = match &src {
            Token::Char(c, k) => Meaning::Char(*c, *k),
            Token::Cs(n) => match self.meanings.get(n) {
                Some(m) => m.clone(),
                None => Meaning::Primitive(*n),
            },
        };
        self.set_meaning(name, meaning);
        Ok(())
    }

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

    fn expand_macro(&mut self, lx: &mut Lexer, name: CsId, pending_only: bool) -> R<()> {
        let Some(Meaning::Macro(m)) = self.meanings.get(&name).cloned() else {
            return Err(TexError(format!("\\{} is not a macro", name.name())));
        };
        let args = self.match_params(lx, &m.params, pending_only)?;
        let mut out = Vec::with_capacity(m.body.len());
        let mut i = 0;
        while i < m.body.len() {
            match &m.body[i] {
                Token::Char(_, Cat::Param) => match m.body.get(i + 1) {
                    Some(Token::Char(d, _)) if d.is_ascii_digit() && *d != '0' => {
                        let idx = (*d as u8 - b'1') as usize;
                        if let Some(a) = args.get(idx) {
                            out.extend(a.iter().cloned());
                        }
                        i += 2;
                        continue;
                    }
                    Some(Token::Char(_, Cat::Param)) => {
                        out.push(m.body[i + 1]);
                        i += 2;
                        continue;
                    }
                    _ => out.push(m.body[i]),
                },
                t => out.push(*t),
            }
            i += 1;
        }
        lx.push_back(&out);
        Ok(())
    }

    fn match_params(
        &mut self,
        lx: &mut Lexer,
        params: &[Token],
        pending_only: bool,
    ) -> R<Vec<Vec<Token>>> {
        let mut args: Vec<Vec<Token>> = Vec::new();
        let mut i = 0;
        while i < params.len() {
            if !matches!(params[i], Token::Char(_, Cat::Param)) {
                let want = params[i];
                let got = self.take(lx, pending_only);
                if got.as_ref() != Some(&want) {
                    return Err(TexError("Use of macro doesn't match its definition".into()));
                }
                i += 1;
                continue;
            }
            let delim: Vec<Token> = params[i + 2..]
                .iter()
                .take_while(|t| !matches!(t, Token::Char(_, Cat::Param)))
                .cloned()
                .collect();
            let arg = match delim.is_empty() {
                true => self.read_undelimited(lx, pending_only)?,
                false => self.read_delimited(lx, &delim, pending_only)?,
            };
            args.push(arg);
            i += 2 + delim.len();
        }
        Ok(args)
    }

    fn read_undelimited(&mut self, lx: &mut Lexer, pending_only: bool) -> R<Vec<Token>> {
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError(
                    "Paragraph ended before argument was complete".into(),
                ));
            };
            if t.is_space() {
                continue;
            }
            return match t {
                Token::Char(_, Cat::BeginGroup) => self.read_balanced_from(lx, pending_only),
                other => Ok(vec![other]),
            };
        }
    }

    /// `read_balanced` for whichever source this context uses.
    fn read_balanced_from(&mut self, lx: &mut Lexer, pending_only: bool) -> R<Vec<Token>> {
        if !pending_only {
            return self.read_balanced(lx);
        }
        let mut depth = 1usize;
        let mut out = Vec::new();
        while let Some(t) = lx.pending.pop() {
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

    fn read_delimited(
        &mut self,
        lx: &mut Lexer,
        delim: &[Token],
        pending_only: bool,
    ) -> R<Vec<Token>> {
        let mut out: Vec<Token> = Vec::new();
        let mut depth = 0usize;
        loop {
            let Some(t) = self.take(lx, pending_only) else {
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
                let wrapped = out.len() >= 2
                    && matches!(out[0], Token::Char(_, Cat::BeginGroup))
                    && matches!(out[out.len() - 1], Token::Char(_, Cat::EndGroup));
                if wrapped {
                    out.remove(0);
                    out.pop();
                }
                return Ok(out);
            }
        }
    }

    // ── assignments ──────────────────────────────────────────────────────

    fn do_catcode(&mut self, lx: &mut Lexer) -> R<()> {
        let ch = self.scan_number(lx, false)?;
        self.skip_equals(lx)?;
        let val = self.scan_number(lx, false)?;
        let (Some(c), Some(cat)) = (char::from_u32(ch as u32), cat_from_i64(val)) else {
            return Err(TexError("Invalid code".into()));
        };
        self.set_cat(c, cat);
        Ok(())
    }

    fn do_count_assign(&mut self, lx: &mut Lexer) -> R<()> {
        let reg = self.scan_number(lx, false)?;
        self.skip_equals(lx)?;
        let val = self.scan_number(lx, false)?;
        self.set_count(reg, val);
        Ok(())
    }

    fn do_arith(&mut self, lx: &mut Lexer, op: Arith) -> R<()> {
        let Some(Token::Cs(what)) = lx.next_token(&self.cats) else {
            return Err(TexError("You can't use this after \\advance".into()));
        };
        if what.name() != "count" {
            return Err(TexError(format!("Unsupported register \\{}", what.name())));
        }
        let reg = self.scan_number(lx, false)?;
        self.skip_by(lx)?;
        let val = self.scan_number(lx, false)?;
        let cur = *self.count.get(&reg).unwrap_or(&0);
        let next = match op {
            Arith::Add => cur + val,
            Arith::Mul => cur * val,
            Arith::Div => match val {
                0 => return Err(TexError("Arithmetic overflow".into())),
                d => cur / d,
            },
        };
        self.set_count(reg, next);
        Ok(())
    }

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
                    lx.push_back(std::slice::from_ref(other));
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn scan_number(&mut self, lx: &mut Lexer, pending_only: bool) -> R<i64> {
        let mut sign = 1i64;
        let mut cur = loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            match &t {
                t if t.is_space() => continue,
                Token::Char('-', _) => {
                    sign = -sign;
                    continue;
                }
                Token::Char('+', _) => continue,
                other => break *other,
            }
        };
        if matches!(cur, Token::Char('`', _)) {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing number".into()));
            };
            let code = match t {
                Token::Char(c, _) => u32::from(c) as i64,
                Token::Cs(n) => n
                    .name()
                    .chars()
                    .next()
                    .map(|c| u32::from(c) as i64)
                    .unwrap_or(0),
            };
            return Ok(sign * code);
        }
        if let Token::Cs(name) = &cur {
            let name = *name;
            if name.name() == "count" {
                let reg = self.scan_number(lx, pending_only)?;
                return Ok(sign * *self.count.get(&reg).unwrap_or(&0));
            }
            // A macro in numeric position expands and the scan resumes.
            if self.try_expand(lx, name, pending_only)? {
                return Ok(sign * self.scan_number(lx, pending_only)?);
            }
            return Err(TexError(format!("Missing number, found \\{}", name.name())));
        }
        let mut digits = String::new();
        loop {
            match &cur {
                Token::Char(c, _) if c.is_ascii_digit() => digits.push(*c),
                // A constant is terminated by ONE optional space, which is
                // ABSORBED (tex.web §444) -- it delimits the number and is not
                // part of what follows. Pushing it back put it in the text, so
                // `\ifnum\count0>3 BIG` rendered as " BIG".
                other if other.is_space() && !digits.is_empty() => break,
                other => {
                    lx.push_back(std::slice::from_ref(other));
                    break;
                }
            }
            match self.take(lx, pending_only) {
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

    // ── text production ──────────────────────────────────────────────────

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

    /// Fully expand a token list to the tokens it produces.
    fn expand_to_tokens(&mut self, lx: &mut Lexer, toks: &[Token]) -> R<Vec<Token>> {
        let saved: Vec<Token> = std::mem::take(&mut lx.pending);
        lx.push_back(toks);
        let mut out = Vec::new();
        let mut steps = 0usize;
        while let Some(t) = lx.pending.pop() {
            steps += 1;
            if steps > 200_000 {
                return Err(TexError("TeX capacity exceeded".into()));
            }
            match &t {
                Token::Cs(name) => {
                    let name = *name;
                    if !self.try_expand(lx, name, true)? {
                        out.push(t);
                    }
                }
                _ => out.push(t),
            }
        }
        lx.pending = saved;
        Ok(out)
    }

    /// Fully expand a token list and render it, for `\message`.
    fn expand_to_text(&mut self, lx: &mut Lexer, toks: &[Token]) -> R<String> {
        let saved: Vec<Token> = std::mem::take(&mut lx.pending);
        lx.push_back(toks);
        let mut out = String::new();
        let mut steps = 0usize;
        while let Some(t) = lx.pending.pop() {
            steps += 1;
            if steps > 200_000 {
                return Err(TexError("TeX capacity exceeded".into()));
            }
            match &t {
                Token::Cs(name) if name.name() == "the" || name.name() == "number" => {
                    let n = self.read_the(lx, name.name() == "number")?;
                    out.push_str(&n);
                }
                Token::Cs(name) if name.name() == "string" => {
                    // `\string` is `sprint_cs` (tex.web §262), NOT `print_cs`:
                    // no trailing space after a multi-letter name.
                    if let Some(next) = lx.pending.pop() {
                        out.push_str(&match &next {
                            Token::Cs(n) => format!("{}{}", self.escape, n.name()),
                            other => other.to_text(self.escape),
                        });
                    }
                }
                Token::Cs(name) => {
                    let name = *name;
                    if !self.try_expand(lx, name, true)? {
                        out.push_str(&t.to_text(self.escape));
                    }
                }
                other => out.push_str(&other.to_text(self.escape)),
            }
        }
        lx.pending = saved;
        Ok(out)
    }

    /// `\the\count<n>`, and `\number<n>` which takes a bare number.
    fn read_the(&mut self, lx: &mut Lexer, bare: bool) -> R<String> {
        if bare {
            return Ok(self.scan_number(lx, true)?.to_string());
        }
        let Some(Token::Cs(what)) = lx.pending.pop() else {
            return Err(TexError("You can't use \\the here".into()));
        };
        if what.name() != "count" {
            return Err(TexError(format!("Unsupported \\the\\{}", what.name())));
        }
        let reg = self.scan_number(lx, true)?;
        Ok(self.count.get(&reg).unwrap_or(&0).to_string())
    }

    fn read_csname(&mut self, lx: &mut Lexer, pending_only: bool) -> R<String> {
        let mut name = String::new();
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing \\endcsname inserted".into()));
            };
            match &t {
                Token::Cs(n) if n.name() == "endcsname" => return Ok(name),
                Token::Char(c, _) => name.push(*c),
                Token::Cs(n) => {
                    let n = *n;
                    if !self.try_expand(lx, n, pending_only)? {
                        return Err(TexError(format!(
                            "Missing \\endcsname before \\{}",
                            n.name()
                        )));
                    }
                }
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

/// The frontend surface `crate::lower` drives.
///
/// Lowering needs the expander's compile-time half — the macro table, the
/// catcode table, and the scanners — without its executor. These are thin,
/// deliberately named wrappers so the lowering pass reads as a compiler rather
/// than as a second interpreter reaching into private state.
impl Engine {
    /// `\def`/`\gdef` at compile time: it changes how the rest of the file reads.
    pub fn compile_time_def(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        self.do_def(lx, kind)
    }

    /// `\catcode` at compile time, for the same reason.
    pub fn compile_time_catcode(&mut self, lx: &mut Lexer) -> R<()> {
        self.do_catcode(lx)
    }

    pub fn scan_number_file(&mut self, lx: &mut Lexer) -> R<i64> {
        self.scan_number(lx, false)
    }

    pub fn scan_number_pending(&mut self, lx: &mut Lexer) -> R<i64> {
        self.scan_number(lx, true)
    }

    pub fn skip_equals_file(&mut self, lx: &mut Lexer) -> R<()> {
        self.skip_equals(lx)
    }

    pub fn skip_by_file(&mut self, lx: &mut Lexer) -> R<()> {
        self.skip_by(lx)
    }

    pub fn read_relation_file(&mut self, lx: &mut Lexer) -> R<char> {
        self.read_relation(lx, false)
    }

    pub fn expand_macro_file(&mut self, lx: &mut Lexer, name: CsId) -> R<()> {
        self.expand_macro(lx, name, false)
    }

    pub fn expand_macro_pending(&mut self, lx: &mut Lexer, name: CsId) -> R<()> {
        self.expand_macro(lx, name, true)
    }

    /// Read `{...}` after `\message`, unexpanded — the pieces are split by the
    /// lowering pass, which has to keep `\the` as a run-time read.
    pub fn read_message_body(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
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
        self.read_balanced(lx)
    }
}

/// Compile-time control the lowering pass needs: grouping and `\let`, both of
/// which act on the macro table and so belong to the frontend, not the VM.
impl Engine {
    pub fn compile_time_let(&mut self, lx: &mut Lexer) -> R<()> {
        self.do_let(lx)
    }
    pub fn compile_time_begin_group(&mut self) {
        self.begin_group();
    }
    pub fn compile_time_end_group(&mut self) -> R<()> {
        self.end_group()
    }
    pub fn set_global_prefix(&mut self, on: bool) {
        self.global = on;
    }
    /// `\ifx` equality over the CURRENT meanings — decidable while lowering,
    /// because a macro's meaning is a frontend fact and not VM state.
    pub fn ifx_equal(&mut self, lx: &mut Lexer) -> R<bool> {
        let a = lx.next_token(&self.cats);
        let b = lx.next_token(&self.cats);
        Ok(self.meanings_equal(a.as_ref(), b.as_ref()))
    }
    pub fn take_file(&mut self, lx: &mut Lexer) -> Option<Token> {
        lx.next_token(&self.cats)
    }
}

/// More frontend surface for the lowering pass.
impl Engine {
    pub fn is_macro(&self, name: CsId) -> bool {
        matches!(self.meanings.get(&name), Some(Meaning::Macro(_)))
    }
    pub fn read_balanced_pub(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        self.read_balanced(lx)
    }
    pub fn read_csname_pending(&mut self, lx: &mut Lexer) -> R<String> {
        self.read_csname(lx, true)
    }
    pub fn read_relation_pending(&mut self, lx: &mut Lexer) -> R<char> {
        self.read_relation(lx, true)
    }
    pub fn meanings_equal_pub(&self, a: Option<&Token>, b: Option<&Token>) -> bool {
        self.meanings_equal(a, b)
    }
    /// Define a macro with no parameters, for `\edef`'s rewritten body.
    pub fn define_macro(&mut self, name: CsId, body: Vec<Token>) {
        self.set_meaning(
            name,
            Meaning::Macro(Macro {
                params: Vec::new(),
                body,
            }),
        );
    }
}

/// Check a macro's parameter text the way `tex.web` §476 does.
///
/// Every `#` must be followed by a digit, and the digits must run 1, 2, 3 ... in
/// order. TeX has one exception -- `#{`, a parameter delimited by the left brace
/// -- which this milestone does not implement and therefore refuses rather than
/// mis-parses.
///
/// Without this the argument reader walks off the end of the parameter list on a
/// trailing `#`: `\def\a#{...}` panicked with `range start index 2 out of range`
/// (found by `cargo fuzz run lower`).
fn validate_params(params: &[Token]) -> R<()> {
    let mut expect = b'1';
    let mut i = 0;
    while i < params.len() {
        if !matches!(params[i], Token::Char(_, Cat::Param)) {
            i += 1;
            continue;
        }
        match params.get(i + 1) {
            Some(Token::Char(c, _)) if *c as u8 == expect => {
                expect += 1;
                i += 2;
            }
            Some(Token::Char(c, _)) if c.is_ascii_digit() => {
                return Err(TexError("Parameters must be numbered consecutively".into()))
            }
            // `#` last in the parameter text is TeX's `#{` form.
            None => return Err(TexError("`#{` parameter text is not implemented".into())),
            _ => return Err(TexError("Illegal parameter number in definition".into())),
        }
    }
    Ok(())
}
