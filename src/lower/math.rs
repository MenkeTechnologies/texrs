//! `main_control`'s math-mode cases (§1136-§1207) as the lowerer follows
//! them: what each command reads, and the [`ListOp`] it leaves for
//! `crate::stomach::math` to carry out.
//!
//! A formula's group (§1139) is a compile-time group here as every other is:
//! `$` opens it, the formula's tokens are lowered as one block, and the `$`
//! that closes it emits `after_math` (§1194) inside the group, before the
//! `unsave` that ends it -- the order §1196 relies on.

use super::stomach::{CMode, Closer};
use super::{Lowerer, R};
use crate::catcode::Cat;
use crate::charcodes::Table;
use crate::expand::{GroupKind, Meaning, TexError};
use crate::ir::list::{Context, ListOp};
use crate::ir::{Cmd, Num};
use crate::lexer::Lexer;
use crate::stomach::BoxKind;
use crate::token::{CsId, Token};

/// A math code that makes its character active in math mode (§1151).
const ACTIVE_MATH_CODE: i64 = 0x8000;

impl Lowerer {
    /// Whether the lowerer is in a formula's list.
    pub(super) fn st_math(&self) -> bool {
        self.st_managed() && self.st_mode().math()
    }

    /// A character token the formula machinery takes: `$` wherever lists
    /// are built, and in math mode the characters §1045 gives a case of their
    /// own. `Ok(false)` leaves the token to the rest of the lowerer.
    pub(super) fn st_math_token(
        &mut self,
        lx: &mut Lexer,
        c: char,
        cat: Cat,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        if !self.st_managed() || self.st_mode() == CMode::Unmanaged {
            return Ok(false);
        }
        let mode = self.st_mode();
        if !mode.math() {
            if cat != Cat::MathShift {
                return Ok(false);
            }
            if mode.vertical() {
                // §1090: `$` begins a paragraph, and is read again in it.
                lx.push_back(&[Token::Char(c, cat)]);
                self.st_begin_paragraph(lx, out)?;
                return Ok(true);
            }
            self.st_init_math(lx, out)?;
            return Ok(true);
        }
        match cat {
            Cat::MathShift => self.st_math_shift(lx, out)?,
            // §1045: `mmode+spacer` does nothing.
            Cat::Space => {}
            Cat::Letter | Cat::Other => self.st_set_math_char_of(lx, c, out)?,
            // §1150.
            Cat::BeginGroup => {
                self.st_emit(out, ListOp::Noad(0))?;
                // §1150's `back_input`, which takes back the brace's count
                // (§325): `scan_math` reads it again.
                self.st_align_unbrace();
                lx.push_back(&[Token::Char(c, cat)]);
                self.st_scan_math(lx, out)?;
            }
            Cat::Superscript | Cat::Subscript => {
                self.st_emit(out, ListOp::SubSup(cat == Cat::Superscript))?;
                self.st_scan_math(lx, out)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// §1090's `back_input; new_graf`, for a token that begins a paragraph.
    fn st_begin_paragraph(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        self.st_new_graf(lx, true, out)
    }

    /// `init_math` (§1138): `$`, or `$$` in a paragraph.
    fn st_init_math(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        // §1138's `get_token`: not expanded, so `$\ifmmode` works.
        let next = lx.next_token(&self.eng.cats);
        let display = matches!(next, Some(Token::Char(_, Cat::MathShift)))
            && self.st_mode() == CMode::Horizontal;
        if !display {
            if let Some(t) = next {
                lx.push_back(&[t]);
            }
        }
        let font = self.eng.cur_font;
        self.st_emit(out, ListOp::InitMath { display, font })?;
        if display {
            // §1145: the paragraph so far is broken and the display goes on
            // the vertical list around it.
            self.st_pop();
        }
        self.st_formula(lx, display, None, out)
    }

    /// The formula's group and body, from just after its opening `$` (or
    /// `\eqno`) to the `after_math` its closing `$` emits.
    fn st_formula(
        &mut self,
        lx: &mut Lexer,
        display: bool,
        eqno: Option<bool>,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        self.eng.compile_time_begin_group(GroupKind::MathShift, lx.line());
        let mode = match display {
            true => CMode::DisplayMath,
            false => CMode::Math,
        };
        self.st_push(mode);
        self.st.run_open = false;
        let mark = self.globals.len();
        let known_mark = self.eng.known.mark();
        // The three dimensions §1145 assigns are the program's, and what the
        // lowerer knew of them does not hold inside the display.
        let display_slots: Vec<i64> = match display {
            true => ["predisplaysize", "displaywidth", "displayindent"]
                .iter()
                .filter_map(|n| crate::params::register_slot(n))
                .collect(),
            false => Vec::new(),
        };
        for &slot in &display_slots {
            self.eng.known.set(slot, None);
        }
        // §1139 and §1145: `\fam` is -1 inside, and `\everymath` or
        // `\everydisplay` is read first.
        if let Some(i) = crate::intpar::index("fam") {
            self.eng.assign_intpar(i, -1);
        }
        let every = match display {
            true => "everydisplay",
            false => "everymath",
        };
        if let Some(toks) = crate::params::toks_register(every).and_then(|r| self.eng.toks.get(&r).cloned()) {
            lx.push_back(&toks);
        }
        // An `\eqno`'s formula is the display's, inside its block.
        let _ = eqno;
        let mut body = self.block(lx, None)?;
        self.st.run_open = false;
        while self.st.nest.len() > 1 && self.st_mode() != mode {
            self.st_pop();
        }
        self.st_pop();
        self.eng.compile_time_end_group()?;
        let after = self.eng.take_after_group();
        lx.push_back(&after);
        let (mut saves, keeps) = self.group_parts(&mut body, mark)?;
        for &slot in &display_slots {
            if !saves.contains(&slot) {
                saves.push(slot);
            }
        }
        self.known_after_group(known_mark, &saves, &keeps);
        out.push(Cmd::Group { saves, keeps, body });
        if display {
            // §1200: `resume_after_display`, then §443's optional space.
            self.st_emit(out, ListOp::ResumeAfterDisplay)?;
            self.st_push(CMode::Horizontal);
            if let Some((t, _)) = self.eng.next_unexpandable(lx)? {
                if !t.is_space() {
                    lx.back_input(&[t]);
                }
            }
        }
        Ok(())
    }

    /// The family fonts `after_math` reads (`fam_fnt`, §230), sixteen per
    /// size.
    fn st_fams(&self) -> Vec<u16> {
        self.eng.fam_fonts.iter().flatten().copied().collect()
    }

    /// `mmode+math_shift` (§1193): the end of the formula, or of an equation
    /// number, if this is its group; otherwise §1064's `off_save`.
    fn st_math_shift(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        if self.eng.innermost_group() != Some(GroupKind::MathShift) {
            self.st_off_save(lx, Token::Char('$', Cat::MathShift));
            return Ok(());
        }
        // §1197: a display, and an equation number inside one, end with `$$`.
        let in_display = self.st_mode() == CMode::DisplayMath || self.st_in_eqno();
        if in_display {
            self.st_check_dollar(lx)?;
        }
        let fams = self.st_fams();
        self.st_emit(out, ListOp::EndMath(fams))?;
        self.st.end_block = true;
        Ok(())
    }

    /// Whether the formula being read is an `\eqno`'s, inside a display.
    fn st_in_eqno(&self) -> bool {
        let n = self.st.nest.len();
        n >= 2 && self.st.nest[n - 1] == CMode::Math && self.st.nest[n - 2] == CMode::DisplayMath
    }

    /// §1197's "Check that another `$` follows".
    fn st_check_dollar(&mut self, lx: &mut Lexer) -> R<()> {
        match self.eng.next_unexpandable(lx)? {
            Some((Token::Char(_, Cat::MathShift), _)) => {}
            other => {
                if let Some((t, _)) = other {
                    lx.back_input(&[t]);
                }
                self.eng.report(lx, "Display math should end with $$");
            }
        }
        Ok(())
    }

    /// §1064's `off_save` for a token that cannot close the innermost group:
    /// what would close it is inserted, and the token is read again.
    fn st_off_save(&mut self, lx: &mut Lexer, t: Token) {
        let esc = self.eng.esc().to_string();
        let (closer, shown) = match self.eng.innermost_group() {
            Some(GroupKind::SemiSimple) => (Token::cs("endgroup"), format!("{esc}endgroup")),
            Some(GroupKind::MathShift) => (Token::Char('$', Cat::MathShift), "$".to_string()),
            Some(GroupKind::MathLeft) => {
                lx.push_back(&[Token::Char('.', Cat::Other)]);
                (Token::cs("right"), format!("{esc}right."))
            }
            _ => (Token::Char('}', Cat::EndGroup), "}".to_string()),
        };
        lx.push_back(&[closer, t]);
        self.eng.report(lx, &format!("Missing {shown} inserted"));
    }

    /// The math code of `c` (§230), with `"8000` meaning an active
    /// character (§1152), which is put back for its meaning to be read.
    fn st_math_code(&mut self, lx: &mut Lexer, c: char) -> Option<i64> {
        let code = self.eng.charcodes.get(Table::Math, c);
        if code == ACTIVE_MATH_CODE {
            lx.push_back(&[Token::Char(c, Cat::Active)]);
            return None;
        }
        Some(code)
    }

    /// §1154's `set_math_char(ho(math_code(cur_chr)))`.
    fn st_set_math_char_of(&mut self, lx: &mut Lexer, c: char, out: &mut Vec<Cmd>) -> R<()> {
        if let Some(code) = self.st_math_code(lx, c) {
            self.st_emit(out, ListOp::MathChar(Num::Literal(code)))?;
        }
        Ok(())
    }

    /// `scan_math` (§1151): the field the action just emitted chose gets a
    /// character, or the subformula in braces that follows.
    fn st_scan_math(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        loop {
            let Some((t, p)) = self.st_next_non_blank(lx)? else {
                return Err(TexError("Missing { inserted".into()));
            };
            let code = match t {
                Token::Char(c, Cat::Letter | Cat::Other) => match self.st_math_code(lx, c) {
                    Some(code) => Num::Literal(code),
                    None => continue,
                },
                Token::Char(c, Cat::Active) => {
                    match self.eng.active_meaning(c) {
                        Some(id) => lx.push_back(&[Token::Cs(id)]),
                        None => self.eng.report(lx, "Undefined control sequence"),
                    }
                    continue;
                }
                Token::Cs(n) => match self.st_math_char_cs(lx, n, p)? {
                    Some(MathCode::Code(code)) => code,
                    Some(MathCode::Reread) => continue,
                    None => {
                        lx.push_back(&[t]);
                        return self.st_math_group(lx, out);
                    }
                },
                _ => {
                    lx.push_back(&[t]);
                    return self.st_math_group(lx, out);
                }
            };
            return self.st_emit(out, ListOp::MathFieldChar(code));
        }
    }

    /// §1153: the subformula in braces `scan_math` found.
    fn st_math_group(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        self.st_emit(out, ListOp::BeginMathGroup)?;
        self.st_list_body(lx, CMode::Math, Closer::Math, out)
    }

    /// The math code a control sequence stands for in `scan_math` (§1151):
    /// `\char`, `\mathchar`, `\delimiter`, a `\chardef` or `\mathchardef`
    /// name, or a `\let` copy of a letter. `None` when it is none of these.
    fn st_math_char_cs(
        &mut self,
        lx: &mut Lexer,
        n: CsId,
        p: Option<CsId>,
    ) -> R<Option<MathCode>> {
        match self.eng.meanings.get(&n) {
            Some(Meaning::CharDef(code)) => {
                let code = *code;
                let c = char_of(code)?;
                return Ok(Some(match self.st_math_code(lx, c) {
                    Some(m) => MathCode::Code(Num::Literal(m)),
                    None => MathCode::Reread,
                }));
            }
            Some(Meaning::MathCharDef(v)) => return Ok(Some(MathCode::Code(Num::Literal(*v)))),
            Some(Meaning::Char(c, Cat::Letter | Cat::Other)) => {
                let c = *c;
                return Ok(Some(match self.st_math_code(lx, c) {
                    Some(m) => MathCode::Code(Num::Literal(m)),
                    None => MathCode::Reread,
                }));
            }
            _ => {}
        }
        Ok(match p.unwrap_or(n).name() {
            "char" => {
                let code = self.st_literal(lx, "\\char")?;
                let c = char_of(code)?;
                match self.st_math_code(lx, c) {
                    Some(m) => Some(MathCode::Code(Num::Literal(m))),
                    None => Some(MathCode::Reread),
                }
            }
            "mathchar" => Some(MathCode::Code(self.number(lx)?)),
            "delimiter" => {
                let code = self.st_literal(lx, "\\delimiter")?;
                Some(MathCode::Code(Num::Literal(code / 0o10000)))
            }
            _ => None,
        })
    }

    /// A number the lowerer must know, as a character code is.
    fn st_literal(&mut self, lx: &mut Lexer, what: &str) -> R<i64> {
        match self.number(lx)? {
            Num::Literal(v) => Ok(v),
            _ => Err(TexError(format!("{what} needs a number known while lowering"))),
        }
    }

    /// §1160's `scan_delimiter`: a delimiter code, from a character's
    /// `\delcode` or from `\delimiter` (or, after `\radical`, the number).
    fn st_scan_delimiter(&mut self, lx: &mut Lexer) -> R<Num> {
        let Some((t, p)) = self.st_next_non_blank(lx)? else {
            return Ok(Num::Literal(0));
        };
        let code = match &t {
            Token::Char(c, Cat::Letter | Cat::Other) => self.eng.charcodes.get(Table::Delimiter, *c),
            Token::Cs(n) => match self.eng.meanings.get(n) {
                Some(Meaning::Char(c, Cat::Letter | Cat::Other)) => {
                    self.eng.charcodes.get(Table::Delimiter, *c)
                }
                _ if p.unwrap_or(*n).name() == "delimiter" => return self.number(lx),
                _ => -1,
            },
            _ => -1,
        };
        if code < 0 {
            // §1161.
            lx.back_input(&[t]);
            self.eng.report(lx, "Missing delimiter (. inserted)");
            return Ok(Num::Literal(0));
        }
        Ok(Num::Literal(code))
    }

    /// The math-mode cases of §1045 that are commands. `Ok(false)` leaves the
    /// command to the rest of the dispatch.
    pub(super) fn st_math_command(&mut self, lx: &mut Lexer, name: CsId, out: &mut Vec<Cmd>) -> R<bool> {
        // A `\let` copy of a character is that character (§1045's `cur_cmd`
        // is its category).
        if let Some(Meaning::Char(c, cat)) = self.eng.meanings.get(&name) {
            let (c, cat) = (*c, *cat);
            return self.st_math_token(lx, c, cat, out);
        }
        match self.eng.meanings.get(&name) {
            Some(Meaning::CharDef(code)) => {
                let c = char_of(*code)?;
                self.st_set_math_char_of(lx, c, out)?;
                return Ok(true);
            }
            Some(Meaning::MathCharDef(v)) => {
                let v = *v;
                self.st_emit(out, ListOp::MathChar(Num::Literal(v)))?;
                return Ok(true);
            }
            _ => {}
        }
        match name.name() {
            "char" => {
                let code = self.st_literal(lx, "\\char")?;
                let c = char_of(code)?;
                self.st_set_math_char_of(lx, c, out)?;
            }
            "mathchar" => {
                let code = self.number(lx)?;
                self.st_emit(out, ListOp::MathChar(code))?;
            }
            "delimiter" => {
                let code = self.st_literal(lx, "\\delimiter")?;
                self.st_emit(out, ListOp::MathChar(Num::Literal(code / 0o10000)))?;
            }
            // §1158.
            "mathord" | "mathop" | "mathbin" | "mathrel" | "mathopen" | "mathclose"
            | "mathpunct" | "mathinner" | "underline" | "overline" => {
                let kind = match name.name() {
                    "mathord" => 0,
                    "mathop" => 1,
                    "mathbin" => 2,
                    "mathrel" => 3,
                    "mathopen" => 4,
                    "mathclose" => 5,
                    "mathpunct" => 6,
                    "mathinner" => 7,
                    "underline" => 8,
                    _ => 9,
                };
                self.st_emit(out, ListOp::Noad(kind))?;
                self.st_scan_math(lx, out)?;
            }
            "displaylimits" => self.st_emit(out, ListOp::Limits(0))?,
            "limits" => self.st_emit(out, ListOp::Limits(1))?,
            "nolimits" => self.st_emit(out, ListOp::Limits(2))?,
            // §1163.
            "radical" => {
                let code = self.number(lx)?;
                self.st_emit(out, ListOp::Radical(code))?;
                self.st_scan_math(lx, out)?;
            }
            // §1165-§1166.
            "mathaccent" | "accent" => {
                if name.name() == "accent" {
                    let esc = self.eng.esc().to_string();
                    self.eng.report(
                        lx,
                        &format!("Please use {esc}mathaccent for accents in math mode"),
                    );
                }
                let code = self.number(lx)?;
                self.st_emit(out, ListOp::MathAccent(code))?;
                self.st_scan_math(lx, out)?;
            }
            // §1167.
            "vcenter" => {
                self.st_begin_box(lx, BoxKind::VCenter, Context::Shift(Num::Literal(0)), out)?;
            }
            // §1171.
            "displaystyle" => self.st_emit(out, ListOp::MathStyle(0))?,
            "textstyle" => self.st_emit(out, ListOp::MathStyle(2))?,
            "scriptstyle" => self.st_emit(out, ListOp::MathStyle(4))?,
            "scriptscriptstyle" => self.st_emit(out, ListOp::MathStyle(6))?,
            "nonscript" => self.st_emit(out, ListOp::NonScript)?,
            // §1172-§1174.
            "mathchoice" => {
                self.st_emit(out, ListOp::BeginChoice)?;
                for _ in 0..4 {
                    self.st_list_body(lx, CMode::Math, Closer::Choice, out)?;
                }
            }
            // §1178-§1183.
            "above" | "over" | "atop" | "abovewithdelims" | "overwithdelims" | "atopwithdelims" => {
                let code: u8 = match name.name() {
                    "above" => 0,
                    "over" => 1,
                    "atop" => 2,
                    "abovewithdelims" => 3,
                    "overwithdelims" => 4,
                    _ => 5,
                };
                let (left, right) = match code >= 3 {
                    true => (self.st_scan_delimiter(lx)?, self.st_scan_delimiter(lx)?),
                    false => (Num::Literal(0), Num::Literal(0)),
                };
                let thickness = match code % 3 {
                    0 => self.dimen_number(lx, false)?,
                    _ => Num::Literal(0),
                };
                self.st_emit(
                    out,
                    ListOp::Fraction {
                        code,
                        thickness,
                        left,
                        right,
                    },
                )?;
            }
            // §1191-§1192.
            "left" => {
                let delim = self.st_scan_delimiter(lx)?;
                self.st_emit(out, ListOp::LeftRight { right: false, delim })?;
                self.st_left_group(lx, out)?;
            }
            "right" => {
                match self.eng.innermost_group() {
                    Some(GroupKind::MathLeft) => {
                        let delim = self.st_scan_delimiter(lx)?;
                        self.st_emit(out, ListOp::LeftRight { right: true, delim })?;
                        self.st.end_block = true;
                    }
                    Some(GroupKind::MathShift) => {
                        let _ = self.st_scan_delimiter(lx)?;
                        let esc = self.eng.esc().to_string();
                        self.eng.report(lx, &format!("Extra {esc}right"));
                    }
                    _ => self.st_off_save(lx, Token::Cs(name)),
                }
            }
            // §1140-§1142.
            "eqno" | "leqno" => {
                if self.st_mode() == CMode::DisplayMath
                    && self.eng.innermost_group() == Some(GroupKind::MathShift)
                {
                    let leqno = name.name() == "leqno";
                    self.st_emit(out, ListOp::EqNo(leqno))?;
                    self.st_formula(lx, false, Some(leqno), out)?;
                    // The equation number has ended the display too (§1194).
                    let fams = self.st_fams();
                    self.st_emit(out, ListOp::EndMath(fams))?;
                    self.st.end_block = true;
                } else {
                    let esc = self.eng.esc().to_string();
                    self.eng
                        .report(lx, &format!("You can't use `{esc}{}' in math mode", name.name()));
                }
            }
            // §1061 with `mu_glue`.
            "mkern" => {
                // §1061: `scan_dimen(mu,false,false)`.
                let was = self.eng.set_mu_units(true);
                let w = self.dimen_number(lx, false);
                self.eng.set_mu_units(was);
                let w = w?;
                self.st_emit(out, ListOp::MuKern(w))?;
            }
            "mskip" => {
                let spec = self.st_glue_spec(lx, "mskip", true)?;
                self.st_emit(
                    out,
                    ListOp::Glue {
                        spec,
                        mu: true,
                        leaders: None,
                    },
                )?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// §1191's `push_math(math_left_group)` and the group's body, to the
    /// `\right` that ends it.
    fn st_left_group(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        self.eng.compile_time_begin_group(GroupKind::MathLeft, lx.line());
        self.st_push(CMode::Math);
        let mark = self.globals.len();
        let known_mark = self.eng.known.mark();
        let mut body = self.block(lx, None)?;
        self.st_pop();
        self.eng.compile_time_end_group()?;
        let after = self.eng.take_after_group();
        lx.push_back(&after);
        let (saves, keeps) = self.group_parts(&mut body, mark)?;
        self.known_after_group(known_mark, &saves, &keeps);
        out.push(Cmd::Group { saves, keeps, body });
        Ok(())
    }
}

/// What `scan_math` found a control sequence to be.
enum MathCode {
    Code(Num),
    /// An active character was put back to be read as what it means.
    Reread,
}

fn char_of(code: i64) -> R<char> {
    u32::try_from(code)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| TexError(format!("Bad character code ({code})")))
}
