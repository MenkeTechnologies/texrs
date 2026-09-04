//! Reading a formula into an mlist.
//!
//! `tex.web` §1090-§1206: what `main_control` does while `mode` is `mmode`.
//! A character becomes a noad through its `\mathcode` (§1154-§1155), a `^` or
//! `_` attaches a script to the noad before it (§1176-§1177), `\over` turns
//! everything read so far into a numerator (§1181-§1184), and `\left` opens a
//! group that `\right` closes (§1191-§1192).
//!
//! The tables here are plain.tex's, because plain.tex IS what `$x+y$` means:
//! INITEX gives a letter `"7100+c`, a digit `"7000+c` and everything else its
//! own code (§232), and plain.tex then rewrites the punctuation
//! (plain.tex:86-110) and names some four hundred symbols with `\mathchardef`
//! (plain.tex:744-920).

use super::noad::*;
use crate::catcode::Cat;
use crate::expand::{Engine, NumericCs, TexError};
use crate::lexer::Lexer;
use crate::token::Token;

type R<T> = Result<T, TexError>;

/// `var_code = "70000` div `"1000` (§1151): a class of 7 means "use the
/// current `\fam`", and with no `\fam` set that is the family in the code.
const VAR_CLASS: i64 = 7;

/// Where a formula stops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stop {
    /// A closing brace: a subformula read for a nucleus or a script.
    Brace,
    /// A math-shift character: `$` or the second `$` of `$$`.
    MathShift,
    /// A control sequence by name: `\)`, `\]`, `\right`, or an environment's
    /// `\end`.
    Cs(&'static str),
}

/// One formula, read: its rows and columns, and the equation number beside it.
///
/// A `$x+y$` is one row of one column and no number, which is what every
/// formula was before `&` and `\eqno` meant anything.
#[derive(Clone, Debug, Default)]
pub struct Formula {
    /// The rows, each a list of columns: `\\` ends a row and `&` a column.
    pub rows: Vec<Vec<Vec<Noad>>>,
    /// `\eqno`'s or `\leqno`'s mlist (§1204). It is MATH -- `$$x=y\eqno(3)$$`
    /// sets its number in a formula -- so it is an mlist like any other.
    pub number: Option<Vec<Noad>>,
    /// `\leqno` rather than `\eqno`: §1206's `l`, the number at the left.
    pub number_left: bool,
}

impl Formula {
    /// Every noad of every cell, in reading order.
    ///
    /// What a formula MEANS when the columns are not being lined up: one list,
    /// which is what a `$...$` in a paragraph is and what `set::plain` spells.
    pub fn flat(&self) -> Vec<Noad> {
        self.rows.iter().flatten().flatten().cloned().collect()
    }

    /// Whether this is the ordinary case: one row, one column.
    pub fn is_single(&self) -> bool {
        self.rows.len() == 1 && self.rows[0].len() == 1
    }
}

/// Read a formula from `lx`, expanding macros as `main_control` would.
///
/// Expansion happens HERE rather than beforehand for the reason §1151 reads
/// the input one token at a time: `\frac` takes two arguments and a macro that
/// produces one of them has to expand before the argument is delimited, while
/// `\sqrt` and `\left` must NOT have their delimiter expanded away.
pub fn formula(eng: &mut Engine, lx: &mut Lexer, stop: Stop) -> R<Formula> {
    let mut scanner = Scanner {
        eng,
        depth: 0,
        rows: Vec::new(),
        cells: Vec::new(),
        number: None,
        number_left: false,
    };
    let last = scanner.mlist(lx, stop)?;
    scanner.cells.push(last);
    scanner.rows.push(std::mem::take(&mut scanner.cells));
    Ok(Formula {
        rows: scanner.rows,
        number: scanner.number,
        number_left: scanner.number_left,
    })
}

struct Scanner<'a> {
    eng: &'a mut Engine,
    /// How deep the subformulas nest, so a pathological document stops with a
    /// diagnostic rather than exhausting the host stack -- the bound `block`
    /// keeps on the lowerer, for the same reason.
    depth: usize,
    /// The rows closed by a `\\`, and the cells of the row in hand. Only the
    /// OUTERMOST level fills them: an `&` inside a `\hbox` or a `\left(` group
    /// belongs to whatever is there, not to the display's columns.
    rows: Vec<Vec<Vec<Noad>>>,
    cells: Vec<Vec<Noad>>,
    number: Option<Vec<Noad>>,
    number_left: bool,
}

const MAX_DEPTH: usize = 48;

impl Scanner<'_> {
    /// One mlist, up to `stop`.
    fn mlist(&mut self, lx: &mut Lexer, stop: Stop) -> R<Vec<Noad>> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(TexError(
                "TeX capacity exceeded, sorry [math nesting]".into(),
            ));
        }
        let out = self.mlist_inner(lx, stop);
        self.depth -= 1;
        out
    }

    fn mlist_inner(&mut self, lx: &mut Lexer, stop: Stop) -> R<Vec<Noad>> {
        let mut list: Vec<Noad> = Vec::new();
        // §1181's `incompleat_noad`: `\over` takes everything read so far as
        // the numerator and leaves the rest of the formula to be the
        // denominator, so the fraction cannot be built until the list ends.
        let mut incompleat: Option<Fraction> = None;
        // The class `\mathbin` and its siblings force on the next field
        // (§1156, §1163), and the family `\mathrm` and its siblings force on
        // the characters inside it.
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            match tok {
                Token::Char(_, Cat::EndGroup) => {
                    if stop == Stop::Brace {
                        return Ok(self.finish(list, incompleat));
                    }
                    // A stray `}` ends the formula rather than running away.
                    return Ok(self.finish(list, incompleat));
                }
                Token::Char(_, Cat::MathShift) => {
                    if stop == Stop::MathShift {
                        return Ok(self.finish(list, incompleat));
                    }
                    return Ok(self.finish(list, incompleat));
                }
                Token::Char(_, Cat::BeginGroup) => {
                    let inner = self.mlist(lx, Stop::Brace)?;
                    list.push(Noad::Atom(Atom::new(Class::Ord, Field::List(inner))));
                }
                // §1090: spaces are ignored in math mode.
                Token::Char(_, Cat::Space) => {}
                Token::Char('^', _) => {
                    let field = self.field(lx)?;
                    self.attach(&mut list, field, true);
                }
                Token::Char('_', _) => {
                    let field = self.field(lx)?;
                    self.attach(&mut list, field, false);
                }
                // §768: an alignment tab ends the column in hand. Only at the
                // outermost level of a formula -- an `&` inside a group is
                // that group's business.
                Token::Char(_, Cat::AlignTab) if self.depth == 1 => {
                    let cell = self.finish(std::mem::take(&mut list), incompleat.take());
                    self.cells.push(cell);
                }
                Token::Char(_, Cat::AlignTab) => {}
                Token::Char(c, _) => {
                    let code = mathcode(c);
                    if let Some(atom) = atom_for_code(code) {
                        list.push(Noad::Atom(atom));
                    }
                }
                Token::Cs(name) => {
                    let n = name.name();
                    // §1046-§1047: a `\par` in math mode is `insert_dollar_sign`
                    // -- TeX decides the author left a `$` out, closes the
                    // formula and rescans. Which is also what keeps a stray
                    // `$` in running text from swallowing the rest of a
                    // document: the damage stops at the end of the paragraph.
                    if n == "par" {
                        lx.push_back(&[Token::Cs(name)]);
                        return Ok(self.finish(list, incompleat));
                    }
                    // `\right` and `\end` end whatever they are inside, even
                    // when this level was not looking for them -- otherwise a
                    // `\left(` with no `\right` swallows the document -- and
                    // both are handed BACK, because each carries an argument
                    // the caller has to read: `\right`'s delimiter and
                    // `\end`'s environment name.
                    if n == "right" || n == "end" {
                        lx.push_back(&[Token::Cs(name)]);
                        return Ok(self.finish(list, incompleat));
                    }
                    // A stop named by a control sequence: `\)` or `\]`, which
                    // carry nothing and are consumed here.
                    if let Stop::Cs(want) = stop {
                        if n == want {
                            return Ok(self.finish(list, incompleat));
                        }
                    }
                    // §774's `car_ret`: `\cr` ends a row, and LaTeX's `\\` is
                    // spelled for it. `\\*` is the same row end with a page
                    // break forbidden after it, which is a penalty this path
                    // has nowhere to put.
                    if self.depth == 1 && matches!(n, "\\" | "cr" | "crcr") {
                        let _ = self.eng.skip_optional_star(lx);
                        let cell = self.finish(std::mem::take(&mut list), incompleat.take());
                        self.cells.push(cell);
                        self.rows.push(std::mem::take(&mut self.cells));
                        continue;
                    }
                    // §1204's `start_eq_no`: everything after `\eqno` is a
                    // formula of its own, and it ends where this one does.
                    if self.depth == 1 && matches!(n, "eqno" | "leqno") {
                        self.number_left = n == "leqno";
                        let number = self.mlist(lx, stop)?;
                        self.number = Some(number);
                        return Ok(self.finish(list, incompleat));
                    }
                    if let Some(f) = self.generalized_fraction(lx, n)? {
                        if incompleat.is_none() {
                            incompleat = Some(Fraction {
                                numerator: std::mem::take(&mut list),
                                ..f
                            });
                        }
                        continue;
                    }
                    self.control_sequence(lx, name, &mut list)?;
                }
            }
        }
        Ok(self.finish(list, incompleat))
    }

    /// Close the list, building the `\over` fraction if one was begun (§1184).
    fn finish(&mut self, list: Vec<Noad>, incompleat: Option<Fraction>) -> Vec<Noad> {
        match incompleat {
            None => list,
            Some(f) => vec![Noad::Fraction(Fraction {
                denominator: list,
                ..f
            })],
        }
    }

    /// `\over`, `\atop`, `\above` and their `withdelims` forms (§1181-§1183).
    ///
    /// The numerator is filled in by the caller, which is the one that holds
    /// the list read so far.
    fn generalized_fraction(&mut self, lx: &mut Lexer, name: &str) -> R<Option<Fraction>> {
        let (thickness, delims) = match name {
            "over" => (None, false),
            "atop" => (Some(0), false),
            "above" => (Some(self.eng.scan_dimen(lx, false)?), false),
            "overwithdelims" => (None, true),
            "atopwithdelims" => (Some(0), true),
            "abovewithdelims" => (Some(i64::MIN), true),
            _ => return Ok(None),
        };
        let (left, right) = match delims {
            true => (self.delimiter(lx)?, self.delimiter(lx)?),
            false => (Delimiter::null(), Delimiter::null()),
        };
        // `\abovewithdelims` reads its dimension AFTER its two delimiters.
        let thickness = match thickness {
            Some(t) if t == i64::MIN => Some(self.eng.scan_dimen(lx, false)?),
            other => other,
        };
        Ok(Some(Fraction {
            thickness,
            numerator: Vec::new(),
            denominator: Vec::new(),
            left_delimiter: left,
            right_delimiter: right,
        }))
    }

    /// Attach a script to the noad before it, or to a fresh empty Ord when
    /// there is none (§1176).
    ///
    /// `scripts_allowed(#)` (§687) is every noad from `ord_noad` up to but not
    /// including `left_noad`, which takes in the radical and the two rules as
    /// well as the eight classes -- so `$\sqrt2^3$` puts the `3` on the
    /// radical rather than beside it. A fraction has a numerator and a
    /// denominator where the scripts would be and so is not among them, and
    /// nor is a `\left`.
    fn attach(&mut self, list: &mut Vec<Noad>, field: Field, superscript: bool) {
        fn slot(item: &mut Noad) -> Option<&mut Atom> {
            match item {
                Noad::Atom(a) | Noad::Over(a) | Noad::Under(a) | Noad::VCenter(a) => Some(a),
                Noad::Radical(r) => Some(&mut r.nucleus),
                Noad::Accent(acc) => Some(&mut acc.atom),
                _ => None,
            }
        }
        let free = match list.last_mut().and_then(slot) {
            Some(a) => match superscript {
                true => a.supscr.is_empty(),
                false => a.subscr.is_empty(),
            },
            None => false,
        };
        if !free {
            list.push(Noad::Atom(Atom::default()));
        }
        if let Some(a) = list.last_mut().and_then(slot) {
            match superscript {
                true => a.supscr = field,
                false => a.subscr = field,
            }
        }
    }

    /// `scan_math` (§1151): the next subformula, as one of a noad's fields.
    fn field(&mut self, lx: &mut Lexer) -> R<Field> {
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            match tok {
                Token::Char(_, Cat::Space) => continue,
                Token::Char(_, Cat::BeginGroup) => {
                    return Ok(Field::List(self.mlist(lx, Stop::Brace)?));
                }
                Token::Char(c, _) => {
                    let code = mathcode(c);
                    return Ok(match char_field(code) {
                        Some(f) => f,
                        None => Field::Empty,
                    });
                }
                Token::Cs(name) => {
                    // A one-token subformula that is a symbol is that symbol;
                    // anything else is read as a list of one item, which is
                    // what `\sqrt{\frac ab}` and `x^\frac12` need.
                    if let Some(code) = self.math_code_of(name.name()) {
                        return Ok(char_field(code).unwrap_or(Field::Empty));
                    }
                    let mut one = Vec::new();
                    self.control_sequence(lx, name, &mut one)?;
                    return Ok(Field::List(one));
                }
            }
        }
        Ok(Field::Empty)
    }

    /// The 15-bit math code a control sequence stands for, from the engine's
    /// own `\mathchardef` table first and plain.tex's names second.
    fn math_code_of(&mut self, name: &str) -> Option<i64> {
        // `\mathchardef\half="2201` in the DOCUMENT wins over the table below:
        // `src/expand.rs` already carries `\mathchardef`, and duplicating its
        // values here rather than asking it is how the two drift apart.
        //
        // It stores the value as a plain constant, so a `\chardef` and a
        // `\mathchardef` are the same `Value` by the time they reach here. A
        // math code names a class, a family and a character and so is at least
        // "100; a `\chardef` is a character code and so is under 256. Values
        // below that are left to the character path, which is what §1154's
        // `char_given` case does with them.
        let id = crate::token::CsId::intern(name);
        if let Some(NumericCs::Value(v)) = self.eng.numeric_cs(id) {
            if v >= 256 {
                return Some(v);
            }
            if (0..256).contains(&v) {
                return Some(mathcode(v as u8 as char));
            }
        }
        MATH_SYMBOLS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
    }

    /// One delimiter argument, as `scan_delimiter` reads it (§1160): a
    /// `\delimiter` code, a control sequence that stands for one, or a
    /// character with a `\delcode`.
    fn delimiter(&mut self, lx: &mut Lexer) -> R<Delimiter> {
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            match tok {
                Token::Char(_, Cat::Space) => continue,
                Token::Char('.', _) => return Ok(Delimiter::null()),
                Token::Char(c, _) => {
                    return Ok(delcode(c)
                        .map(Delimiter::from_code)
                        .unwrap_or_else(Delimiter::null))
                }
                Token::Cs(name) => {
                    return Ok(match delimiter_code(name.name()) {
                        Some(code) => Delimiter::from_code(code & 0xFF_FFFF),
                        None => Delimiter::null(),
                    })
                }
            }
        }
        Ok(Delimiter::null())
    }

    /// One group's worth of tokens as an mlist, for `\frac`'s two arguments
    /// and `\sqrt`'s one.
    fn argument(&mut self, lx: &mut Lexer) -> R<Vec<Noad>> {
        match self.field(lx)? {
            Field::List(l) => Ok(l),
            Field::Char(c) => Ok(vec![Noad::Atom(Atom::new(Class::Ord, Field::Char(c)))]),
            Field::Literal(c) => Ok(vec![Noad::Atom(Atom::new(Class::Ord, Field::Literal(c)))]),
            _ => Ok(Vec::new()),
        }
    }

    /// `math_ac` (§1165): the `accent_noad` a `\mathaccent` code makes.
    ///
    /// The code is fifteen bits like a `\mathchardef`'s: the class is read and
    /// discarded -- an accent has no class of its own, and §761 leaves it Ord
    /// -- and what is kept is the family and the character. The nucleus is the
    /// next subformula, exactly as `scan_math` reads one.
    fn accent_noad(&mut self, lx: &mut Lexer, code: i64) -> R<Noad> {
        let accent = MathChar {
            fam: ((code / 256) % 16) as usize,
            character: (code % 256) as u8,
        };
        let nucleus = self.field(lx)?;
        Ok(Noad::Accent(Accent {
            accent,
            atom: Atom::new(Class::Ord, nucleus),
        }))
    }

    /// One `<mudimen>` (§448, §455): an optional sign, a decimal constant, and
    /// the unit `mu`.
    ///
    /// The value is returned IN `mu`, scaled by $2^{16}$ the way every other
    /// dimension is held: `5mu` is `5*65536`. It is not converted to points
    /// here because a `mu` is one eighteenth of the `math_quad` of the size the
    /// glue finally lands in, which the reader cannot know (§716).
    ///
    /// The three names plain.tex gives (plain.tex:373-375) stand where a
    /// number does, because `\mskip\thinmuskip` is how `\,` is written.
    fn mu_dimen(&mut self, lx: &mut Lexer) -> R<Scaled> {
        Ok(self.mu_glue_amount(lx)?.natural)
    }

    /// One `<muglue>`: a `<mudimen>` and the optional `plus` and `minus` parts.
    fn mu_glue(&mut self, lx: &mut Lexer) -> R<MuGlue> {
        let mut glue = self.mu_glue_amount(lx)?;
        if self.eng.scan_keyword(lx, "plus", false) {
            glue.stretch = self.mu_glue_amount(lx)?.natural;
        }
        if self.eng.scan_keyword(lx, "minus", false) {
            glue.shrink = self.mu_glue_amount(lx)?.natural;
        }
        Ok(glue)
    }

    /// A `<mudimen>` or one of plain.tex's three named mu glues, with the sign
    /// in front of it that `\mskip-\thinmuskip` writes (plain.tex:733).
    fn mu_glue_amount(&mut self, lx: &mut Lexer) -> R<MuGlue> {
        // §440: any number of signs and spaces, of which only the minus signs
        // count.
        let mut negative = false;
        let mut digits: Vec<u8> = Vec::new();
        let mut fraction: Vec<u8> = Vec::new();
        loop {
            let Some(tok) = lx.next_token(&self.eng.cats) else {
                return Ok(MuGlue::default());
            };
            match tok {
                Token::Char(_, Cat::Space) => continue,
                Token::Char('-', _) => negative = !negative,
                Token::Char('+', _) => {}
                Token::Char(c, _) if c.is_ascii_digit() || c == '.' || c == ',' => {
                    lx.push_back(&[tok]);
                    break;
                }
                // An internal mu glue: the three plain.tex names.
                Token::Cs(name) => {
                    let spec = match name.name() {
                        "thinmuskip" => THIN_MU_SKIP,
                        "medmuskip" => MED_MU_SKIP,
                        "thickmuskip" => THICK_MU_SKIP,
                        _ => return Ok(MuGlue::default()),
                    };
                    let glue = MuGlue::of(spec);
                    return Ok(match negative {
                        true => glue.negated(),
                        false => glue,
                    });
                }
                other => {
                    lx.push_back(&[other]);
                    break;
                }
            }
        }
        // §444-§452: the integer part, then at most seventeen digits of
        // fraction, then the unit.
        let mut past_point = false;
        while let Some(tok) = lx.next_token(&self.eng.cats) {
            match tok {
                Token::Char(c, _) if c.is_ascii_digit() => match past_point {
                    true => fraction.push(c as u8 - b'0'),
                    false => digits.push(c as u8 - b'0'),
                },
                Token::Char('.', _) | Token::Char(',', _) if !past_point => past_point = true,
                other => {
                    lx.push_back(&[other]);
                    break;
                }
            }
        }
        if !self.eng.scan_keyword(lx, "mu", false) {
            return Err(TexError("Illegal unit of measure (mu inserted)".into()));
        }
        let whole: i64 = digits.iter().fold(0i64, |a, d| a * 10 + *d as i64);
        fraction.truncate(17);
        let amount = whole * 65536 + round_decimals(&fraction);
        Ok(MuGlue::fixed(match negative {
            true => -amount,
            false => amount,
        }))
    }

    /// Everything a control sequence can mean inside a formula.
    fn control_sequence(
        &mut self,
        lx: &mut Lexer,
        name: crate::token::CsId,
        out: &mut Vec<Noad>,
    ) -> R<()> {
        let n = name.name();
        // A symbol named by `\mathchardef`, in the document or in plain.tex.
        if let Some(code) = self.math_code_of(n) {
            if let Some(atom) = atom_for_code(code) {
                out.push(Noad::Atom(atom));
            }
            return Ok(());
        }
        // A delimiter named by `\delimiter`: in mid-list it is the class and
        // small variant of its code (§1154's `delim_num` case).
        if let Some(code) = delimiter_code(n) {
            if let Some(atom) = atom_for_code(code / 0x10000) {
                out.push(Noad::Atom(atom));
            }
            return Ok(());
        }
        match n {
            // §1156: the eight class-forcing primitives, and the two that
            // build a rule over or under their argument.
            "mathord" | "mathop" | "mathbin" | "mathrel" | "mathopen" | "mathclose"
            | "mathpunct" | "mathinner" => {
                let class = match n {
                    "mathop" => Class::Op,
                    "mathbin" => Class::Bin,
                    "mathrel" => Class::Rel,
                    "mathopen" => Class::Open,
                    "mathclose" => Class::Close,
                    "mathpunct" => Class::Punct,
                    "mathinner" => Class::Inner,
                    _ => Class::Ord,
                };
                let field = self.field(lx)?;
                out.push(Noad::Atom(Atom::new(class, field)));
            }
            "overline" => {
                let field = self.field(lx)?;
                out.push(Noad::Over(Atom::new(Class::Ord, field)));
            }
            "underline" => {
                let field = self.field(lx)?;
                out.push(Noad::Under(Atom::new(Class::Ord, field)));
            }
            // §1156's `limit_switch`: they modify the op noad before them.
            "limits" | "nolimits" | "displaylimits" => {
                if let Some(Noad::Atom(a)) = out.last_mut() {
                    a.limits = match n {
                        "limits" => Limits::Above,
                        "nolimits" => Limits::Beside,
                        _ => Limits::Normal,
                    };
                }
            }
            // §688's style nodes.
            "displaystyle" => out.push(Noad::Style(DISPLAY_STYLE)),
            "textstyle" => out.push(Noad::Style(TEXT_STYLE)),
            "scriptstyle" => out.push(Noad::Style(SCRIPT_STYLE)),
            "scriptscriptstyle" => out.push(Noad::Style(SCRIPT_SCRIPT_STYLE)),
            // `\radical"270370` (plain.tex:1013), which is what `\sqrt` is.
            "sqrt" | "radical" => {
                let delim = match n {
                    "radical" => {
                        Delimiter::from_code(self.eng.scan_number_pending(lx)? & 0xFF_FFFF)
                    }
                    _ => Delimiter::from_code(0x270370),
                };
                // LaTeX's `\sqrt[n]{x}` is plain.tex's `\root n \of {x}`
                // (plain.tex:1018-1022), which builds its index out of BOXES
                // around the radical rather than out of a noad. The tokens are
                // read here and `make_root` places them.
                let index = match self.eng.read_optional_bracket(lx)? {
                    Some(tokens) => {
                        // The index's tokens are already read, so they go back
                        // in front of a closing brace of this scanner's own
                        // making and are read as one subformula -- which is
                        // what `{#1}` inside `\root`'s `\hbox` makes of them.
                        let mut back = tokens;
                        back.push(Token::Char('}', Cat::EndGroup));
                        lx.push_back(&back);
                        Some(self.mlist(lx, Stop::Brace)?)
                    }
                    None => None,
                };
                let body = self.argument(lx)?;
                out.push(Noad::Radical(Radical {
                    left_delimiter: delim,
                    nucleus: Atom::new(Class::Ord, Field::List(body)),
                    index,
                }));
            }
            // LaTeX's `\frac`, which is `\over` written with its two arguments
            // in front of it rather than around it.
            "frac" | "dfrac" | "tfrac" => {
                let numerator = self.argument(lx)?;
                let denominator = self.argument(lx)?;
                out.push(Noad::Fraction(Fraction {
                    thickness: None,
                    numerator,
                    denominator,
                    left_delimiter: Delimiter::null(),
                    right_delimiter: Delimiter::null(),
                }));
            }
            "binom" => {
                let numerator = self.argument(lx)?;
                let denominator = self.argument(lx)?;
                out.push(Noad::Fraction(Fraction {
                    thickness: Some(0),
                    numerator,
                    denominator,
                    left_delimiter: Delimiter::from_code(0x028300),
                    right_delimiter: Delimiter::from_code(0x029301),
                }));
            }
            // §1191: `\left` opens a group that `\right` closes, and the pair
            // becomes one Inner noad.
            "left" => {
                let left = self.delimiter(lx)?;
                let mut inner = self.mlist(lx, Stop::Cs("right"))?;
                // The `\right` that stopped the scan was pushed back by the
                // arm above; take it and read its delimiter.
                let right = match lx.next_token(&self.eng.cats) {
                    Some(Token::Cs(c)) if c.name() == "right" => self.delimiter(lx)?,
                    Some(t) => {
                        lx.push_back(&[t]);
                        Delimiter::null()
                    }
                    None => Delimiter::null(),
                };
                let mut whole = vec![Noad::Left(left)];
                whole.append(&mut inner);
                whole.push(Noad::Right(right));
                out.push(Noad::Atom(Atom::new(Class::Inner, Field::List(whole))));
            }
            // plain.tex:730-733 and the `\quad` family: pure spacing, in mu
            // and in em respectively.
            "," => out.push(Noad::MuGlue(MuGlue::of(THIN_MU_SKIP))),
            ">" => out.push(Noad::MuGlue(MuGlue::of(MED_MU_SKIP))),
            ";" => out.push(Noad::MuGlue(MuGlue::of(THICK_MU_SKIP))),
            "!" => out.push(Noad::MuGlue(MuGlue::of(THIN_MU_SKIP).negated())),
            "thinspace" => out.push(Noad::MuGlue(MuGlue::of(THIN_MU_SKIP))),
            "quad" => out.push(Noad::Glue(mu(18 * 65536))),
            "qquad" => out.push(Noad::Glue(mu(36 * 65536))),
            " " | "enspace" => out.push(Noad::Glue(mu(9 * 65536))),
            // The dot runs, as `\mathinner` of three punctuation dots
            // (plain.tex:931-932).
            "ldots" | "dots" | "cdots" => {
                let code = match n == "cdots" {
                    true => 0x6201,
                    false => 0x613A,
                };
                let dots: Vec<Noad> = (0..3)
                    .filter_map(|_| atom_for_code(code).map(Noad::Atom))
                    .collect();
                out.push(Noad::Atom(Atom::new(Class::Inner, Field::List(dots))));
            }
            // `math_ac` (§1165): `\mathaccent"7013` and the twelve plain.tex
            // names for it (plain.tex:939-950).
            "mathaccent" => {
                let code = self.eng.scan_number_pending(lx)?;
                out.push(self.accent_noad(lx, code)?);
            }
            _ if MATH_ACCENTS.iter().any(|(a, _)| *a == n) => {
                let code = MATH_ACCENTS
                    .iter()
                    .find(|(a, _)| *a == n)
                    .map(|(_, c)| *c)
                    .unwrap_or(0);
                out.push(self.accent_noad(lx, code)?);
            }
            // §1167: `\vcenter{...}` leaves a `vcenter_noad` whose nucleus is
            // the vbox the group packed.
            "vcenter" => {
                let body = self.argument(lx)?;
                out.push(Noad::VCenter(Atom::new(Class::Ord, Field::List(body))));
            }
            // §1171-§1174: `\mathchoice` reads four groups, one per style.
            "mathchoice" => {
                out.push(Noad::Choice(Box::new(Choice {
                    display: self.argument(lx)?,
                    text: self.argument(lx)?,
                    script: self.argument(lx)?,
                    script_script: self.argument(lx)?,
                })));
            }
            // §1171: `\mkern` and `\mskip`, written in `mu` by the document.
            // They stay in `mu` until `mlist_to_hlist` knows the size they
            // land in (§716-§717), which is the whole point of the unit.
            "mkern" => {
                let amount = self.mu_dimen(lx)?;
                out.push(Noad::MuKern(amount));
            }
            "mskip" => {
                let glue = self.mu_glue(lx)?;
                out.push(Noad::MuGlue(glue));
            }
            // A family switch: `\mathrm{x}` sets its argument in family 0, and
            // `\mathit` in family 1. The other three name faces this port has
            // no math font for and are set upright rather than dropped.
            "mathrm" | "mathbf" | "mathsf" | "mathtt" | "mathnormal" | "mathit" | "text"
            | "textrm" | "mbox" | "operatorname" => {
                let fam = match n {
                    "mathit" | "mathnormal" => 1,
                    _ => 0,
                };
                let body = self.argument(lx)?;
                let body = in_family(body, fam);
                let class = match n == "operatorname" {
                    true => Class::Op,
                    false => Class::Ord,
                };
                let mut atom = Atom::new(class, Field::List(body));
                if n == "operatorname" {
                    atom.limits = Limits::Beside;
                }
                out.push(Noad::Atom(atom));
            }
            // plain.tex:1054-1081: the operator names, which are upright
            // roman set as one Op noad. The ones that take their limits above
            // and below are the ones plain.tex does NOT follow with
            // `\nolimits`.
            _ if OPERATOR_NAMES.contains(&n) => {
                let letters: Vec<Noad> = n
                    .bytes()
                    .map(|b| {
                        Noad::Atom(Atom::new(
                            Class::Ord,
                            Field::Char(MathChar {
                                fam: 0,
                                character: b,
                            }),
                        ))
                    })
                    .collect();
                out.push(Noad::Atom(Atom {
                    class: ClassOrOrd(Class::Op),
                    nucleus: Field::List(letters),
                    limits: match OPERATOR_LIMITS.contains(&n) {
                        true => Limits::Normal,
                        false => Limits::Beside,
                    },
                    ..Atom::default()
                }));
            }
            // Anything else: a macro the document defined expands the way it
            // would anywhere else, and a name nothing defines contributes what
            // the text path would have set for it, if that is a character at
            // all.
            _ => {
                if self.eng.is_macro(name) {
                    self.eng.expand_macro_file(lx, name)?;
                    return Ok(());
                }
                if let Some(c) = crate::typeset::symbol_char(n) {
                    out.push(Noad::Atom(Atom::new(Class::Ord, Field::Literal(c))));
                }
            }
        }
        Ok(())
    }
}

/// A `\mskip` in mu, as points, at the text size's `math_quad/18` (§703,
/// §716).
///
/// The conversion needs `cur_mu`, which depends on the size the glue lands in,
/// and a glue written by the AUTHOR is converted where it is read rather than
/// where it is set. `18mu` is one quad, and one quad of `cmsy10` at ten points
/// is ten points, which is what this uses until `mlist_to_hlist` can say
/// otherwise.
fn mu(amount: i64) -> Scaled {
    amount / 18
}

/// `round_decimals(k)` (§102): the digits after a decimal point, as a fraction
/// of $2^{16}$.
///
/// Knuth's own loop, verbatim: it works backwards from the last digit,
/// doubling into `two = 2^17` so that the final halving rounds to nearest.
fn round_decimals(digits: &[u8]) -> Scaled {
    let mut a: i64 = 0;
    for d in digits.iter().rev() {
        a = (a + *d as i64 * 131_072) / 10;
    }
    (a + 1) / 2
}

/// The `\mathaccent` codes plain.tex names (plain.tex:939-950), and the two
/// LaTeX spellings of the same accents.
///
/// A `"70..` code is class 7 -- "use the current `\fam`" -- out of family 0,
/// which is `cmr10`'s accents; `\vec`, `\widehat` and `\widetilde` are class
/// 0 out of family 1 and 3, where the wide variants have charlists to walk
/// (§741).
pub const MATH_ACCENTS: &[(&str, i64)] = &[
    ("acute", 0x7013),
    ("grave", 0x7012),
    ("ddot", 0x707F),
    ("tilde", 0x707E),
    ("bar", 0x7016),
    ("breve", 0x7015),
    ("check", 0x7014),
    ("hat", 0x705E),
    ("vec", 0x017E),
    ("dot", 0x705F),
    ("widetilde", 0x0365),
    ("widehat", 0x0362),
];

/// Rewrite every `math_char` in a list into family `fam`, which is what
/// `\mathrm` and `\mathit` do to their argument.
fn in_family(list: Vec<Noad>, fam: usize) -> Vec<Noad> {
    fn field(f: Field, fam: usize) -> Field {
        match f {
            Field::Char(c) => Field::Char(MathChar { fam, ..c }),
            Field::List(l) => Field::List(in_family(l, fam)),
            other => other,
        }
    }
    list.into_iter()
        .map(|item| match item {
            Noad::Atom(a) => Noad::Atom(Atom {
                nucleus: field(a.nucleus, fam),
                supscr: field(a.supscr, fam),
                subscr: field(a.subscr, fam),
                ..a
            }),
            other => other,
        })
        .collect()
}

/// `set_math_char(c)` (§1155): the noad a 15-bit math code makes.
fn atom_for_code(code: i64) -> Option<Atom> {
    // `"8000` is an active character (§1155). The two plain.tex gives one to
    // are the space, which math mode ignores, and `'`, which is a prime.
    if code >= 0o100000 {
        return None;
    }
    let class = code / 0o10000;
    let fam = ((code / 256) % 16) as usize;
    let character = (code % 256) as u8;
    let atom = Atom::new(
        match class == VAR_CLASS {
            true => Class::Ord,
            false => Class::from_code(class),
        },
        Field::Char(MathChar { fam, character }),
    );
    Some(atom)
}

/// The same, as a field rather than a noad -- what `scan_math` leaves in a
/// script (§1151).
fn char_field(code: i64) -> Option<Field> {
    if code >= 0o100000 {
        return None;
    }
    Some(Field::Char(MathChar {
        fam: ((code / 256) % 16) as usize,
        character: (code % 256) as u8,
    }))
}

/// `\mathcode c` — INITEX's table (§232) with plain.tex's rewrites
/// (plain.tex:86-110).
///
/// The engine's own `\mathcode` table is not asked here because a formula is
/// read while the document is being lowered and set after it: what reaches the
/// page is the code in force where the formula stood, and carrying that per
/// formula is what `\mathcode` in a document would need. See `crate::math`'s
/// note on what is not wired.
pub fn mathcode(c: char) -> i64 {
    let b = match u32::from(c) {
        n if n < 256 => n as u8,
        // Outside Latin-1 a character has no Computer Modern slot at all; it
        // is set as itself, which `Field::Literal` is for.
        _ => return c as i64,
    };
    match b {
        b'A'..=b'Z' | b'a'..=b'z' => 0x7100 + b as i64,
        b'0'..=b'9' => 0x7000 + b as i64,
        b'!' => 0x5021,
        b'(' => 0x4028,
        b')' => 0x5029,
        b'*' => 0x2203,
        b'+' => 0x202B,
        b',' => 0x613B,
        b'-' => 0x2200,
        b'.' => 0x013A,
        b'/' => 0x013D,
        b':' => 0x303A,
        b';' => 0x603B,
        b'<' => 0x313C,
        b'=' => 0x303D,
        b'>' => 0x313E,
        b'?' => 0x503F,
        b'[' => 0x405B,
        b'\\' => 0x026E,
        b']' => 0x505D,
        b'{' => 0x4266,
        b'|' => 0x026A,
        b'}' => 0x5267,
        _ => b as i64,
    }
}

/// `\delcode c` — plain.tex:122-130. INITEX leaves every one at -1 except the
/// period, which is 0 and means "no delimiter".
pub fn delcode(c: char) -> Option<i64> {
    Some(match c {
        '(' => 0x028300,
        ')' => 0x029301,
        '[' => 0x05B302,
        ']' => 0x05D303,
        '<' => 0x26830A,
        '>' => 0x26930B,
        '/' => 0x02F30E,
        '|' => 0x26A30C,
        '\\' => 0x26E30F,
        _ => return None,
    })
}

/// The 27-bit `\delimiter` code a plain.tex name stands for
/// (plain.tex:963-990), and the LaTeX spellings of the same delimiters.
pub fn delimiter_code(name: &str) -> Option<i64> {
    Some(match name {
        "lmoustache" => 0x437A340,
        "rmoustache" => 0x537B341,
        "lgroup" => 0x462833A,
        "rgroup" => 0x562933B,
        "arrowvert" => 0x26A33C,
        "Arrowvert" => 0x26B33D,
        "bracevert" => 0x77C33E,
        "Vert" | "|" => 0x26B30D,
        "vert" => 0x26A30C,
        "uparrow" => 0x3222378,
        "downarrow" => 0x3223379,
        "updownarrow" => 0x326C33F,
        "Uparrow" => 0x322A37E,
        "Downarrow" => 0x322B37F,
        "Updownarrow" => 0x326D377,
        "backslash" => 0x26E30F,
        "rangle" => 0x526930B,
        "langle" => 0x426830A,
        "rbrace" | "}" => 0x5267309,
        "lbrace" | "{" => 0x4266308,
        "rceil" => 0x5265307,
        "lceil" => 0x4264306,
        "rfloor" => 0x5263305,
        "lfloor" => 0x4262304,
        _ => return None,
    })
}

/// The operator names plain.tex defines with `\mathop{\rm …}`
/// (plain.tex:1054-1081).
const OPERATOR_NAMES: &[&str] = &[
    "log", "lg", "ln", "lim", "limsup", "liminf", "sin", "arcsin", "sinh", "cos", "arccos", "cosh",
    "tan", "arctan", "tanh", "cot", "coth", "sec", "csc", "max", "min", "sup", "inf", "arg", "ker",
    "dim", "hom", "det", "exp", "Pr", "gcd", "deg",
];

/// The ones plain.tex does NOT follow with `\nolimits`, so their scripts go
/// above and below in display style.
const OPERATOR_LIMITS: &[&str] = &[
    "lim", "limsup", "liminf", "max", "min", "sup", "inf", "det", "Pr", "gcd",
];

/// Every `\mathchardef` in plain.tex, verbatim (plain.tex:744-920).
///
/// The value is the 15-bit math code: class, family, character. It says both
/// what a symbol IS -- a Rel spaces differently from a Bin -- and where its
/// glyph lives, so it cannot be reduced to a character.
pub const MATH_SYMBOLS: &[(&str, i64)] = &[
    // Lowercase Greek, family 1 (plain.tex:744-772).
    ("alpha", 0x010B),
    ("beta", 0x010C),
    ("gamma", 0x010D),
    ("delta", 0x010E),
    ("epsilon", 0x010F),
    ("zeta", 0x0110),
    ("eta", 0x0111),
    ("theta", 0x0112),
    ("iota", 0x0113),
    ("kappa", 0x0114),
    ("lambda", 0x0115),
    ("mu", 0x0116),
    ("nu", 0x0117),
    ("xi", 0x0118),
    ("pi", 0x0119),
    ("rho", 0x011A),
    ("sigma", 0x011B),
    ("tau", 0x011C),
    ("upsilon", 0x011D),
    ("phi", 0x011E),
    ("chi", 0x011F),
    ("psi", 0x0120),
    ("omega", 0x0121),
    ("varepsilon", 0x0122),
    ("vartheta", 0x0123),
    ("varpi", 0x0124),
    ("varrho", 0x0125),
    ("varsigma", 0x0126),
    ("varphi", 0x0127),
    // Uppercase Greek, family 0 and class 7 (plain.tex:773-783).
    ("Gamma", 0x7000),
    ("Delta", 0x7001),
    ("Theta", 0x7002),
    ("Lambda", 0x7003),
    ("Xi", 0x7004),
    ("Pi", 0x7005),
    ("Sigma", 0x7006),
    ("Upsilon", 0x7007),
    ("Phi", 0x7008),
    ("Psi", 0x7009),
    ("Omega", 0x700A),
    // The ordinary symbols (plain.tex:785-816).
    ("aleph", 0x0240),
    ("imath", 0x017B),
    ("jmath", 0x017C),
    ("ell", 0x0160),
    ("wp", 0x017D),
    ("Re", 0x023C),
    ("Im", 0x023D),
    ("partial", 0x0140),
    ("infty", 0x0231),
    ("prime", 0x0230),
    ("emptyset", 0x023B),
    ("varnothing", 0x023B),
    ("nabla", 0x0272),
    ("top", 0x023E),
    ("bot", 0x023F),
    ("triangle", 0x0234),
    ("forall", 0x0238),
    ("exists", 0x0239),
    ("neg", 0x023A),
    ("lnot", 0x023A),
    ("flat", 0x015B),
    ("natural", 0x015C),
    ("sharp", 0x015D),
    ("clubsuit", 0x027C),
    ("diamondsuit", 0x027D),
    ("heartsuit", 0x027E),
    ("spadesuit", 0x027F),
    // The large operators, family 3 (plain.tex:818-833).
    ("coprod", 0x1360),
    ("bigvee", 0x1357),
    ("bigwedge", 0x1356),
    ("biguplus", 0x1355),
    ("bigcap", 0x1354),
    ("bigcup", 0x1353),
    ("intop", 0x1352),
    ("prod", 0x1351),
    ("sum", 0x1350),
    ("bigotimes", 0x134E),
    ("bigoplus", 0x134C),
    ("bigodot", 0x134A),
    ("ointop", 0x1348),
    ("bigsqcup", 0x1346),
    ("smallint", 0x1273),
    // The binary operators (plain.tex:835-869).
    ("triangleleft", 0x212F),
    ("triangleright", 0x212E),
    ("bigtriangleup", 0x2234),
    ("bigtriangledown", 0x2235),
    ("wedge", 0x225E),
    ("land", 0x225E),
    ("vee", 0x225F),
    ("lor", 0x225F),
    ("cap", 0x225C),
    ("cup", 0x225B),
    ("ddagger", 0x227A),
    ("dagger", 0x2279),
    ("sqcap", 0x2275),
    ("sqcup", 0x2274),
    ("uplus", 0x225D),
    ("amalg", 0x2271),
    ("diamond", 0x2205),
    ("bullet", 0x220F),
    ("wr", 0x226F),
    ("div", 0x2204),
    ("odot", 0x220C),
    ("oslash", 0x220B),
    ("otimes", 0x220A),
    ("ominus", 0x2209),
    ("oplus", 0x2208),
    ("mp", 0x2207),
    ("pm", 0x2206),
    ("circ", 0x220E),
    ("bigcirc", 0x220D),
    ("setminus", 0x226E),
    ("cdot", 0x2201),
    ("ast", 0x2203),
    ("times", 0x2202),
    ("star", 0x213F),
    // The relations (plain.tex:871-914).
    ("propto", 0x322F),
    ("sqsubseteq", 0x3276),
    ("sqsupseteq", 0x3277),
    ("parallel", 0x326B),
    ("mid", 0x326A),
    ("dashv", 0x3261),
    ("vdash", 0x3260),
    ("nearrow", 0x3225),
    ("searrow", 0x3226),
    ("nwarrow", 0x322D),
    ("swarrow", 0x322E),
    ("Leftrightarrow", 0x322C),
    ("Leftarrow", 0x3228),
    ("Rightarrow", 0x3229),
    ("leq", 0x3214),
    ("le", 0x3214),
    ("geq", 0x3215),
    ("ge", 0x3215),
    ("succ", 0x321F),
    ("prec", 0x321E),
    ("approx", 0x3219),
    ("succeq", 0x3217),
    ("preceq", 0x3216),
    ("supset", 0x321B),
    ("subset", 0x321A),
    ("supseteq", 0x3213),
    ("subseteq", 0x3212),
    ("in", 0x3232),
    ("ni", 0x3233),
    ("owns", 0x3233),
    ("gg", 0x321D),
    ("ll", 0x321C),
    ("not", 0x3236),
    ("leftrightarrow", 0x3224),
    ("leftarrow", 0x3220),
    ("gets", 0x3220),
    ("rightarrow", 0x3221),
    ("to", 0x3221),
    ("mapstochar", 0x3237),
    ("sim", 0x3218),
    ("simeq", 0x3227),
    ("perp", 0x323F),
    ("equiv", 0x3211),
    ("asymp", 0x3210),
    ("smile", 0x315E),
    ("frown", 0x315F),
    ("leftharpoonup", 0x3128),
    ("leftharpoondown", 0x3129),
    ("rightharpoonup", 0x312A),
    ("rightharpoondown", 0x312B),
    ("lhook", 0x312C),
    ("rhook", 0x312D),
    // The punctuation (plain.tex:928-930).
    ("ldotp", 0x613A),
    ("cdotp", 0x6201),
    ("colon", 0x603A),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// INITEX's rule (§232) and plain.tex's rewrites, which together decide
    /// which font every character of a formula is set from.
    #[test]
    fn a_letter_is_family_one_and_a_digit_family_zero() {
        // `\mathcode`\A` is "7141: class 7, family 1, character `A`.
        assert_eq!(mathcode('A'), 0x7141);
        assert_eq!(mathcode('x'), 0x7178);
        // `\mathcode`\0` is "7030: class 7, family 0, character `0`.
        assert_eq!(mathcode('0'), 0x7030);
        // `\mathcode`\+` is "202B: class 2 (Bin), family 0, character `+`.
        assert_eq!(mathcode('+'), 0x202B);
        // `=` is a Rel, `(` an Open, `)` a Close, `,` a Punct.
        assert_eq!(mathcode('='), 0x303D);
        assert_eq!(mathcode('('), 0x4028);
        assert_eq!(mathcode(')'), 0x5029);
        assert_eq!(mathcode(','), 0x613B);
    }

    /// The class and the family a code makes, as §1155 unpacks them.
    #[test]
    fn a_math_code_unpacks_the_way_section_1155_unpacks_it() {
        let x = atom_for_code(mathcode('x')).unwrap();
        assert_eq!(x.class(), Class::Ord, "a letter is an Ord, not a class 7");
        assert!(matches!(
            x.nucleus,
            Field::Char(MathChar {
                fam: 1,
                character: b'x'
            })
        ));
        let plus = atom_for_code(mathcode('+')).unwrap();
        assert_eq!(plus.class(), Class::Bin);
        assert!(matches!(
            plus.nucleus,
            Field::Char(MathChar {
                fam: 0,
                character: b'+'
            })
        ));
        // `\sum` is class 1, family 3, character "50 -- an Op out of cmex10.
        let sum = atom_for_code(0x1350).unwrap();
        assert_eq!(sum.class(), Class::Op);
        assert!(matches!(
            sum.nucleus,
            Field::Char(MathChar {
                fam: 3,
                character: 0x50
            })
        ));
    }

    /// A delimiter code names a small and a large variant, in two families.
    #[test]
    fn a_delimiter_code_names_both_variants() {
        // `\delcode`\(` is "028300: cmsy10's `(` at "28, cmex10's at "00.
        let d = Delimiter::from_code(0x028300);
        assert_eq!((d.small_fam, d.small_char), (0, b'('));
        assert_eq!((d.large_fam, d.large_char), (3, 0x00));
        // `\sqrt` is `\radical"270370`: cmsy10's radical at "70, cmex10's
        // at "70.
        let r = Delimiter::from_code(0x270370);
        assert_eq!((r.small_fam, r.small_char), (2, 0x70));
        assert_eq!((r.large_fam, r.large_char), (3, 0x70));
        assert!(Delimiter::null().is_null());
    }
}
