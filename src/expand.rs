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
    /// `\long\def` — the argument may contain `\par`.
    pub long: bool,
    /// `\protected\def` — the macro does not expand inside an `\edef` or a
    /// `\write`; it survives as itself and runs when the result does.
    pub protected: bool,
    /// `\outer\def` — the macro may not appear in an argument, in a group
    /// being scanned as text, or in skipped conditional text.
    ///
    /// Both flags are part of the MEANING, not decoration: tex's `\ifx` says
    /// `\long\def\a{}` and `\def\a{}` differ, which falls out of deriving
    /// `PartialEq` here.
    pub outer: bool,
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
    /// `\chardef\a=65` — a character code, standing for a number wherever one
    /// is scanned. plain.tex builds its constants this way: `\chardef\active=13`
    /// is what makes `\catcode`\~=\active` readable.
    CharDef(i64),
    /// `\toksdef\toks@=0` — another name for a token register.
    ToksDef(i64),
    /// `\countdef\pageno=0` — another name for a count register, in every
    /// position `\count0` itself works: assignment, arithmetic and `\the`.
    CountDef(i64),
}

/// eTeX's division: round half AWAY from zero, which is not what `\divide`
/// does (`tex.web` §1236 truncates). Measured -- 7/2 is 4, 5/2 is 3, -7/2 is
/// -4, and 6/4 is 2.
fn round_div(a: i64, b: i64) -> i64 {
    let sign = match (a < 0) == (b < 0) {
        true => 1,
        false => -1,
    };
    let (a, b) = (a.abs(), b.abs());
    sign * ((a + b / 2) / b)
}

/// What a control sequence contributes when a number is being scanned.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NumericCs {
    /// A constant, already known while lowering.
    Value(i64),
    /// A count register, whose value is only known at run time.
    Register(i64),
}

/// One undo record. TeX's save stack restores individual changes at group end
/// rather than snapshotting the whole state, which is what makes deep grouping
/// affordable; the same choice is made here.
enum Save {
    Cat(char, Cat),
    /// One token register as it stood before a group assigned to it.
    Toks(i64, Option<Vec<Token>>),
    /// One entry of one character-code table, as it stood before a group
    /// changed it. The tables are scoped exactly as the category codes are --
    /// measured: 777 inside the group, 555 outside.
    CharCode(crate::charcodes::Table, char, i64),
    Count(i64, Option<i64>),
    Meaning(CsId, Option<Meaning>),
    /// The whole intercept registry as it stood before a registration inside
    /// this group. Advice is a document-wide effect, and TeX's rule is that a
    /// non-global assignment does not escape its group; the registry is small
    /// and registrations are rare, so the undo record is the whole list rather
    /// than a per-entry diff.
    Intercepts(crate::intercepts::Registry),
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
    /// Default values recorded by `\newcommand{\x}[n][default]`.
    optional_defaults: HashMap<CsId, Vec<Token>>,
    pub count: HashMap<i64, i64>,
    pub messages: Vec<String>,
    pub escape: char,
    /// One frame per open group; each holds the undo records for that group.
    groups: Vec<Vec<Save>>,
    /// Open conditionals, so `\else`/`\fi` know what they close.
    conds: Vec<CondState>,
    /// Set by `\global`, cleared by the assignment it prefixes.
    /// `\mathcode`, `\lccode`, `\uccode`, `\sfcode`, `\delcode`.
    pub charcodes: crate::charcodes::CharCodes,
    /// `\toks` registers. A token list is frontend state like a macro body, not
    /// a number in a slot, so it lives here and is scoped by the same save
    /// stack -- measured: `{\toks0={b}}` leaves the outer value behind.
    pub toks: std::collections::HashMap<i64, Vec<Token>>,
    global: bool,
    /// Whether a `\long` prefix is in force for the definition being read.
    long: bool,
    /// Whether an `\outer` prefix is.
    outer: bool,
    /// Whether a `\protected` prefix is.
    protected: bool,
    /// Advice registered with `\intercept`, woven into matching expansions.
    pub intercepts: crate::intercepts::Registry,
    /// How deep inside an advice body expansion currently is.
    ///
    /// Advice is NOT woven into a call that occurs inside advice: a handler
    /// that calls the macro it advises would otherwise weave itself forever.
    /// The depth is carried in the token stream by the two markers below rather
    /// than by a counter around the weave, because the handler's tokens are
    /// pushed back and expanded later, long after the weave returned.
    advice_depth: u32,
}

/// An advice body between the two depth markers, so a call inside it is not
/// itself advised.
fn wrapped(body: &[Token]) -> Vec<Token> {
    let mut v = Vec::with_capacity(body.len() + 2);
    v.push(Token::cs(ADVICE_IN));
    v.extend(body.iter().copied());
    v.push(Token::cs(ADVICE_OUT));
    v
}

/// Pushed in front of an advice body, and behind it. The NUL keeps both out of
/// reach of any document: a control sequence's name comes from the mouth, and
/// the mouth cannot produce one.
pub const ADVICE_IN: &str = "\u{0}advice-in";
pub const ADVICE_OUT: &str = "\u{0}advice-out";

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
            optional_defaults: HashMap::new(),
            count: HashMap::new(),
            messages: Vec::new(),
            escape: '\\',
            groups: Vec::new(),
            conds: Vec::new(),
            charcodes: crate::charcodes::CharCodes::default(),
            toks: std::collections::HashMap::new(),
            global: false,
            long: false,
            outer: false,
            protected: false,
            intercepts: crate::intercepts::Registry::new(),
            advice_depth: 0,
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
                Save::Toks(reg, was) => {
                    match was {
                        Some(v) => self.toks.insert(reg, v),
                        None => self.toks.remove(&reg),
                    };
                }
                Save::CharCode(t, c, v) => {
                    let _ = self.charcodes.set(t, c, v);
                }
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
                Save::Intercepts(old) => self.intercepts = old,
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
            "newcommand" | "renewcommand" | "providecommand" | "DeclareRobustCommand" => {
                self.do_newcommand(lx, name.name())?
            }
            "let" => self.do_let(lx)?,
            "futurelet" => self.do_futurelet(lx, false)?,
            // `\long` and `\outer` take a prefix each, exactly as `\global`
            // does, and chain in any order: `\global\outer\long\def` is one
            // definition with three of them.
            "long" | "outer" | "protected" => {
                let which = name.name();
                match which {
                    "long" => self.long = true,
                    "outer" => self.outer = true,
                    _ => self.protected = true,
                }
                let Some(next) = lx.next_token(&self.cats) else {
                    return Err(TexError("Missing control sequence".into()));
                };
                let out = self.step(lx, next);
                match which {
                    "long" => self.long = false,
                    "outer" => self.outer = false,
                    _ => self.protected = false,
                }
                return out;
            }
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
            "mathcode" | "lccode" | "uccode" | "sfcode" | "delcode" => {
                let t = crate::charcodes::Table::from_name(name.name())
                    .expect("one of the five just matched");
                self.do_charcode(lx, t)?
            }
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
        // A `\let` alias MEANS the primitive it was given, so it must act like
        // one here too. The dispatch below is by NAME, so without this an alias
        // is matched as itself: `\let\ifdim=\iffalse` still reached the
        // \ifdim arm and reported an unsupported conditional, which is how a
        // document says "I know this engine has no dimensions, take the other
        // branch" and was refused anyway.
        let name = match self.meanings.get(&name) {
            Some(Meaning::Primitive(p)) if *p != name => *p,
            _ => name,
        };
        match name.name() {
            // The advice markers: they carry the "inside advice" depth through
            // the token stream, and expand to nothing.
            ADVICE_IN | ADVICE_OUT => {
                self.advice_marker(name.name());
                Ok(true)
            }
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
            // `\string` is expandable and reaches running text, not only the
            // inside of a `\message`. tex.web 262 has the expander produce the
            // characters of the next token wherever it appears; handling it only
            // in message context left it undefined in the body of a document.
            "string" => {
                if let Some(t) = self.take(lx, pending_only) {
                    let text = match &t {
                        Token::Cs(n) => format!("{}{}", self.escape, n.name()),
                        other => other.to_text(self.escape),
                    };
                    let toks: Vec<Token> =
                        text.chars().map(|c| Token::Char(c, Cat::Other)).collect();
                    lx.push_back(&toks);
                }
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
                // Expand in the SAME context `\expandafter` was read in. Forcing
                // pending-only here meant the one-step expansion could not reach
                // the file, so `\expandafter\def\csname foo\endcsname{...}` at
                // the top of a document had `\csname` scanning an empty pending
                // list and reporting a missing `\endcsname` that was sitting in
                // the source a token away.
                if !self.try_expand(lx, n, pending_only)? {
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
            // A control sequence `\let` (or `\futurelet`) to a character token
            // MEANS that character, so it compares equal to it -- `tex.web` §507
            // compares meanings, and the meaning of `\next` after
            // `\futurelet\next` over an `A` is the character A. Without this,
            // `\ifx\next[` is always false, and that single comparison is what
            // LaTeX's `\@ifnextchar` is, hence every optional argument in the
            // language.
            (Some(Token::Cs(x)), Some(Token::Char(c, k)))
            | (Some(Token::Char(c, k)), Some(Token::Cs(x))) => {
                matches!(self.meanings.get(x), Some(Meaning::Char(mc, mk)) if mc == c && mk == k)
            }
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
    /// A dimension: an optional sign, a decimal number, and a unit
    /// (`tex.web` §448-453). The result is scaled points, which is the only
    /// form a dimension ever has -- `pt` is not privileged, it is just the unit
    /// whose ratio is one.
    /// The number in front of a unit: a sign, an integer part, and a fraction
    /// already scaled to 65536ths. Shared by the finite scanner and the one
    /// that also accepts `fil`, so the two cannot drift.
    fn scan_dimen_number(&mut self, lx: &mut Lexer, pending_only: bool) -> R<(i64, i64, i64)> {
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
        // The integer part, then an optional fraction after `.` or `,`.
        let mut whole = String::new();
        let mut fraction = String::new();
        let mut seen_point = false;
        loop {
            match &cur {
                Token::Char(c, _) if c.is_ascii_digit() => match seen_point {
                    true => fraction.push(*c),
                    false => whole.push(*c),
                },
                Token::Char('.' | ',', _) if !seen_point => seen_point = true,
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
        let int: i64 = whole.parse().unwrap_or(0);
        Ok((sign, int, crate::dimen::round_decimals(&fraction)))
    }

    /// A dimension: the number, then a unit (`tex.web` §448-453). The result is
    /// scaled points, which is the only form a dimension ever has.
    pub fn scan_dimen(&mut self, lx: &mut Lexer, pending_only: bool) -> R<i64> {
        // `\dimexpr ...\relax` stands where a dimension does.
        if let Some(t) = self.take(lx, pending_only) {
            let is_expr = matches!(&t, Token::Cs(n) if n.name() == "dimexpr");
            lx.push_back(std::slice::from_ref(&t));
            if is_expr {
                let _ = self.take(lx, pending_only);
                return self.scan_expr(lx, pending_only, true);
            }
        }
        let (sign, int, frac) = self.scan_dimen_number(lx, pending_only)?;
        let mut unit = String::new();
        while unit.len() < 2 {
            let Some(t) = self.take(lx, pending_only) else {
                break;
            };
            match &t {
                t if t.is_space() && unit.is_empty() => continue,
                Token::Char(c, _) if c.is_ascii_alphabetic() => unit.push(c.to_ascii_lowercase()),
                other => {
                    lx.push_back(std::slice::from_ref(other));
                    break;
                }
            }
        }
        let Some(sp) = crate::dimen::to_scaled(int, frac, &unit) else {
            return Err(TexError("Illegal unit of measure (pt inserted)".into()));
        };
        // One optional space is absorbed after a unit, as after a constant.
        if let Some(t) = self.take(lx, pending_only) {
            if !t.is_space() {
                lx.push_back(std::slice::from_ref(&t));
            }
        }
        Ok((sign * sp).clamp(-crate::dimen::MAX_DIMEN, crate::dimen::MAX_DIMEN))
    }

    /// `\toks<n>={...}` or `\toks<n>=\toks<m>`.
    ///
    /// The braced form is stored VERBATIM: nothing in it expands, which is the
    /// difference between a token register and a macro. Measured --
    /// `\toks0={\x}` reads back as `\x`, whatever `\x` means.
    pub fn do_toks_assign(&mut self, lx: &mut Lexer, reg: i64) -> R<()> {
        self.skip_equals(lx)?;
        let value = loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError("Missing { inserted".into()));
            };
            match &t {
                t if t.is_space() => continue,
                Token::Char(_, Cat::BeginGroup) => break self.read_balanced(lx)?,
                // `\toks1=\toks0` copies, and `\toks1=\toksA` copies through a
                // name defined by \toksdef.
                Token::Cs(n) => {
                    let from = match n.name() {
                        "toks" => self.scan_number(lx, false)?,
                        _ => match self.meanings.get(n) {
                            Some(Meaning::ToksDef(r)) => *r,
                            _ => return Err(TexError("Missing { inserted".into())),
                        },
                    };
                    break self.toks.get(&from).cloned().unwrap_or_default();
                }
                _ => return Err(TexError("Missing { inserted".into())),
            }
        };
        self.save(Save::Toks(reg, self.toks.get(&reg).cloned()));
        self.toks.insert(reg, value);
        Ok(())
    }

    /// What `\the\toks<n>` writes: the tokens as text, by the same rule
    /// `\string` uses -- a control word carries a trailing space, a
    /// single-character control sequence does not.
    /// A token list as text, by the token-list rule: a control WORD carries a
    /// trailing space, a one-character control sequence does not. Shared by
    /// `\the\toks` and `\detokenize`, which write the same way.
    pub fn tokens_text(&self, tokens: &[Token]) -> String {
        let mut out = String::new();
        for t in tokens {
            match t {
                Token::Char(c, _) => out.push(*c),
                Token::Cs(id) => {
                    let name = id.name();
                    out.push(self.escape);
                    out.push_str(name);
                    if name.chars().all(|c| c.is_alphabetic()) {
                        out.push(' ');
                    }
                }
            }
        }
        out
    }

    pub fn toks_text(&self, reg: i64) -> String {
        let Some(tokens) = self.toks.get(&reg) else {
            return String::new();
        };
        let mut out = String::new();
        for t in tokens {
            match t {
                Token::Char(c, _) => out.push(*c),
                Token::Cs(id) => {
                    let name = id.name();
                    out.push(self.escape);
                    out.push_str(name);
                    // A control WORD carries a trailing space here, however
                    // short: `\b` prints as `\b `. A control sequence made of
                    // one non-letter does not. Measured -- and it is not the
                    // rule `\string` follows, which never adds the space, so
                    // the two cannot share a renderer.
                    if name.chars().all(|c| c.is_alphabetic()) {
                        out.push(' ');
                    }
                }
            }
        }
        out
    }

    /// The register a `\toksdef` name stands for.
    pub fn toks_cs(&self, name: CsId) -> Option<i64> {
        match self.meanings.get(&name) {
            Some(Meaning::ToksDef(r)) => Some(*r),
            _ => None,
        }
    }

    /// eTeX's `\numexpr` and `\dimexpr`, evaluated on the spot.
    ///
    /// Ordinary precedence, parentheses, and left to right within a level. The
    /// one rule worth stating is division: it ROUNDS, half away from zero, so
    /// `7/2` is 4 and `-7/2` is -4 -- which is not what `\divide` does, and is
    /// why the two cannot share an implementation. Measured against
    /// LuaTeX 1.24.0.
    pub fn scan_expr(&mut self, lx: &mut Lexer, pending_only: bool, dimen: bool) -> R<i64> {
        let v = self.expr_sum(lx, pending_only, dimen)?;
        // An expression is closed by an optional `\relax`, which is absorbed.
        if let Some(t) = self.take(lx, pending_only) {
            match &t {
                Token::Cs(n) if n.name() == "relax" => {}
                other => lx.push_back(std::slice::from_ref(other)),
            }
        }
        Ok(v)
    }

    fn expr_sum(&mut self, lx: &mut Lexer, pending_only: bool, dimen: bool) -> R<i64> {
        let mut acc = self.expr_term(lx, pending_only, dimen)?;
        loop {
            match self.expr_operator(lx, pending_only, &['+', '-']) {
                Some('+') => acc += self.expr_term(lx, pending_only, dimen)?,
                Some('-') => acc -= self.expr_term(lx, pending_only, dimen)?,
                _ => return Ok(acc),
            }
        }
    }

    fn expr_term(&mut self, lx: &mut Lexer, pending_only: bool, dimen: bool) -> R<i64> {
        let mut acc = self.expr_factor(lx, pending_only, dimen)?;
        loop {
            match self.expr_operator(lx, pending_only, &['*', '/']) {
                // A multiplier and a divisor are integers even in a dimension
                // expression: `1pt*3` is three points, not a square.
                Some('*') => acc *= self.expr_factor(lx, pending_only, false)?,
                Some('/') => {
                    let by = self.expr_factor(lx, pending_only, false)?;
                    if by == 0 {
                        return Err(TexError("Arithmetic overflow".into()));
                    }
                    acc = round_div(acc, by);
                }
                _ => return Ok(acc),
            }
        }
    }

    fn expr_factor(&mut self, lx: &mut Lexer, pending_only: bool, dimen: bool) -> R<i64> {
        // Skip spaces to see whether a group is opening.
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Err(TexError("Missing number, treated as zero".into()));
            };
            match &t {
                t if t.is_space() => continue,
                Token::Char('(', _) => {
                    let v = self.expr_sum(lx, pending_only, dimen)?;
                    match self.take(lx, pending_only) {
                        Some(Token::Char(')', _)) => return Ok(v),
                        _ => return Err(TexError("Missing ) inserted".into())),
                    }
                }
                other => {
                    lx.push_back(std::slice::from_ref(other));
                    return match dimen {
                        true => self.scan_dimen(lx, pending_only),
                        false => self.scan_number(lx, pending_only),
                    };
                }
            }
        }
    }

    /// The next operator, if the next non-space token is one of `wanted`.
    fn expr_operator(
        &mut self,
        lx: &mut Lexer,
        pending_only: bool,
        wanted: &[char],
    ) -> Option<char> {
        loop {
            let t = self.take(lx, pending_only)?;
            if t.is_space() {
                continue;
            }
            if let Token::Char(c, _) = &t {
                if wanted.contains(c) {
                    return Some(*c);
                }
            }
            lx.push_back(std::slice::from_ref(&t));
            return None;
        }
    }

    /// A dimension that may be infinite: the same number, but with `fil`,
    /// `fill` or `filll` accepted as its unit. Returns the value and its order,
    /// 0 being an ordinary finite dimension.
    pub fn scan_dimen_or_fil(&mut self, lx: &mut Lexer, pending_only: bool) -> R<(i64, i64)> {
        let (sign, int, frac) = self.scan_dimen_number(lx, pending_only)?;
        // Up to five letters, because `filll` is five; whatever is not part of
        // the unit goes back, so `1pt x` still leaves the `x` in the document.
        let mut letters = String::new();
        let mut extra = Vec::new();
        while letters.len() < 5 {
            let Some(t) = self.take(lx, pending_only) else {
                break;
            };
            match &t {
                t if t.is_space() && letters.is_empty() => continue,
                Token::Char(c, _) if c.is_ascii_alphabetic() => {
                    letters.push(c.to_ascii_lowercase());
                    extra.push(t);
                }
                other => {
                    lx.push_back(std::slice::from_ref(other));
                    break;
                }
            }
        }
        // Longest match wins, so `filll` is not read as `fil` with two letters
        // left over.
        let mut order = 0;
        let mut taken = 0;
        for n in (2..=letters.len()).rev() {
            let candidate = &letters[..n];
            if let Some(o) = crate::glue::order_of(candidate) {
                order = o;
                taken = n;
                break;
            }
            if crate::dimen::unit_ratio(candidate).is_some() && n == 2 {
                taken = 2;
                break;
            }
        }
        if taken == 0 {
            return Err(TexError("Illegal unit of measure (pt inserted)".into()));
        }
        // Letters past the unit were never part of it.
        for t in extra[taken..].iter().rev() {
            lx.push_back(std::slice::from_ref(t));
        }
        let unit = &letters[..taken];
        let sp = match order {
            // An infinite component's number is not converted: `1fil` is one,
            // in the same 65536ths a point uses, at a different order.
            0 => crate::dimen::to_scaled(int, frac, unit)
                .ok_or_else(|| TexError("Illegal unit of measure (pt inserted)".into()))?,
            _ => int * crate::dimen::UNITY + frac,
        };
        if let Some(t) = self.take(lx, pending_only) {
            if !t.is_space() {
                lx.push_back(std::slice::from_ref(&t));
            }
        }
        Ok((
            (sign * sp).clamp(-crate::dimen::MAX_DIMEN, crate::dimen::MAX_DIMEN),
            order,
        ))
    }

    /// Whether the next tokens spell `word`, consuming them if they do.
    ///
    /// TeX's keyword scan (`tex.web` §407): letters match either case, and
    /// nothing is consumed when the word is not there.
    pub fn scan_keyword(&mut self, lx: &mut Lexer, word: &str, pending_only: bool) -> bool {
        let mut seen = Vec::new();
        for want in word.chars() {
            loop {
                let Some(t) = self.take(lx, pending_only) else {
                    for t in seen.iter().rev() {
                        lx.push_back(std::slice::from_ref(t));
                    }
                    return false;
                };
                if t.is_space() && seen.is_empty() {
                    continue;
                }
                let matched = matches!(&t, Token::Char(c, _)
                    if c.eq_ignore_ascii_case(&want));
                seen.push(t);
                if !matched {
                    for t in seen.iter().rev() {
                        lx.push_back(std::slice::from_ref(t));
                    }
                    return false;
                }
                break;
            }
        }
        true
    }

    /// `<dimen> [plus <dimen|fil>] [minus <dimen|fil>]` — a glue.
    pub fn scan_glue(&mut self, lx: &mut Lexer) -> R<(i64, i64, i64, i64, i64)> {
        let natural = self.scan_dimen(lx, false)?;
        let (stretch, stretch_order) = match self.scan_keyword(lx, "plus", false) {
            true => self.scan_dimen_or_fil(lx, false)?,
            false => (0, 0),
        };
        let (shrink, shrink_order) = match self.scan_keyword(lx, "minus", false) {
            true => self.scan_dimen_or_fil(lx, false)?,
            false => (0, 0),
        };
        Ok((natural, stretch, stretch_order, shrink, shrink_order))
    }

    /// The same, for a caller that is lowering.
    pub fn scan_dimen_file(&mut self, lx: &mut Lexer) -> R<i64> {
        self.scan_dimen(lx, false)
    }

    /// The control sequence an ACTIVE CHARACTER stands for.
    ///
    /// An active `~` and the control sequence `\~` are different things in TeX
    /// -- `\def~{[a]}\def\~{[b]}` gives `[a][b]`, measured -- so they cannot
    /// share a name. A name the lexer produces is either a run of letters or
    /// exactly one character, so `active:~` is a name no source can spell, and
    /// interning under it keeps the two apart with no second table.
    pub fn active_cs(c: char) -> CsId {
        CsId::intern(&format!("active:{c}"))
    }

    /// The control sequence an active character stands for, if it HAS a
    /// meaning.
    ///
    /// One with no meaning is left alone rather than reported: an undefined
    /// control sequence is not an error in this engine either (see BUGS.md), so
    /// an undefined active character behaves the same way instead of being the
    /// one construct that stops a run.
    pub fn active_meaning(&self, c: char) -> Option<CsId> {
        let id = Self::active_cs(c);
        self.meanings.contains_key(&id).then_some(id)
    }

    /// The token a definition names: a control sequence, or an active
    /// character, which TeX's `get_r_token` accepts in the same position
    /// (`tex.web` §1215).
    fn scan_defined_name(&mut self, lx: &mut Lexer) -> R<CsId> {
        match lx.next_token(&self.cats) {
            Some(Token::Cs(name)) => Ok(name),
            Some(Token::Char(c, Cat::Active)) => Ok(Self::active_cs(c)),
            _ => Err(TexError("Missing control sequence inserted".into())),
        }
    }

    fn do_def(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        let global = matches!(kind, "gdef" | "xdef") || self.global;
        let expand_body = matches!(kind, "edef" | "xdef");
        let name = self.scan_defined_name(lx)?;
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
        self.set_meaning(name, Meaning::Macro(self.new_macro(params, body)));
        self.global = was;
        // A prefix applies to the definition it precedes and to nothing after
        // it, so it is spent here rather than left to colour the next `\def`.
        self.long = false;
        self.outer = false;
        self.protected = false;
        Ok(())
    }

    /// `\newcommand{\x}{body}`, `\newcommand{\x}[n]{body}`,
    /// `\newcommand{\x}[n][default]{body}`.
    ///
    /// LaTeX writes this in TeX, as a chain of `\ifnum...\def...\else` that
    /// dispatches on the argument count. That chain cannot run here: a `\def`
    /// inside an arm of a run-time conditional is executed while lowering, and
    /// lowering emits BOTH arms, so the losing branch's definition wins
    /// (`tests/cases/def_in_conditional_arm.tex`). Doing the dispatch natively
    /// sidesteps it entirely, and the observable behaviour is the same one
    /// latex.ltx specifies — which is what a port has to preserve, not the
    /// particular macros it was written with.
    ///
    /// The optional-argument form defines a macro whose FIRST parameter has a
    /// default. TeX has no such thing, so it is built the way LaTeX builds it:
    /// the macro takes `n` ordinary parameters and a separate `\\x` is not
    /// created; a call that omits the bracket gets the default substituted.
    fn do_newcommand(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        // The name, either bare (`\newcommand\x`) or braced (`\newcommand{\x}`).
        let Some(name) = self.scan_command_name(lx)? else {
            // Nothing definable was found; consume what a definition would have
            // taken so the arguments do not land in the document as text.
            let _ = self.scan_optional_bracket(lx)?;
            let _ = self.scan_optional_bracket(lx)?;
            self.skip_spaces(lx);
            let _ = self.read_group_tokens(lx)?;
            return Ok(());
        };
        if kind == "providecommand" && self.meanings.contains_key(&name) {
            // `\providecommand` leaves an existing definition alone; the body
            // still has to be consumed or it lands in the document as text.
            let _ = self.scan_optional_bracket(lx)?;
            let _ = self.scan_optional_bracket(lx)?;
            self.skip_spaces(lx);
            let _ = self.read_group_tokens(lx)?;
            return Ok(());
        }
        let argc = match self.scan_optional_bracket(lx)? {
            Some(toks) => {
                let text: String = toks.iter().map(|t| t.to_text(self.escape)).collect();
                text.trim().parse::<usize>().unwrap_or(0).min(9)
            }
            None => 0,
        };
        // A second bracket is the default for the first argument.
        let default = self.scan_optional_bracket(lx)?;
        self.skip_spaces(lx);
        let body = self.read_group_tokens(lx)?;

        let mut params = Vec::new();
        for i in 1..=argc {
            params.push(Token::Char('#', Cat::Param));
            params.push(Token::Char(
                char::from_digit(i as u32, 10).unwrap_or('1'),
                Cat::Other,
            ));
        }
        self.set_meaning(name, Meaning::Macro(self.new_macro(params, body)));
        // The default is recorded but not yet honoured: a call that omits the
        // bracket would need the same peek `\@ifnextchar` does, at every use
        // site rather than at the definition. Recorded here so the definition
        // does not silently drop it.
        if let Some(d) = default {
            self.optional_defaults.insert(name, d);
        }
        Ok(())
    }

    /// A command name after `\newcommand` and friends: `\x` or `{\x}`,
    /// either spelling optionally preceded by the star of `\newcommand*`.
    ///
    /// The star asks LaTeX for a command whose arguments may not contain
    /// `\par`. Nothing here reads that restriction, so the star is skipped
    /// rather than refused -- and skipping it is not cosmetic: an unrecognised
    /// star made the scan give up, which DROPPED the definition. That is how
    /// pandoc's `\newcommand*\pandocbounded[1]{...}` left every use of
    /// `\pandocbounded` undefined and stopped the document.
    fn scan_command_name(&mut self, lx: &mut Lexer) -> R<Option<CsId>> {
        let mut braced = false;
        let name = loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Ok(None);
            };
            match t {
                t if t.is_space() => continue,
                Token::Char('*', _) => continue,
                Token::Char(_, Cat::BeginGroup) => {
                    braced = true;
                    continue;
                }
                Token::Cs(n) => break n,
                // A name that is not a control sequence: `\newcommand` given
                // something it cannot define. LaTeX would raise an error and
                // carry on; refusing the whole document over one definition is
                // the harsher answer, and the rest of the file is still
                // readable. The definition is dropped, not guessed at.
                //
                // The rest of the braced name goes with it. Leaving it in the
                // stream is worse than the error was: the scan that follows
                // takes the NEXT group as the body, which swallows whatever
                // came after the definition.
                _ => {
                    if braced {
                        let mut depth = 1usize;
                        while let Some(t) = lx.next_token(&self.cats) {
                            match t {
                                Token::Char(_, Cat::BeginGroup) => depth += 1,
                                Token::Char(_, Cat::EndGroup) => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    return Ok(None);
                }
            }
        };
        // `{\x}` leaves its closing brace, and a brace left in the stream is a
        // group the document never opened -- it swallowed the definition body
        // and printed as text.
        if braced {
            while let Some(t) = lx.next_token(&self.cats) {
                if matches!(t, Token::Char(_, Cat::EndGroup)) {
                    break;
                }
                if !t.is_space() {
                    lx.push_back(&[t]);
                    break;
                }
            }
        }
        Ok(Some(name))
    }

    /// `[...]` if one is next, leaving the stream untouched when it is not.
    fn scan_optional_bracket(&mut self, lx: &mut Lexer) -> R<Option<Vec<Token>>> {
        let mut skipped = Vec::new();
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                lx.push_back(&skipped);
                return Ok(None);
            };
            if t.is_space() {
                skipped.push(t);
                continue;
            }
            if !matches!(&t, Token::Char('[', _)) {
                lx.push_back(&[t]);
                return Ok(None);
            }
            let mut inner = Vec::new();
            loop {
                let Some(t) = lx.next_token(&self.cats) else {
                    return Err(TexError("Missing ] inserted".into()));
                };
                if matches!(&t, Token::Char(']', _)) {
                    return Ok(Some(inner));
                }
                inner.push(t);
            }
        }
    }

    fn skip_spaces(&mut self, lx: &mut Lexer) {
        while let Some(t) = lx.next_token(&self.cats) {
            if !t.is_space() {
                lx.push_back(&[t]);
                return;
            }
        }
    }

    /// A `{...}` group's tokens, with the braces removed.
    pub fn read_group_tokens(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                return Err(TexError("Runaway argument".into()));
            };
            if t.is_space() {
                continue;
            }
            if matches!(t, Token::Char(_, Cat::BeginGroup)) {
                return self.read_balanced(lx);
            }
            // A single token stands in for a group, as TeX's argument scan does.
            return Ok(vec![t]);
        }
    }

    /// `\let\a=\b` — `\a` takes `\b`'s CURRENT meaning, not a reference to it.
    /// `\futurelet\a\b\c` — look one token past the next without eating it.
    ///
    /// `tex.web` §1221: read three tokens, `\let` the first take the meaning of
    /// the THIRD, then put the second and third back so the stream is exactly as
    /// it was. That non-destructive peek is the whole basis of LaTeX's
    /// `\@ifnextchar`, and so of every optional argument in the language --
    /// `\newcommand{\x}[1]{...}` cannot be written without it.
    /// What `name` contributes to a number, if anything.
    ///
    /// The two definitions differ in WHEN the value is known: a `\chardef`
    /// constant is fixed at definition time and can be folded while lowering,
    /// while a `\countdef` name is a register whose value the run may change.
    pub fn numeric_cs(&self, name: CsId) -> Option<NumericCs> {
        match self.meanings.get(&name) {
            Some(Meaning::CharDef(v)) => Some(NumericCs::Value(*v)),
            Some(Meaning::CountDef(r)) => Some(NumericCs::Register(*r)),
            _ => None,
        }
    }

    /// `\chardef\a=65` and `\countdef\pageno=0`, which differ only in what the
    /// number means and in the message tex reports when it is out of range.
    pub fn compile_time_numeric_def(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        let Some(Token::Cs(name)) = self.take(lx, false) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        self.skip_equals_file(lx)?;
        let v = self.scan_number_file(lx)?;
        // Both are limited to 0..255, and tex names the limit differently for
        // each: `! Bad character code (256).` against `! Bad register code
        // (256).`. Measured, and worth keeping apart -- the message is how a
        // document author finds which of the two they got wrong.
        // Each has its own limit and its own message, all measured: a mathchar
        // runs to "7FFF and is reported as `Bad mathchar', while the other two
        // stop at 255 and name the table they overran.
        let (limit, what) = match kind {
            "mathchardef" => (32767, "mathchar"),
            "dimendef" | "skipdef" | "toksdef" => (255, "register code"),
            "chardef" => (255, "character code"),
            _ => (255, "register code"),
        };
        if !(0..=limit).contains(&v) {
            return Err(TexError(format!("Bad {what} ({v})")));
        }
        let meaning = match kind {
            // A mathchar is a constant like a chardef: what differs is the
            // range it may hold, not what it then does.
            "chardef" | "mathchardef" => Meaning::CharDef(v),
            // A dimension register is a register: the name stands for the slot,
            // and the slot is the one the dimensions live in.
            "dimendef" => Meaning::CountDef(crate::compiler::DIMEN_BASE + v),
            // A token register is not a slot: the list lives in the frontend,
            // so the name stands for the register number itself.
            "toksdef" => Meaning::ToksDef(v),
            "skipdef" => {
                Meaning::CountDef(crate::compiler::SKIP_BASE + v * crate::compiler::SKIP_STRIDE)
            }
            _ => Meaning::CountDef(v),
        };
        self.set_meaning(name, meaning);
        Ok(())
    }

    fn do_futurelet(&mut self, lx: &mut Lexer, pending_only: bool) -> R<()> {
        let Some(Token::Cs(name)) = self.take(lx, pending_only) else {
            return Err(TexError("Missing control sequence inserted".into()));
        };
        let Some(first) = self.take(lx, pending_only) else {
            return Err(TexError("Missing token for \\futurelet".into()));
        };
        let Some(second) = self.take(lx, pending_only) else {
            return Err(TexError("Missing token for \\futurelet".into()));
        };
        let meaning = match &second {
            Token::Char(c, k) => Meaning::Char(*c, *k),
            Token::Cs(n) => match self.meanings.get(n) {
                Some(m) => m.clone(),
                None => Meaning::Primitive(*n),
            },
        };
        self.set_meaning(name, meaning);
        // Both tokens go back, in order: the peek must not consume them.
        lx.push_back(&[first, second]);
        Ok(())
    }

    fn do_let(&mut self, lx: &mut Lexer) -> R<()> {
        let name = self.scan_defined_name(lx)?;
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
        // `\newcommand{\x}[n][default]` gives the FIRST parameter a default, and
        // a call may leave it out: `\includegraphics[width=1in]{f}` passes it,
        // `\includegraphics{f}` takes the default. TeX has no such parameter,
        // so the macro carries n ordinary ones and the bracket is matched here
        // -- which is where LaTeX matches it too, in `\@ifnextchar`.
        let args = match self.optional_defaults.get(&name).cloned() {
            Some(default) => {
                let first = self.read_optional(lx, &default, pending_only)?;
                let mut args = vec![first];
                args.extend(self.match_params(
                    lx,
                    m.params.get(2..).unwrap_or(&[]),
                    pending_only,
                )?);
                args
            }
            None => self.match_params(lx, &m.params, pending_only)?,
        };
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
        let out = self.weave_advice(name, out)?;
        lx.push_back(&out);
        Ok(())
    }

    /// Wrap `expansion` in whatever advice matches `name`.
    ///
    /// Costs one `is_empty` check when a document registers none, which is
    /// every document that does not use the feature.
    fn weave_advice(&mut self, name: CsId, expansion: Vec<Token>) -> R<Vec<Token>> {
        if self.intercepts.is_empty() || self.advice_depth > 0 {
            return Ok(expansion);
        }
        let advice: Vec<crate::intercepts::Intercept> = self
            .intercepts
            .matching(name.name())
            .into_iter()
            .cloned()
            .collect();
        if advice.is_empty() {
            return Ok(expansion);
        }

        let mut out = expansion;
        for a in advice {
            let body = self.advice_body(&a.handler)?;
            out = match a.advice {
                crate::intercepts::Advice::Before => {
                    let mut v = wrapped(&body);
                    v.extend(out);
                    v
                }
                crate::intercepts::Advice::After => {
                    let mut v = out;
                    v.extend(wrapped(&body));
                    v
                }
                // `\proceed` stands for what the macro would have expanded to.
                // A handler with no `\proceed` replaces the call outright,
                // which is what an around advice that suppresses is for.
                crate::intercepts::Advice::Around => {
                    let mut v = Vec::with_capacity(body.len() + out.len());
                    for t in &body {
                        match t {
                            Token::Cs(n) if n.name() == "proceed" => v.extend(out.iter().copied()),
                            other => v.push(*other),
                        }
                    }
                    wrapped(&v)
                }
            };
        }
        Ok(out)
    }

    /// The body of an advice handler: a macro that takes no parameters.
    ///
    /// A handler with parameters is refused rather than called with whatever
    /// followed the intercepted call, which would silently eat the document's
    /// own tokens.
    fn advice_body(&self, handler: &str) -> R<Vec<Token>> {
        let id = CsId::intern(handler);
        match self.meanings.get(&id) {
            Some(Meaning::Macro(m)) if m.params.is_empty() => Ok(m.body.clone()),
            Some(Meaning::Macro(_)) => Err(TexError(format!(
                "Intercept handler \\{handler} takes parameters; advice handlers take none"
            ))),
            _ => Err(TexError(format!(
                "Intercept handler \\{handler} is not a macro"
            ))),
        }
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

    /// The `[...]` of a call to a macro whose first parameter has a default,
    /// or the default itself when the call left the brackets out.
    fn read_optional(
        &mut self,
        lx: &mut Lexer,
        default: &[Token],
        pending_only: bool,
    ) -> R<Vec<Token>> {
        loop {
            let Some(t) = self.take(lx, pending_only) else {
                return Ok(default.to_vec());
            };
            if t.is_space() {
                continue;
            }
            if !matches!(&t, Token::Char('[', _)) {
                lx.push_back(&[t]);
                return Ok(default.to_vec());
            }
            let mut inner = Vec::new();
            loop {
                let Some(t) = self.take(lx, pending_only) else {
                    return Err(TexError("Missing ] inserted".into()));
                };
                if matches!(&t, Token::Char(']', _)) {
                    return Ok(inner);
                }
                inner.push(t);
            }
        }
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

    /// `\mathcode`\x=N` and its three siblings, which read and write exactly as
    /// `\catcode` does and are scoped by a group the same way.
    fn do_charcode(&mut self, lx: &mut Lexer, table: crate::charcodes::Table) -> R<()> {
        let ch = self.scan_number(lx, false)?;
        self.skip_equals(lx)?;
        let val = self.scan_number(lx, false)?;
        let Some(c) = char::from_u32(ch as u32) else {
            return Err(TexError("Invalid code".into()));
        };
        self.save(Save::CharCode(table, c, self.charcodes.get(table, c)));
        self.charcodes
            .set(table, c, val)
            .map_err(|e| TexError(e.to_string()))
    }

    /// The same, for a caller that is lowering rather than expanding.
    pub fn compile_time_charcode(
        &mut self,
        lx: &mut Lexer,
        table: crate::charcodes::Table,
    ) -> R<()> {
        self.do_charcode(lx, table)
    }

    /// What one of the tables says about a character, for `\the`.
    pub fn charcode_value(&mut self, lx: &mut Lexer, table: crate::charcodes::Table) -> R<i64> {
        let ch = self.scan_number(lx, false)?;
        let Some(c) = char::from_u32(ch as u32) else {
            return Err(TexError("Invalid code".into()));
        };
        Ok(self.charcodes.get(table, c))
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
            // `\numexpr ...\relax` in numeric position.
            if name.name() == "numexpr" {
                return Ok(sign * self.scan_expr(lx, pending_only, false)?);
            }
            if name.name() == "count" {
                let reg = self.scan_number(lx, pending_only)?;
                return Ok(sign * *self.count.get(&reg).unwrap_or(&0));
            }
            // A `\chardef` constant IS a number here, and a `\countdef` name is
            // the register it stands for -- which is the whole reason plain.tex
            // can write `\catcode`\~=\active`.
            match self.numeric_cs(name) {
                Some(NumericCs::Value(v)) => return Ok(sign * v),
                Some(NumericCs::Register(r)) => {
                    return Ok(sign * *self.count.get(&r).unwrap_or(&0));
                }
                None => {}
            }
            // A macro in numeric position expands and the scan resumes.
            if self.try_expand(lx, name, pending_only)? {
                return Ok(sign * self.scan_number(lx, pending_only)?);
            }
            return Err(TexError(format!("Missing number, found \\{}", name.name())));
        }
        // A constant may be octal or hexadecimal as well as decimal
        // (tex.web §445). The hex digits are UPPERCASE -- the opposite of `^^`
        // notation, which takes lowercase -- so `"FF` is 255 and `"ff` is an
        // error. Measured against tex -ini, not assumed from the symmetry.
        let radix = match &cur {
            Token::Char('\'', _) => Some(8u32),
            Token::Char('"', _) => Some(16u32),
            _ => None,
        };
        if let Some(radix) = radix {
            let ok = |c: char| match radix {
                8 => ('0'..='7').contains(&c),
                _ => c.is_ascii_digit() || ('A'..='F').contains(&c),
            };
            let mut digits = String::new();
            while let Some(t) = self.take(lx, pending_only) {
                match &t {
                    Token::Char(c, _) if ok(*c) => digits.push(*c),
                    other if other.is_space() && !digits.is_empty() => break,
                    other => {
                        lx.push_back(std::slice::from_ref(other));
                        break;
                    }
                }
            }
            if digits.is_empty() {
                return Err(TexError("Missing number, treated as zero".into()));
            }
            const INFINITY: i64 = 2147483647;
            return match i64::from_str_radix(&digits, radix) {
                Ok(n) if n <= INFINITY => Ok(sign * n),
                _ => Err(TexError("Number too big".into())),
            };
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
        // tex.web §445: a constant above 2147483647 is too big. The limit is
        // TeX's, not the host integer's -- checking only for `i64` overflow let
        // `\\count1=99999999999` through, where real tex reports and clamps.
        // The magnitude is tested before the sign is applied, which is why tex
        // answers -2147483647 (not -2147483648) for a too-big negative.
        const INFINITY: i64 = 2147483647;
        match digits.parse::<i64>() {
            Ok(n) if n <= INFINITY => Ok(sign * n),
            _ => Err(TexError("Number too big".into())),
        }
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
                    // A `\protected` macro is NOT expanded here: that is the
                    // whole point of the prefix. It goes into the body as
                    // itself and runs when the body does, so redefining it
                    // afterwards changes what the \edef'd macro produces.
                    let protected = matches!(
                        self.meanings.get(&name),
                        Some(Meaning::Macro(m)) if m.protected
                    );
                    if protected || !self.try_expand(lx, name, true)? {
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

    /// Skip the arm of a conditional the frontend decided against, the way
    /// TeX's `\if` skips: tokens are read and thrown away, nested conditionals
    /// counted, and nothing in them expands or assigns.
    ///
    /// Answers `true` when it stopped at an `\else` and `false` at the `\fi`;
    /// either way the token it stopped on is consumed.
    pub fn compile_time_skip_arm(&mut self, lx: &mut Lexer, stop_at_else: bool) -> R<bool> {
        self.skip_to(lx, stop_at_else, false)
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

    /// Try to expand `name` as an EXPANDABLE primitive in running text.
    ///
    /// Returns whether it was one. `\expandafter`, `\csname`, `\noexpand` and
    /// the conditionals are expandable: `tex.web` §366 has the expander handle
    /// them wherever they appear, including at the outermost level of a file.
    /// The lowerer dispatches top-level control sequences itself and so had no
    /// arm for them, which made `\expandafter\def\csname foo\endcsname{...}`
    /// -- the idiom LaTeX's own `\newcommand` is built out of -- an undefined
    /// control sequence at the top of a document while working perfectly inside
    /// a macro body. This is the door back into the expander for that case.
    pub fn expand_in_text(&mut self, lx: &mut Lexer, name: CsId) -> R<bool> {
        self.try_expand(lx, name, false)
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
    /// `\futurelet` while lowering: the peek it performs is a frontend fact,
    /// exactly as `\let` is.
    pub fn compile_time_futurelet(&mut self, lx: &mut Lexer) -> R<()> {
        self.do_futurelet(lx, false)
    }

    /// `\newcommand` and friends while lowering, exactly as `\def` is: a macro
    /// definition is a frontend fact.
    pub fn compile_time_newcommand(&mut self, lx: &mut Lexer, kind: &str) -> R<()> {
        self.do_newcommand(lx, kind)
    }

    /// Consume a LaTeX preamble directive that takes `[options]{arguments}` and
    /// produces nothing here.
    ///
    /// `\documentclass`, `\usepackage` and `\PassOptionsToPackage` load files
    /// texrs has no way to run -- a package is TeX that builds boxes, and there
    /// is no stomach to build them in. Consuming the arguments rather than
    /// failing on them lets the REST of a document be read, which is the
    /// difference between "cannot open this file at all" and "read it, minus
    /// what the packages would have drawn".
    pub fn compile_time_preamble_directive(&mut self, lx: &mut Lexer, args: usize) -> R<()> {
        let _ = self.scan_optional_bracket(lx)?;
        for _ in 0..args {
            self.skip_spaces(lx);
            let _ = self.read_group_tokens(lx)?;
        }
        Ok(())
    }

    /// `\makeatletter` / `\makeatother`: @ becomes a letter, or stops being
    /// one. LaTeX's internal names are only spellable while it is.
    pub fn compile_time_set_at_letter(&mut self, on: bool) {
        let cat = match on {
            true => Cat::Letter,
            false => Cat::Other,
        };
        self.set_cat('@', cat);
    }

    /// `[...]` if one is next, for a caller outside the expander.
    pub fn read_optional_bracket(&mut self, lx: &mut Lexer) -> R<Option<Vec<Token>>> {
        self.scan_optional_bracket(lx)
    }

    /// Consume a `*` if the next thing is one, for the starred form of a
    /// sectioning command.
    ///
    /// `\chapter*{Preface}` is the unnumbered chapter, and it is one token
    /// different from `\chapter{Preface}`. Left unread, the `*` becomes the
    /// first character of the heading.
    pub fn skip_optional_star(&mut self, lx: &mut Lexer) -> bool {
        let mut skipped = Vec::new();
        loop {
            let Some(t) = lx.next_token(&self.cats) else {
                lx.push_back(&skipped);
                return false;
            };
            if t.is_space() {
                skipped.push(t);
                continue;
            }
            if matches!(&t, Token::Char('*', _)) {
                return true;
            }
            lx.push_back(&[t]);
            lx.push_back(&skipped);
            return false;
        }
    }

    /// A `{...}` group's text, for a caller outside the expander.
    pub fn read_group_text_pub(&mut self, lx: &mut Lexer) -> R<String> {
        let toks = self.read_group_tokens(lx)?;
        Ok(toks.iter().map(|t| t.to_text(self.escape)).collect())
    }

    /// A `{...}` group's tokens, unexpanded.
    pub fn read_balanced_group(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        self.read_group_tokens(lx)
    }

    pub fn compile_time_let(&mut self, lx: &mut Lexer) -> R<()> {
        self.do_let(lx)
    }

    /// Consume an advice marker, if `name` is one.
    ///
    /// The markers travel in the token stream, so every walker that reads
    /// tokens has to know them — the expander's own dispatch, and the message
    /// walker in `lower.rs`, which would otherwise render one as text.
    pub fn advice_marker(&mut self, name: &str) -> bool {
        match name {
            ADVICE_IN => {
                self.advice_depth += 1;
                true
            }
            ADVICE_OUT => {
                self.advice_depth = self.advice_depth.saturating_sub(1);
                true
            }
            _ => false,
        }
    }

    /// `\intercept{<kind>}{<pattern>}{\handler}` — register advice.
    ///
    /// At COMPILE time, like `\def`: advice changes what the macros AFTER it
    /// expand to, so a registration read at run time would be a registration
    /// that never fired. Scoped to the enclosing group, like any other
    /// assignment.
    pub fn compile_time_intercept(&mut self, lx: &mut Lexer) -> R<()> {
        let kind = self.group_text(lx)?;
        let advice = crate::intercepts::Advice::parse(&kind).ok_or_else(|| {
            TexError(format!(
                "Unknown intercept kind `{kind}'; use before, after or around"
            ))
        })?;
        let pattern = self.group_text(lx)?;
        let handler = self.group_cs_name(lx)?;
        if let Some(frame) = self.groups.last_mut() {
            frame.push(Save::Intercepts(self.intercepts.clone()));
        }
        self.intercepts
            .register(&pattern, advice, &handler)
            .map_err(TexError)
    }

    /// The characters of the next brace group, spaces and all.
    fn group_text(&mut self, lx: &mut Lexer) -> R<String> {
        let toks = self.next_group(lx)?;
        Ok(toks
            .iter()
            .filter_map(|t| match t {
                Token::Char(c, _) => Some(*c),
                Token::Cs(_) => None,
            })
            .collect())
    }

    /// The name of the single control sequence in the next brace group.
    fn group_cs_name(&mut self, lx: &mut Lexer) -> R<String> {
        let toks = self.next_group(lx)?;
        match toks.iter().find(|t| matches!(t, Token::Cs(_))) {
            Some(Token::Cs(n)) => Ok(n.name().to_string()),
            _ => Err(TexError(
                "Intercept handler must be a control sequence, as in {\\handler}".into(),
            )),
        }
    }

    /// Skip to the next `{` and read the group it opens.
    fn next_group(&mut self, lx: &mut Lexer) -> R<Vec<Token>> {
        while let Some(t) = lx.next_token(&self.cats) {
            match &t {
                _ if t.is_space() => continue,
                Token::Char(_, Cat::BeginGroup) => return self.read_balanced(lx),
                _ => return Err(TexError("Missing { for \\intercept".into())),
            }
        }
        Err(TexError("Runaway \\intercept: missing {".into()))
    }
    pub fn compile_time_begin_group(&mut self) {
        self.begin_group();
    }
    pub fn compile_time_end_group(&mut self) -> R<()> {
        self.end_group()
    }
    /// A macro carrying whichever definition prefixes are in force.
    fn new_macro(&self, params: Vec<Token>, body: Vec<Token>) -> Macro {
        Macro {
            params,
            body,
            long: self.long,
            outer: self.outer,
            protected: self.protected,
        }
    }

    /// `\long`, for a caller that is lowering rather than expanding.
    pub fn set_long_prefix(&mut self, on: bool) {
        self.long = on;
    }

    /// `\outer`, likewise.
    pub fn set_outer_prefix(&mut self, on: bool) {
        self.outer = on;
    }

    /// `\protected`, likewise.
    pub fn set_protected_prefix(&mut self, on: bool) {
        self.protected = on;
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
        self.set_meaning(name, Meaning::Macro(self.new_macro(Vec::new(), body)));
    }

    /// The same, with a parameter text — validated as `\def`'s is.
    ///
    /// `\edef` takes parameters exactly as `\def` does; what differs is only
    /// when the BODY is expanded. A definition path that dropped them would
    /// leave `\edef\pair#1,#2.{…}` matching nothing and its delimiters landing
    /// in the output.
    pub fn define_macro_with_params(
        &mut self,
        name: CsId,
        params: Vec<Token>,
        body: Vec<Token>,
    ) -> R<()> {
        validate_params(&params)?;
        self.set_meaning(name, Meaning::Macro(self.new_macro(params, body)));
        Ok(())
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
