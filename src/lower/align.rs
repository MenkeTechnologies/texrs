//! `\halign` and `\valign` as the lowerer reads them (§768-§799, §1126-§1133).
//!
//! The preamble is the lowerer's: §777-§784 scan it into a template pair per
//! column, and each entry is read as tex reads it -- the column's u template,
//! then the entry's text, and, once `&`, `\span` or `\cr` ends it at brace
//! level zero (§789), the v template and the frozen `\endtemplate` (§780),
//! whose arrival is §1131's `do_endv`. What the program does with the
//! entries -- unset boxes, column widths, setting them (§791-§812) -- is
//! `crate::stomach::align`'s.
//!
//! The groups are tex's: the alignment's own (§774, opened by `scan_spec`),
//! and one per entry, which `fin_col` ends and opens again (§791), so an
//! assignment in an entry lasts to the end of it and one in a `\noalign`
//! lasts into the next row's first entry.

use super::stomach::CMode;
use super::{Lowerer, R};
use crate::catcode::Cat;
use crate::expand::{GroupKind, Meaning, TexError};
use crate::ir::list::ListOp;
use crate::ir::{Cmd, Num};
use crate::lexer::Lexer;
use crate::token::{CsId, Token};

/// The frozen `\endtemplate` (§780), which ends every v template.
const END_TEMPLATE: &str = "\u{0}endtemplate";
/// Where a u template ends, which is where §324 sets `align_state` to zero.
const U_END: &str = "\u{0}u-template-end";

/// One column's templates (§779).
#[derive(Clone, Debug)]
struct Column {
    u: Vec<Token>,
    v: Vec<Token>,
}

/// An entry being read (§788-§789).
#[derive(Clone, Debug)]
struct Entry {
    /// §324: the u template has been read, so braces count from here.
    u_done: bool,
    /// `align_state` for the entry's own text: braces opened and not closed.
    braces: i64,
    /// `\omit` (§788): no templates.
    omit: bool,
    /// What ended the entry: 0 `&`, 1 `\span`, 2 `\cr` or `\crcr`.
    ended: Option<u8>,
    /// Whether the v template is in (§789), so the next `\endtemplate` ends
    /// the entry.
    in_v: bool,
}

/// An alignment being read (§770).
#[derive(Clone, Debug)]
pub(super) struct Align {
    halign: bool,
    columns: Vec<Column>,
    /// §783's `cur_loop`: the column the preamble repeats from, and the
    /// column the next lengthening copies.
    repeat: Option<(usize, usize)>,
    /// `cur_align`, as a column number.
    cur: usize,
    entry: Option<Entry>,
}

impl Lowerer {
    fn tabskip_slots(&self) -> [Num; 4] {
        let s = crate::params::register_slot("tabskip").unwrap_or(0);
        [Num::Count(s), Num::Count(s + 1), Num::Count(s + 2), Num::Count(s + 3)]
    }

    /// The primitive a token means, for the alignment's own commands.
    fn st_meaning_of(&self, t: &Token) -> Option<String> {
        let Token::Cs(n) = t else {
            return None;
        };
        let p = match self.eng.meanings.get(n) {
            Some(Meaning::Primitive(p)) => *p,
            Some(_) => return None,
            None => *n,
        };
        Some(p.name().to_string())
    }

    /// §1130's `init_align`, from `\halign` or `\valign` (§774-§777).
    pub(super) fn st_init_align(&mut self, lx: &mut Lexer, halign: bool, out: &mut Vec<Cmd>) -> R<()> {
        let display = self.st_mode() == CMode::DisplayMath;
        // §645's `scan_spec`.
        let (exactly, size) = if self.eng.scan_keyword(lx, "to", false)? {
            (true, self.dimen_number(lx, false)?)
        } else if self.eng.scan_keyword(lx, "spread", false)? {
            (false, self.dimen_number(lx, false)?)
        } else {
            (false, Num::Literal(0))
        };
        self.st_emit(out, ListOp::InitAlign { halign, exactly, size })?;
        let mode = match halign {
            true => CMode::InternalVertical,
            false => CMode::RestrictedHorizontal,
        };
        self.st_push(mode);
        self.eng.compile_time_begin_group(GroupKind::Align, lx.line());
        let outer_mark = self.globals.len();
        let outer_known = self.eng.known.mark();
        let mut outer: Vec<Cmd> = Vec::new();
        match self.st_next_non_blank(lx)?.map(|(t, p)| (self.st_implicit(t), p)) {
            Some((Token::Char(_, Cat::BeginGroup), _)) => {}
            other => {
                if let Some((t, _)) = other {
                    lx.push_back(&[t]);
                }
                self.eng.report(lx, "Missing { inserted");
            }
        }
        self.st.aligns.push(Align {
            halign,
            columns: Vec::new(),
            repeat: None,
            cur: 0,
            entry: None,
        });
        self.st_preamble(lx, &mut outer)?;
        outer.push(Cmd::List(ListOp::AlignEntries));
        // The entries' group (§774's `new_save_level(align_group)`).
        let mut entry = self.st_open_entry_group(lx);
        self.st_every_cr(lx);
        // §785-§799: rows until the `}` that ends the alignment.
        let mut entry_body: Vec<Cmd> = Vec::new();
        loop {
            let Some(t) = self.st_align_peek(lx, &mut entry_body)? else {
                break;
            };
            self.st_emit(&mut entry_body, ListOp::InitRow)?;
            if let Some(a) = self.st.aligns.last_mut() {
                a.cur = 0;
            }
            let mut next = t;
            loop {
                let ended = self.st_read_entry(lx, next, &mut entry_body)?;
                let kind = self.st_end_of_preamble_check(lx, ended, &mut entry_body)?;
                if kind == 1 {
                    // §791: a `\span` keeps the entry's group and list open.
                    self.st_emit(&mut entry_body, ListOp::FinCol(1))?;
                } else {
                    self.st_close_entry_group(entry, &mut entry_body, &mut outer)?;
                    outer.push(Cmd::List(ListOp::FinCol(kind)));
                    entry = self.st_open_entry_group(lx);
                    if kind == 2 {
                        outer.push(Cmd::List(ListOp::FinRow));
                        self.st_every_cr(lx);
                        break;
                    }
                }
                if let Some(a) = self.st.aligns.last_mut() {
                    a.cur += 1;
                }
                next = self.st_next_non_call(lx)?;
            }
        }
        // §800: `fin_align`, after the entries' group and the alignment's.
        self.st_close_entry_group(entry, &mut entry_body, &mut outer)?;
        self.st.aligns.pop();
        self.st_pop();
        self.eng.compile_time_end_group()?;
        let after = self.eng.take_after_group();
        lx.push_back(&after);
        let (saves, keeps) = self.group_parts(&mut outer, outer_mark)?;
        self.known_after_group(outer_known, &saves, &keeps);
        out.push(Cmd::Group {
            saves,
            keeps,
            body: outer,
        });
        let indent = match display {
            true => Num::Count(crate::params::register_slot("displayindent").unwrap_or(0)),
            false => Num::Literal(0),
        };
        self.st_emit(out, ListOp::FinAlign(indent))?;
        if display {
            self.st_finish_display_alignment(lx, out)?;
        }
        Ok(())
    }

    /// §1206: an alignment that was a display ends with `$$`, after which the
    /// display does, and the paragraph resumes (`st_formula` resumes it).
    fn st_finish_display_alignment(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        // `do_assignments` is not carried out here; the `$$` is checked.
        let mut dollars = 0;
        while dollars < 2 {
            match self.eng.next_unexpandable(lx)? {
                Some((Token::Char(_, Cat::MathShift), _)) => dollars += 1,
                Some((t, _)) if t.is_space() && dollars == 0 => {}
                other => {
                    if let Some((t, _)) = other {
                        lx.back_input(&[t]);
                    }
                    let msg = match dollars {
                        0 => "Missing $$ inserted",
                        _ => "Display math should end with $$",
                    };
                    self.eng.report(lx, msg);
                    break;
                }
            }
        }
        self.st_emit(out, ListOp::FinishDisplayAlignment)?;
        self.st.end_block = true;
        Ok(())
    }

    /// §777-§784: the preamble, to its `\cr`.
    fn st_preamble(&mut self, lx: &mut Lexer, outer: &mut Vec<Cmd>) -> R<()> {
        loop {
            // §778.
            outer.push(Cmd::List(ListOp::AlignTabskip(self.tabskip_slots())));
            // §783: the u template, to its `#`.
            let mut u: Vec<Token> = Vec::new();
            let mut periodic = false;
            loop {
                let t = self.st_preamble_token(lx, outer)?;
                if matches!(t, Token::Char(_, Cat::Param)) {
                    break;
                }
                let m = self.st_meaning_of(&t);
                let ends = matches!(t, Token::Char(_, Cat::AlignTab))
                    || matches!(m.as_deref(), Some("span" | "cr" | "crcr"));
                if ends {
                    let first = u.is_empty()
                        && matches!(t, Token::Char(_, Cat::AlignTab))
                        && !self.st.aligns.last().is_some_and(|a| a.repeat.is_some())
                        && !periodic;
                    if first {
                        periodic = true;
                        continue;
                    }
                    lx.back_input(&[t]);
                    self.eng.report(lx, "Missing # inserted in alignment preamble");
                    break;
                }
                if !(t.is_space() && u.is_empty()) {
                    u.push(t);
                }
            }
            if periodic {
                let n = self.st.aligns.last().map_or(0, |a| a.columns.len());
                if let Some(a) = self.st.aligns.last_mut() {
                    a.repeat = Some((n, n));
                }
            }
            outer.push(Cmd::List(ListOp::AlignRecord(periodic)));
            // §784: the v template, to `&`, `\span` or `\cr`.
            let mut v: Vec<Token> = Vec::new();
            let ender = loop {
                let t = self.st_preamble_token(lx, outer)?;
                let m = self.st_meaning_of(&t);
                if matches!(t, Token::Char(_, Cat::AlignTab))
                    || matches!(m.as_deref(), Some("span" | "cr" | "crcr"))
                {
                    break t;
                }
                if matches!(t, Token::Char(_, Cat::Param)) {
                    self.eng.report(lx, "Only one # is allowed per tab");
                    continue;
                }
                v.push(t);
            };
            v.push(Token::cs(END_TEMPLATE));
            if let Some(a) = self.st.aligns.last_mut() {
                a.columns.push(Column { u, v });
            }
            // §778's loop: the tabskip after this column, and on.
            if matches!(self.st_meaning_of(&ender).as_deref(), Some("cr" | "crcr")) {
                outer.push(Cmd::List(ListOp::AlignTabskip(self.tabskip_slots())));
                return Ok(());
            }
        }
    }

    /// §782's `get_preamble_token`: the next preamble token unexpanded, with
    /// `\span` expanding the one after it and `\tabskip` assigned.
    fn st_preamble_token(&mut self, lx: &mut Lexer, outer: &mut Vec<Cmd>) -> R<Token> {
        loop {
            let Some(mut t) = lx.next_token(&self.eng.cats) else {
                return Err(TexError("File ended while scanning preamble of \\halign".into()));
            };
            if let Token::Char(c, Cat::Active) = t {
                if let Some(id) = self.eng.active_meaning(c) {
                    t = Token::Cs(id);
                }
            }
            match self.st_meaning_of(&t).as_deref() {
                Some("span") => {
                    let Some(next) = lx.next_token(&self.eng.cats) else {
                        continue;
                    };
                    if let Token::Cs(n) = next {
                        if self.eng.expand_once(lx, n)? {
                            continue;
                        }
                    }
                    lx.push_back(&[next]);
                    continue;
                }
                Some("tabskip") => {
                    let slot = crate::params::register_slot("tabskip").unwrap_or(0);
                    self.eng.skip_equals_file(lx)?;
                    let cmds = self.glue_assign(lx, slot)?;
                    outer.extend(cmds);
                    continue;
                }
                _ => {}
            }
            if matches!(&t, Token::Cs(n) if n.name() == END_TEMPLATE) {
                return Err(TexError("(interwoven alignment preambles are not allowed)".into()));
            }
            return Ok(t);
        }
    }

    fn st_every_cr(&mut self, lx: &mut Lexer) {
        if let Some(toks) =
            crate::params::toks_register("everycr").and_then(|r| self.eng.toks.get(&r).cloned())
        {
            lx.push_back(&toks);
        }
    }

    /// The entries' group: its compile-time frame, and the marks its body
    /// needs when it closes.
    fn st_open_entry_group(&mut self, lx: &Lexer) -> (usize, usize) {
        self.eng.compile_time_begin_group(GroupKind::Align, lx.line());
        (self.globals.len(), self.eng.known.mark())
    }

    fn st_close_entry_group(
        &mut self,
        (mark, known): (usize, usize),
        body: &mut Vec<Cmd>,
        outer: &mut Vec<Cmd>,
    ) -> R<()> {
        self.eng.compile_time_end_group()?;
        let mut b = std::mem::take(body);
        let (saves, keeps) = self.group_parts(&mut b, mark)?;
        self.known_after_group(known, &saves, &keeps);
        outer.push(Cmd::Group { saves, keeps, body: b });
        Ok(())
    }

    /// §406's "Get the next non-blank non-call token".
    fn st_next_non_call(&mut self, lx: &mut Lexer) -> R<Token> {
        loop {
            match self.eng.next_unexpandable(lx)? {
                Some((t, _)) if t.is_space() => continue,
                Some((t, _)) => return Ok(self.st_implicit(t)),
                None => return Err(TexError("File ended within an alignment".into())),
            }
        }
    }

    /// `align_peek` (§785): a `\noalign`, a row, or the end. `None` is the
    /// `}` that ends the alignment.
    fn st_align_peek(&mut self, lx: &mut Lexer, body: &mut Vec<Cmd>) -> R<Option<Token>> {
        loop {
            let t = self.st_next_non_call(lx)?;
            match self.st_meaning_of(&t).as_deref() {
                Some("noalign") => {
                    self.st_noalign(lx, body)?;
                    continue;
                }
                Some("crcr") => continue,
                _ => {}
            }
            if matches!(t, Token::Char(_, Cat::EndGroup)) {
                return Ok(None);
            }
            return Ok(Some(t));
        }
    }

    /// §785's `\noalign`: its group, in the alignment's own mode, and §1133's
    /// end of it.
    fn st_noalign(&mut self, lx: &mut Lexer, body: &mut Vec<Cmd>) -> R<()> {
        match self.st_next_non_blank(lx)?.map(|(t, p)| (self.st_implicit(t), p)) {
            Some((Token::Char(_, Cat::BeginGroup), _)) => {}
            other => {
                if let Some((t, _)) = other {
                    lx.push_back(&[t]);
                }
                self.eng.report(lx, "Missing { inserted");
            }
        }
        self.eng.compile_time_begin_group(GroupKind::NoAlign, lx.line());
        let mark = self.globals.len();
        let known = self.eng.known.mark();
        let mut inner = Vec::new();
        if self.st.aligns.last().is_some_and(|a| a.halign) {
            self.st_normal_paragraph(&mut inner);
        }
        let saved = self.st.aligns.last_mut().and_then(|a| a.entry.take());
        let mut b = self.block(lx, Some(&["\u{0}endgroup"]))?;
        if let Some(a) = self.st.aligns.last_mut() {
            a.entry = saved;
        }
        inner.append(&mut b);
        if self.st_mode() == CMode::Horizontal {
            self.st_end_graf(&mut inner)?;
        }
        self.eng.compile_time_end_group()?;
        self.st_group_body(&mut inner);
        let (saves, keeps) = self.group_parts(&mut inner, mark)?;
        self.known_after_group(known, &saves, &keeps);
        body.push(Cmd::Group {
            saves,
            keeps,
            body: inner,
        });
        Ok(())
    }

    /// One entry (§788-§789, §1131), from the token `init_col` was given:
    /// what ended it.
    fn st_read_entry(&mut self, lx: &mut Lexer, t: Token, body: &mut Vec<Cmd>) -> R<u8> {
        let (halign, column) = {
            let a = self.st.aligns.last().ok_or_else(|| TexError("no alignment".into()))?;
            (a.halign, a.columns.get(a.cur).cloned())
        };
        let omit = matches!(self.st_meaning_of(&t).as_deref(), Some("omit"));
        let mode = match halign {
            true => CMode::RestrictedHorizontal,
            false => CMode::InternalVertical,
        };
        self.st_push(mode);
        let nest_len = self.st.nest.len();
        if omit {
            // §788: no u template, and §1131 still wants `\endtemplate`.
        } else {
            let mut toks = column.as_ref().map(|c| c.u.clone()).unwrap_or_default();
            toks.push(Token::cs(U_END));
            toks.push(t);
            lx.push_back(&toks);
        }
        if let Some(a) = self.st.aligns.last_mut() {
            a.entry = Some(Entry {
                u_done: omit,
                braces: 0,
                omit,
                ended: None,
                in_v: false,
            });
        }
        self.st.run_open = false;
        let mut b = self.block(lx, None)?;
        self.st.run_open = false;
        body.append(&mut b);
        while self.st.nest.len() > nest_len {
            self.st_pop();
        }
        self.st_pop();
        let ended = self
            .st
            .aligns
            .last_mut()
            .and_then(|a| a.entry.take())
            .and_then(|e| e.ended)
            .unwrap_or(2);
        Ok(ended)
    }

    /// §792-§793: an entry ended by `&` or `\span` past the last column
    /// lengthens a periodic preamble, or is taken as `\cr`.
    fn st_end_of_preamble_check(&mut self, lx: &mut Lexer, kind: u8, body: &mut Vec<Cmd>) -> R<u8> {
        if kind == 2 {
            return Ok(2);
        }
        let Some(a) = self.st.aligns.last_mut() else {
            return Ok(2);
        };
        if a.cur + 1 < a.columns.len() {
            return Ok(kind);
        }
        match a.repeat {
            // The copy is of the column after `cur_loop`, which moves one
            // column on each time; the copies are appended, so the preamble
            // repeats from where `&&` stood.
            Some((start, next)) => {
                let col = a.columns[next].clone();
                a.columns.push(col);
                a.repeat = Some((start, next + 1));
                body.push(Cmd::List(ListOp::AlignLengthen));
                Ok(kind)
            }
            None => {
                let esc = self.eng.esc().to_string();
                self.eng
                    .report(lx, &format!("Extra alignment tab has been changed to {esc}cr"));
                Ok(2)
            }
        }
    }

    /// The alignment's own tokens met while an entry is being read: the end
    /// of the u template, braces counted for `align_state`, `&`, `\span` and
    /// `\cr` at level zero bringing in the v template (§789), and the
    /// `\endtemplate` that ends it (§1131). `Ok(true)` when the token was
    /// taken.
    pub(super) fn st_align_token(&mut self, lx: &mut Lexer, t: &Token, out: &mut Vec<Cmd>) -> R<bool> {
        let meaning = self.st_meaning_of(t);
        let in_group = self.eng.innermost_group();
        let Some(a) = self.st.aligns.last_mut() else {
            return Ok(false);
        };
        let Some(entry) = a.entry.as_mut() else {
            return Ok(false);
        };
        match t {
            Token::Cs(n) if n.name() == U_END => {
                entry.u_done = true;
                entry.braces = 0;
                return Ok(true);
            }
            Token::Cs(n) if n.name() == END_TEMPLATE => {
                if !entry.in_v {
                    return Ok(true);
                }
                // §1131's `do_endv`: the entry's group must be the one open.
                if in_group != Some(GroupKind::Align) {
                    lx.push_back(&[*t]);
                    let esc = self.eng.esc().to_string();
                    let (closer, shown) = match in_group {
                        Some(GroupKind::SemiSimple) => (Token::cs("endgroup"), format!("{esc}endgroup")),
                        Some(GroupKind::MathShift) => (Token::Char('$', Cat::MathShift), "$".into()),
                        _ => (Token::Char('}', Cat::EndGroup), "}".into()),
                    };
                    lx.push_back(&[closer]);
                    self.eng.report(lx, &format!("Missing {shown} inserted"));
                    return Ok(true);
                }
                if self.st_mode() == CMode::Horizontal {
                    self.st_end_graf(out)?;
                }
                self.st.end_block = true;
                return Ok(true);
            }
            Token::Char(_, Cat::BeginGroup) if entry.u_done => {
                entry.braces += 1;
                return Ok(false);
            }
            Token::Char(_, Cat::EndGroup) if entry.u_done => {
                entry.braces -= 1;
                return Ok(false);
            }
            _ => {}
        }
        let implicit_tab = matches!(t, Token::Cs(n)
            if matches!(self.eng.meanings.get(n), Some(Meaning::Char(_, Cat::AlignTab))));
        let Some(a) = self.st.aligns.last_mut() else {
            return Ok(false);
        };
        let Some(entry) = a.entry.as_mut() else {
            return Ok(false);
        };
        let kind = match (t, meaning.as_deref()) {
            (Token::Char(_, Cat::AlignTab), _) => 0,
            _ if implicit_tab => 0,
            (_, Some("span")) => 1,
            (_, Some("cr" | "crcr")) => 2,
            _ => return Ok(false),
        };
        if !entry.u_done || entry.braces != 0 || entry.in_v {
            return Ok(false);
        }
        // §789: the v template, or the omit template, then `\endtemplate`.
        entry.ended = Some(kind);
        entry.in_v = true;
        let omit = entry.omit;
        let cur = a.cur;
        let v = match omit {
            true => vec![Token::cs(END_TEMPLATE)],
            false => a.columns.get(cur).map(|c| c.v.clone()).unwrap_or_default(),
        };
        lx.push_back(&v);
        Ok(true)
    }

    /// Whether `name` is one of the alignment's commands met outside an
    /// entry, which §1127-§1129 report.
    pub(super) fn st_misplaced_align(&mut self, lx: &mut Lexer, name: CsId) -> bool {
        let shown = match self.st_meaning_of(&Token::Cs(name)).as_deref() {
            Some("cr") => "cr",
            Some("crcr") => "crcr",
            Some("span") => "span",
            Some("noalign") => "noalign",
            Some("omit") => "omit",
            _ => return false,
        };
        if matches!(shown, "crcr") && self.st.aligns.is_empty() {
            // §1132: `\crcr` outside an alignment is ignored.
            return true;
        }
        let esc = self.eng.esc().to_string();
        self.eng.report(lx, &format!("Misplaced {esc}{shown}"));
        true
    }
}

impl Lowerer {
    /// Whether an alignment's entry is being read.
    pub(super) fn st_in_align_entry(&self) -> bool {
        self.st.aligns.last().is_some_and(|a| a.entry.is_some())
    }
}

impl Lowerer {
    /// A `{` a list command took (§403's `scan_left_brace`) while an entry
    /// is read: `align_state` counts it, so a `&` inside the box does not end
    /// the entry (its `}` is counted where the block reads it).
    pub(super) fn st_align_brace(&mut self) {
        if let Some(e) = self.st.aligns.last_mut().and_then(|a| a.entry.as_mut()) {
            if e.u_done {
                e.braces += 1;
            }
        }
    }
}

impl Lowerer {
    /// `back_input` of an explicit `{` an entry had counted (§325).
    pub(super) fn st_align_unbrace(&mut self) {
        if let Some(e) = self.st.aligns.last_mut().and_then(|a| a.entry.as_mut()) {
            if e.u_done {
                e.braces -= 1;
            }
        }
    }
}
