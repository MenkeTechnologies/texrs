//! LaTeX's equation numbers: which places of a display take one, and what the
//! number reads.
//!
//! Both runs ask [`places`] the same question, so they cannot disagree about
//! which rows are numbered. The label pass (`super::pass_over_math`) answers
//! each [`Number::Step`] with `\refstepcounter{equation}` and lets the kernel's
//! own counters run; the typesetting run has no run-time registers to read
//! while it lowers -- a formula's setting, number and all, is made from what
//! lowering read -- so [`Numbering`] keeps the same counters in step itself,
//! from the same definitions: `\the<counter>` and the `\cl@<counter>` reset
//! lists are macros, and macros ARE known while lowering.

use std::collections::HashMap;

use super::noad::{Atom, Class, Field, MathChar, Noad};
use super::parse::Formula;
use crate::catcode::Cat;
use crate::expand::{Engine, Meaning};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};

/// What one place of a display is numbered with.
pub enum Number<'f> {
    /// The next value of `equation`: ltmath.dtx's `\refstepcounter{equation}`.
    Step,
    /// amsmath's `\tag{..}`, or `\tag*{..}` when the flag is set: the tag is
    /// the number and nothing is stepped.
    Tag(&'f [Token], bool),
    /// No number: a starred environment, `\nonumber`, or a display that never
    /// numbers.
    Unnumbered,
}

/// One numbered place: the row the number goes beside, what it is, and the
/// `\label`s that name it.
pub struct Place<'f> {
    pub row: usize,
    pub number: Number<'f>,
    pub labels: Vec<&'f [Token]>,
}

/// The places `f` numbers, in the order latex.ltx meets them.
///
/// `equation` and `multline` number the display once, whichever row says
/// `\nonumber`; `align`, `alignat`, `gather` and `eqnarray` number every row
/// that does not, and amsmath's `\label` names its own row's number. A starred
/// form numbers nothing, a `\tag` replaces the number it is on, and anything
/// else -- `\[`, `$$`, `displaymath` -- has one unnumbered place holding every
/// label it carries.
pub fn places<'f>(f: &'f Formula, environment: &str) -> Vec<Place<'f>> {
    let starred = environment.ends_with('*');
    let base = environment.trim_end_matches('*');
    let last = f.rows.len().saturating_sub(1);
    let tag_in = |row: Option<usize>| {
        f.tags
            .iter()
            .rev()
            .find(|(r, _, _)| row.is_none_or(|row| *r == row))
            .map(|(_, t, star)| Number::Tag(t.as_slice(), *star))
    };
    let labels_in = |row: Option<usize>| -> Vec<&'f [Token]> {
        f.labels
            .iter()
            .filter(|(r, _)| row.is_none_or(|row| *r == row))
            .map(|(_, key)| key.as_slice())
            .collect()
    };
    match base {
        "equation" | "multline" => vec![Place {
            row: last,
            number: tag_in(None).unwrap_or(match starred || !f.unnumbered.is_empty() {
                true => Number::Unnumbered,
                false => Number::Step,
            }),
            labels: labels_in(None),
        }],
        "align" | "alignat" | "gather" | "eqnarray" => (0..f.rows.len())
            .map(|row| Place {
                row,
                number: tag_in(Some(row)).unwrap_or(match starred || f.unnumbered.contains(&row) {
                    true => Number::Unnumbered,
                    false => Number::Step,
                }),
                labels: labels_in(Some(row)),
            })
            .collect(),
        _ => vec![Place {
            row: last,
            number: tag_in(None).unwrap_or(Number::Unnumbered),
            labels: labels_in(None),
        }],
    }
}

/// The characters of a key or a tag, as `\label` reads its argument.
pub fn spelled(tokens: &[Token]) -> String {
    tokens
        .iter()
        .filter_map(|t| match t {
            Token::Char(c, _) => Some(*c),
            Token::Cs(_) => None,
        })
        .collect()
}

/// The sectioning commands `src/lower.rs` claims by name, which step the
/// counter of the same name unless starred.
const UNITS: [&str; 4] = ["chapter", "section", "subsection", "subsubsection"];

/// LaTeX's counters as the typesetting run follows them while it lowers.
///
/// Only what an equation number can read is kept: the sectioning units, which
/// a `\numberwithin` or a class with chapters puts in front of it and whose
/// steps reset it, and `equation` itself. A counter nothing has stepped is 0,
/// as `\newcounter` leaves it.
#[derive(Default)]
pub struct Numbering {
    counters: HashMap<String, i64>,
}

impl Numbering {
    /// Follow `tok`, a token the lowerer is about to act on, when it moves a
    /// counter: a sectioning command or a `\setcounter` / `\addtocounter`.
    /// Whatever is read to decide is put back, so the lowerer acts on the
    /// token exactly as it would have.
    pub fn watch(&mut self, eng: &mut Engine, lx: &mut Lexer, tok: &Token) {
        let Token::Cs(name) = tok else { return };
        let name = name.name();
        if UNITS.contains(&name) {
            // latex.ltx's `\@ssect` is `\@sect` without the `\refstepcounter`.
            match eng.skip_optional_star(lx) {
                true => lx.push_back(&[Token::Char('*', Cat::Other)]),
                false => self.step(eng, name),
            }
            return;
        }
        if name != "setcounter" && name != "addtocounter" {
            return;
        }
        let mut taken = Vec::new();
        let counter = raw_group(eng, lx, &mut taken);
        let value = raw_group(eng, lx, &mut taken);
        lx.push_back(&taken);
        let (Some(counter), Some(value)) = (counter, value) else {
            return;
        };
        // A value that is not a literal number -- `\value{x}`, a macro -- is
        // run-time arithmetic and is not followed.
        let Ok(value) = value.trim().parse::<i64>() else {
            return;
        };
        let held = self.counters.entry(counter).or_insert(0);
        *held = match name {
            "setcounter" => value,
            _ => *held + value,
        };
    }

    /// latex.ltx's `\stepcounter`: one more, and every counter on its
    /// `\cl@` list back to nought.
    pub fn step(&mut self, eng: &Engine, name: &str) {
        *self.counters.entry(name.to_string()).or_insert(0) += 1;
        self.reset_within(eng, name, 0);
    }

    /// `\@stpelt` over `\cl@name`: each subordinate is set to -1 and stepped,
    /// so its own subordinates are reset too.
    fn reset_within(&mut self, eng: &Engine, name: &str, depth: usize) {
        // The lists are a tree in every class; the bound is for a document
        // that made one a cycle.
        if depth > 8 {
            return;
        }
        for under in elts(eng, &format!("cl@{name}")) {
            self.counters.insert(under.clone(), 0);
            self.reset_within(eng, &under, depth + 1);
        }
    }

    /// `\the<name>` as it reads now: the class's definition followed through
    /// `\the<unit>`, `\arabic` and their relatives. A definition this cannot
    /// follow reads as `\arabic{name}`.
    pub fn the(&self, eng: &Engine, name: &str) -> String {
        let body = macro_body(eng, &format!("the{name}"));
        body.and_then(|b| self.represent(eng, b, 0))
            .unwrap_or_else(|| self.value(name).to_string())
    }

    fn value(&self, name: &str) -> i64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    fn represent(&self, eng: &Engine, toks: &[Token], depth: usize) -> Option<String> {
        if depth > 16 {
            return None;
        }
        let mut out = String::new();
        let mut at = 0;
        while at < toks.len() {
            match &toks[at] {
                Token::Char(_, Cat::Space | Cat::BeginGroup | Cat::EndGroup) => {}
                Token::Char(c, _) => out.push(*c),
                Token::Cs(cs) => match cs.name() {
                    // ltcounts.dtx's `\arabic{c}` and the kernel's: the
                    // counter's name in braces.
                    style @ ("arabic" | "roman" | "Roman" | "alph" | "Alph") => {
                        let (counter, used) = braced(&toks[at + 1..])?;
                        out.push_str(&styled(style, self.value(&counter))?);
                        at += used;
                    }
                    // latex.ltx's internal forms, `\@arabic\c@section`, which
                    // a class's `\appendix` writes.
                    style @ ("@arabic" | "@roman" | "@Roman" | "@alph" | "@Alph" | "number") => {
                        let Some(Token::Cs(register)) = toks.get(at + 1) else {
                            return None;
                        };
                        let counter = register.name().strip_prefix("c@")?;
                        let style = style.trim_start_matches('@');
                        let style = if style == "number" { "arabic" } else { style };
                        out.push_str(&styled(style, self.value(counter))?);
                        at += 1;
                    }
                    "relax" => {}
                    // `\thechapter`, `\p@equation`: a macro of no arguments
                    // reads as its body.
                    other => {
                        out.push_str(&self.represent(eng, macro_body(eng, other)?, depth + 1)?)
                    }
                },
            }
            at += 1;
        }
        Some(out)
    }
}

/// Whether the class sets equation numbers at the left: leqno.clo's
/// `\@eqnnum` backs up over `\displaywidth` to put the number there, and
/// latex.ltx's own does not mention it.
pub fn left(eng: &Engine) -> bool {
    macro_body(eng, "@eqnnum").is_some_and(|b| {
        b.iter()
            .any(|t| matches!(t, Token::Cs(c) if c.name() == "displaywidth"))
    })
}

/// A number as the mlist set beside its display: upright, from the text
/// family, at text style -- which is what `\normalfont(\theequation)` comes to
/// in Computer Modern.
pub fn mlist(text: &str) -> Vec<Noad> {
    text.chars()
        .map(|c| {
            let field = match u8::try_from(c) {
                Ok(b) if b.is_ascii() => Field::Char(MathChar {
                    fam: 0,
                    character: b,
                }),
                _ => Field::Literal(c),
            };
            Noad::Atom(Atom::new(Class::Ord, field))
        })
        .collect()
}

/// The body of a parameterless macro.
fn macro_body<'e>(eng: &'e Engine, name: &str) -> Option<&'e [Token]> {
    match eng.meanings.get(&CsId::intern(name)) {
        Some(Meaning::Macro(m)) if m.params.is_empty() => Some(&m.body),
        _ => None,
    }
}

/// The names on a `\cl@` list: `\@elt {section}\@elt {figure}...`.
fn elts(eng: &Engine, list: &str) -> Vec<String> {
    let Some(body) = macro_body(eng, list) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (at, t) in body.iter().enumerate() {
        if matches!(t, Token::Cs(c) if c.name() == "@elt") {
            if let Some((name, _)) = braced(&body[at + 1..]) {
                out.push(name);
            }
        }
    }
    out
}

/// A `{name}` at the head of `toks`, or a single character: its characters
/// and how many tokens it took.
fn braced(toks: &[Token]) -> Option<(String, usize)> {
    match toks.first()? {
        Token::Char(_, Cat::BeginGroup) => {
            let close = toks
                .iter()
                .position(|t| matches!(t, Token::Char(_, Cat::EndGroup)))?;
            Some((spelled(&toks[1..close]), close + 1))
        }
        Token::Char(c, _) => Some((c.to_string(), 1)),
        Token::Cs(_) => None,
    }
}

/// A balanced `{...}` read raw off `lx`, its characters, every token read
/// appended to `taken` so the caller can put them back. Spaces before it are
/// skipped (and kept in `taken`); anything but a group answers `None`.
fn raw_group(eng: &Engine, lx: &mut Lexer, taken: &mut Vec<Token>) -> Option<String> {
    let mut first = lx.next_token(&eng.cats)?;
    while first.is_space() {
        taken.push(first);
        first = lx.next_token(&eng.cats)?;
    }
    taken.push(first);
    if !matches!(first, Token::Char(_, Cat::BeginGroup)) {
        return None;
    }
    let mut depth = 1usize;
    let mut inner = Vec::new();
    while let Some(t) = lx.next_token(&eng.cats) {
        taken.push(t);
        match t {
            Token::Char(_, Cat::BeginGroup) => depth += 1,
            Token::Char(_, Cat::EndGroup) => {
                depth -= 1;
                if depth == 0 {
                    return Some(spelled(&inner));
                }
            }
            _ => {}
        }
        inner.push(t);
    }
    None
}

/// latex.ltx's five representations of a counter's value.
fn styled(style: &str, n: i64) -> Option<String> {
    let letter = |base: u8| match n {
        1..=26 => Some(((base + (n - 1) as u8) as char).to_string()),
        _ => None,
    };
    match style {
        "arabic" => Some(n.to_string()),
        "alph" => letter(b'a'),
        "Alph" => letter(b'A'),
        "roman" => Some(roman(n)),
        "Roman" => Some(roman(n).to_uppercase()),
        _ => None,
    }
}

/// `\romannumeral`: lower-case, and nothing for a value below one.
fn roman(mut n: i64) -> String {
    const DIGITS: [(i64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, digits) in DIGITS {
        while n >= value {
            out.push_str(digits);
            n -= value;
        }
    }
    out
}
