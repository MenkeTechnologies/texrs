//! `main_control`'s dispatch (§1045) for the commands that build lists.
//!
//! tex decides what a command does from the pair (mode, command), and the
//! mode is something only the stomach has -- except that every change of mode
//! is itself one of these commands, so the lowerer can follow it exactly: an
//! `\hbox{` opens restricted horizontal mode and its `}` closes it, whatever
//! the program does at run time. So the lowerer keeps its own copy of the
//! semantic nest ([`State::nest`]), dispatches on it here, and emits the
//! action as a [`ListOp`] for `crate::stomach` to carry out when the program
//! runs.
//!
//! What this file decides while lowering is what tex decides while READING:
//! which case of §1045 a token is, the keywords and numbers §1073-§1084 scan,
//! the `\everyhbox` §1083 inserts, and the font a character is in (§1034's
//! `main_f:=cur_font`, an `eqtb` entry the expander scopes). What it leaves to
//! the program is every value a register holds.
//!
//! The outer vertical list is not built here yet: the paragraph builder
//! (§1091-§1096) is not on this path, so a character at the outer level still
//! goes where it went before, and [`State::paragraphs`] is off.

use super::{Lowerer, R};
use crate::catcode::Cat;
use crate::expand::{GroupKind, Meaning, TexError};
use crate::ir::list::{Context, Fetch, ListOp};
use crate::ir::{Cmd, MsgOp, Num};
use crate::lexer::Lexer;
use crate::stomach::{BoxDimen, BoxKind, Internal};
use crate::token::{CsId, Token};

/// `mode` (§211) as the lowerer follows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CMode {
    /// The outer list, while the paragraph builder is off: characters and
    /// paragraphs go where they went before this file existed.
    Unmanaged,
    /// `-vmode`: a `\vbox`, `\vtop`, `\insert` or `\vadjust`.
    InternalVertical,
    /// `-hmode`: an `\hbox`, or one of a discretionary's lists.
    RestrictedHorizontal,
}

impl CMode {
    fn horizontal(self) -> bool {
        self == CMode::RestrictedHorizontal
    }

    fn vertical(self) -> bool {
        matches!(self, CMode::Unmanaged | CMode::InternalVertical)
    }
}

/// The lowerer's half of the stomach.
pub(super) struct State {
    /// Whether this document's list commands are lowered here at all. Off
    /// for a LaTeX document, whose prelude defines `\hbox` and its siblings
    /// as text macros for the PDF path.
    pub enabled: bool,
    /// Whether the outer list is built here (§1091's paragraphs onwards).
    pub paragraphs: bool,
    nest: Vec<CMode>,
    /// The integer parameters and space factor codes the program has been
    /// told, so each list action carries only what changed.
    synced: Vec<i64>,
    synced_sf: rustc_hash::FxHashMap<char, i64>,
    /// How many fonts the program has been told about.
    fonts_told: usize,
    /// Whether the last command lowered was a run of characters that the
    /// next character may extend: §1038's lookahead ends at the first token
    /// that is not a character.
    run_open: bool,
    /// A `\noboundary` (§1045) waiting for the run it affects.
    no_boundary: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            enabled: true,
            paragraphs: false,
            nest: vec![CMode::Unmanaged],
            synced: crate::stomach::initex_int_pars(),
            synced_sf: rustc_hash::FxHashMap::default(),
            fonts_told: 1,
            run_open: false,
            no_boundary: false,
        }
    }
}

/// A list command's right brace, waiting at the end of its body.
enum Closer {
    /// §1086's `package`.
    Box,
    /// §1100's `insert_group`.
    Insert,
    /// §1119's `build_discretionary` for list `k`.
    Disc(u8),
}

impl Lowerer {
    fn st_mode(&self) -> CMode {
        *self.st.nest.last().unwrap_or(&CMode::Unmanaged)
    }

    /// Whether list commands are lowered here at the current level.
    fn st_managed(&self) -> bool {
        self.st.enabled && (self.st.paragraphs || self.st_mode() != CMode::Unmanaged)
    }

    /// Emit a list action, with whatever the program has not been told yet
    /// in front of it.
    fn st_emit(&mut self, out: &mut Vec<Cmd>, op: ListOp) {
        self.st_sync(out);
        self.st.run_open = false;
        out.push(Cmd::List(op));
    }

    /// Bring the program's integer parameters and fonts up to date.
    fn st_sync(&mut self, out: &mut Vec<Cmd>) {
        let mut changed = Vec::new();
        for (i, was) in self.st.synced.iter_mut().enumerate() {
            let now = self.eng.intpars.get(i);
            if *was != now {
                *was = now;
                changed.push((i as u16, now));
            }
        }
        if !changed.is_empty() {
            out.push(Cmd::List(ListOp::IntPars(changed)));
        }
        while self.st.fonts_told < self.eng.fonts.fonts.len() {
            let f = self.st.fonts_told;
            let font = &self.eng.fonts.fonts[f];
            out.push(Cmd::List(ListOp::DefineFont {
                f: f as u16,
                name: font.name.clone(),
                id: font.id_text.clone(),
                size: font.size,
            }));
            self.st.fonts_told += 1;
        }
    }

    /// `\the` of a quantity the stomach holds, for a message.
    pub(super) fn st_the(&mut self, w: CsId, work: &mut Lexer) -> R<Option<MsgOp>> {
        let which = match w.name() {
            "wd" => Some(BoxDimen::Wd),
            "ht" => Some(BoxDimen::Ht),
            "dp" => Some(BoxDimen::Dp),
            _ => None,
        };
        if let Some(which) = which {
            let n = self.eng.scan_register_pending(work)?;
            let slot = crate::stomach::box_dimen_slot(n as u16, which);
            return Ok(Some(MsgOp::Dimen(Num::Count(slot))));
        }
        let Some(i) = Internal::of_name(w.name()) else {
            return Ok(None);
        };
        if i == Internal::OutputPenalty {
            return Ok(None);
        }
        let slot = i.slot();
        Ok(Some(match i {
            Internal::LastSkip => MsgOp::Glue([
                Num::Count(slot),
                Num::Count(slot + 1),
                Num::Count(slot + 2),
                Num::Count(slot + 3),
            ]),
            _ if i.is_dimen() => MsgOp::Dimen(Num::Count(slot)),
            _ => MsgOp::Number(Num::Count(slot)),
        }))
    }

    /// Whether a character token of category `cat` is this file's to lower.
    pub(super) fn st_takes(&self, cat: Cat) -> bool {
        self.st_managed() && matches!(cat, Cat::Letter | Cat::Other | Cat::Space)
    }

    /// A character token in a list this file manages; `false` when it is
    /// not this file's to lower.
    pub(super) fn st_char(&mut self, c: char, cat: Cat, out: &mut Vec<Cmd>) -> R<bool> {
        if !self.st_managed() {
            return Ok(false);
        }
        match cat {
            Cat::Letter | Cat::Other => {}
            Cat::Space => {
                self.st.run_open = false;
                if self.st_mode().horizontal() {
                    let f = self.eng.cur_font;
                    self.st_emit(out, ListOp::Space { f, normal: false });
                }
                return Ok(true);
            }
            _ => return Ok(false),
        }
        self.st_letter(c, out)?;
        Ok(true)
    }

    /// §1045's `hmode+letter` and `hmode+other_char` (and `char_given`):
    /// extend the run of characters the main loop is reading, or start one.
    fn st_letter(&mut self, c: char, out: &mut Vec<Cmd>) -> R<()> {
        if self.st_mode().vertical() {
            // §1090: a letter in vertical mode starts a paragraph.
            return Err(TexError(
                "a paragraph inside a \\vbox needs the paragraph builder, which \
                 texrs's list path does not have yet"
                    .into(),
            ));
        }
        let f = self.eng.cur_font;
        let c = self.st_sf_sync(c, out);
        if self.st.run_open {
            if let Some(Cmd::List(ListOp::Chars { f: g, text, .. })) =
                out.iter_mut().rev().find(|c| !matches!(c, Cmd::Line(_)))
            {
                if *g == f {
                    text.push(c);
                    return Ok(());
                }
            }
        }
        let cancel_left = std::mem::take(&mut self.st.no_boundary);
        self.st_sync(out);
        out.push(Cmd::List(ListOp::Chars {
            f,
            text: c.to_string(),
            cancel_left,
            no_right: false,
        }));
        self.st.run_open = true;
        Ok(())
    }

    /// Tell the program `c`'s space factor code if it is not what the
    /// program last heard.
    fn st_sf_sync(&mut self, c: char, out: &mut Vec<Cmd>) -> char {
        let now = self.eng.charcodes.get(crate::charcodes::Table::Space, c);
        let initex = match c.is_ascii_uppercase() {
            true => 999,
            false => 1000,
        };
        let was = self.st.synced_sf.get(&c).copied().unwrap_or(initex);
        if now != was {
            self.st.synced_sf.insert(c, now);
            // A change ends the run: the program applies it before the next
            // character, which therefore starts a run of its own.
            self.st.run_open = false;
            out.push(Cmd::List(ListOp::SfCodes(vec![(c, now)])));
        }
        c
    }

    /// Wrap a group's lowered body in the box-register save level the
    /// program needs when it assigns a box register locally (§274, §283).
    pub(super) fn st_group_body(&mut self, body: &mut Vec<Cmd>) {
        self.st.run_open = false;
        if self.st.enabled && assigns_box_locally(body) {
            body.insert(0, Cmd::List(ListOp::GroupBegin));
            body.push(Cmd::List(ListOp::GroupEnd));
        }
    }

    /// Dispatch a primitive (or a font identifier) to its §1045 case, if it
    /// is one this file lowers. `Ok(false)` leaves it to the rest of the
    /// lowerer.
    #[inline(never)]
    pub(super) fn st_command(&mut self, lx: &mut Lexer, name: CsId, out: &mut Vec<Cmd>) -> R<bool> {
        let was_open = std::mem::take(&mut self.st.run_open);
        if !self.st.enabled {
            return Ok(false);
        }
        // A font identifier selects its font (§1217).
        if let Some(Meaning::Font(f)) = self.eng.meanings.get(&name) {
            let f = *f;
            let global = self.eng.take_global_prefix();
            self.eng.select_font(f, global);
            self.st_after_assignment(lx);
            return Ok(true);
        }
        // §1045's `char_given`: a `\chardef` name is a character.
        if self.st_managed() && self.st_mode().horizontal() {
            if let Some(Meaning::CharDef(code)) = self.eng.meanings.get(&name) {
                let code = *code;
                self.st.run_open = was_open;
                return self.st_char_code(code, out).map(|_| true);
            }
        }
        let managed = self.st_managed();
        let mode = self.st_mode();
        match name.name() {
            "nullfont" => {
                let global = self.eng.take_global_prefix();
                self.eng.select_font(0, global);
                self.st_after_assignment(lx);
            }
            "font" => self.st_new_font(lx, out)?,
            "setbox" => {
                let global = self.eng.take_global_prefix();
                let n = self.eng.scan_register_file(lx)? as u16;
                self.eng.skip_equals_file(lx)?;
                self.st_scan_box(lx, Context::SetBox { n, global }, out)?;
            }
            "wd" | "ht" | "dp" => {
                // §1247: `\wd n=` changes the box in place.
                let which = match name.name() {
                    "wd" => BoxDimen::Wd,
                    "ht" => BoxDimen::Ht,
                    _ => BoxDimen::Dp,
                };
                let _ = self.eng.take_global_prefix();
                let n = self.eng.scan_register_file(lx)? as u16;
                self.eng.skip_equals_file(lx)?;
                let value = self.dimen_number(lx, false)?;
                self.st_emit(out, ListOp::SetBoxDimen { n, which, value });
                self.st_after_assignment(lx);
            }
            "showbox" => self.st_show_box(lx, out)?,
            "ifvoid" | "ifhbox" | "ifvbox" => {
                let negated = self.eng.take_unless();
                let n = self.eng.scan_register_file(lx)?;
                let (left, right) = Self::st_box_test(name.name(), n);
                self.eng.set_unless(negated);
                self.flush_reports(out);
                let (then_branch, else_branch) = self.arms(lx)?;
                out.push(Cmd::IfNum {
                    left,
                    rel: crate::ir::Rel::Equal,
                    right,
                    then_branch,
                    else_branch,
                });
                self.st.run_open = was_open;
            }
            "hbox" | "vbox" | "vtop" | "box" | "copy" | "lastbox" | "vsplit" if managed => {
                lx.push_back(&[Token::Cs(name)]);
                self.st_scan_box(lx, Context::Shift(Num::Literal(0)), out)?;
            }
            "raise" | "lower" if managed && mode.horizontal() => {
                let d = self.dimen_number(lx, false)?;
                let shift = match name.name() {
                    "raise" => negate(d)?,
                    _ => d,
                };
                self.st_scan_box(lx, Context::Shift(shift), out)?;
            }
            "moveleft" | "moveright" if managed && mode.vertical() => {
                let d = self.dimen_number(lx, false)?;
                let shift = match name.name() {
                    "moveleft" => negate(d)?,
                    _ => d,
                };
                self.st_scan_box(lx, Context::Shift(shift), out)?;
            }
            "shipout" => self.st_scan_box(lx, Context::ShipOut, out)?,
            "leaders" | "cleaders" | "xleaders" if managed => {
                let kind = match name.name() {
                    "leaders" => crate::node::LeaderKind::Aligned,
                    "cleaders" => crate::node::LeaderKind::Centred,
                    _ => crate::node::LeaderKind::Expanded,
                };
                self.st_scan_box(lx, Context::Leaders(kind), out)?;
                self.st_leader_glue(lx, kind, out)?;
            }
            "hskip" | "hfil" | "hfill" | "hss" | "hfilneg" if managed && mode.horizontal() => {
                let spec = self.st_glue_spec(lx, name.name())?;
                self.st_emit(
                    out,
                    ListOp::Glue {
                        spec,
                        mu: false,
                        leaders: None,
                    },
                );
            }
            "vskip" | "vfil" | "vfill" | "vss" | "vfilneg" if managed && mode.vertical() => {
                let spec = self.st_glue_spec(lx, name.name())?;
                self.st_emit(
                    out,
                    ListOp::Glue {
                        spec,
                        mu: false,
                        leaders: None,
                    },
                );
            }
            "kern" if managed => {
                let w = self.dimen_number(lx, false)?;
                self.st_emit(out, ListOp::Kern(w));
            }
            "penalty" if managed => {
                let p = self.number(lx)?;
                self.st_emit(out, ListOp::Penalty(p));
            }
            "unpenalty" | "unkern" | "unskip" if managed => {
                let what = match name.name() {
                    "unpenalty" => crate::stomach::build::Removed::Penalty,
                    "unkern" => crate::stomach::build::Removed::Kern,
                    _ => crate::stomach::build::Removed::Glue,
                };
                self.st_emit(out, ListOp::DeleteLast(what));
            }
            "hrule" if managed && mode.vertical() => {
                let op = self.st_rule(lx, false, false)?;
                self.st_emit(out, op);
            }
            "vrule" if managed && mode.horizontal() => {
                let op = self.st_rule(lx, true, false)?;
                self.st_emit(out, op);
            }
            "unhbox" | "unhcopy" if managed && mode.horizontal() => {
                let n = self.eng.scan_register_file(lx)? as u16;
                let copy = name.name() == "unhcopy";
                self.st_emit(out, ListOp::Unpackage { n, copy });
            }
            "unvbox" | "unvcopy" if managed && mode.vertical() => {
                let n = self.eng.scan_register_file(lx)? as u16;
                let copy = name.name() == "unvcopy";
                self.st_emit(out, ListOp::Unpackage { n, copy });
            }
            "/" if managed && mode.horizontal() => self.st_emit(out, ListOp::ItalicCorrection),
            "-" if managed && mode.horizontal() => {
                let f = self.eng.cur_font;
                self.st_emit(out, ListOp::Hyphen(f));
            }
            "discretionary" if managed && mode.horizontal() => {
                self.st_emit(out, ListOp::BeginDiscretionary);
                for k in 0..3u8 {
                    self.st_list_body(lx, CMode::RestrictedHorizontal, Closer::Disc(k), out)?;
                }
            }
            "mark" if managed => {
                // §1101: the text is expanded as `scan_toks(false,true)`
                // expands it, which is what `\message` does.
                let parts = self.message_parts(lx)?;
                let text = self.static_text(&parts);
                self.st_emit(out, ListOp::Mark(text));
            }
            "insert" if managed => {
                let mut n = self.eng.scan_register_file(lx)? as u16;
                if n == 255 {
                    self.eng.report(lx, "You can't \\insert255");
                    n = 0;
                }
                self.st_emit(out, ListOp::BeginInsert(n));
                self.st_list_body(lx, CMode::InternalVertical, Closer::Insert, out)?;
            }
            "vadjust" if managed && mode.horizontal() => {
                self.st_emit(out, ListOp::BeginInsert(255));
                self.st_list_body(lx, CMode::InternalVertical, Closer::Insert, out)?;
            }
            "indent" if managed && mode.horizontal() => {
                // §1093: an empty box `\parindent` wide.
                self.st_emit(
                    out,
                    ListOp::BeginBox {
                        kind: BoxKind::H,
                        exactly: true,
                        size: Num::Count(crate::params::register_slot("parindent").unwrap_or(0)),
                        context: Context::Shift(Num::Literal(0)),
                    },
                );
                self.st_emit(out, ListOp::EndBox);
            }
            "noindent" if managed && mode.horizontal() => {}
            "noboundary" if managed && mode.horizontal() => {
                // §1045: before a run it cancels the left boundary; after
                // one, §1038 drops the right.
                if was_open {
                    if let Some(Cmd::List(ListOp::Chars { no_right, .. })) =
                        out.iter_mut().rev().find(|c| !matches!(c, Cmd::Line(_)))
                    {
                        *no_right = true;
                    }
                } else {
                    self.st.no_boundary = true;
                }
            }
            "char" if managed && mode.horizontal() => {
                let Num::Literal(code) = self.number(lx)? else {
                    return Err(TexError(
                        "\\char needs a number known while lowering".into(),
                    ));
                };
                self.st.run_open = was_open;
                self.st_char_code(code, out)?;
            }
            " " if managed && mode.horizontal() => {
                let f = self.eng.cur_font;
                self.st_emit(out, ListOp::Space { f, normal: true });
            }
            "par" if managed => {
                // §1094: `\par` in restricted horizontal mode ends nothing;
                // in internal vertical mode it is `normal_paragraph`.
                if mode.vertical() {
                    self.st_normal_paragraph(out);
                }
            }
            "prevdepth" | "spacefactor" | "prevgraf" if managed => {
                self.eng.skip_equals_file(lx)?;
                let op = match name.name() {
                    "prevdepth" => ListOp::SetPrevDepth(self.dimen_number(lx, false)?),
                    "spacefactor" => ListOp::SetSpaceFactor(self.number(lx)?),
                    _ => ListOp::SetPrevGraf(self.number(lx)?),
                };
                self.st_emit(out, op);
                self.st_after_assignment(lx);
            }
            _ => {
                // §1038's lookahead EXPANDS what it meets, so an expandable
                // command between two characters leaves the run open.
                self.st.run_open = was_open && self.eng.is_expandable(name);
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether `\ifvoid`, `\ifhbox` and `\ifvbox` test the program's
    /// registers. For a LaTeX document they stay what they were -- decided
    /// while lowering, every register void -- because its boxes are not built
    /// as lists.
    pub(super) fn st_box_conditionals(&self) -> bool {
        self.st.enabled
    }

    /// §505: the comparison `\ifvoid n`, `\ifhbox n` or `\ifvbox n` makes,
    /// as a slot and the kind it must hold.
    pub(super) fn st_box_test(conditional: &str, n: i64) -> (Num, Num) {
        let kind = match conditional {
            "ifvoid" => crate::stomach::KIND_VOID,
            "ifhbox" => crate::stomach::KIND_HBOX,
            _ => crate::stomach::KIND_VBOX,
        };
        let slot = crate::stomach::box_kind_slot(n as u16);
        (Num::Count(slot), Num::Literal(kind))
    }

    /// A character by code in the current font (§1045's `char_num` and
    /// `char_given`, §1038's lookahead +1).
    fn st_char_code(&mut self, code: i64, out: &mut Vec<Cmd>) -> R<()> {
        let c = u32::try_from(code)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| TexError(format!("Bad character code ({code})")))?;
        self.st_letter(c, out)
    }

    /// §1269 after an assignment this file made.
    fn st_after_assignment(&mut self, lx: &mut Lexer) {
        if let Some(t) = self.eng.take_after_assignment(true) {
            lx.push_back(&[t]);
        }
    }

    /// §1070's `normal_paragraph`.
    fn st_normal_paragraph(&mut self, out: &mut Vec<Cmd>) {
        if let Some(i) = crate::intpar::index("looseness") {
            if self.eng.intpars.get(i) != 0 {
                self.eng.assign_intpar(i, 0);
            }
        }
        if let Some(i) = crate::intpar::index("hangafter") {
            if self.eng.intpars.get(i) != 1 {
                self.eng.assign_intpar(i, 1);
            }
        }
        if let Some(slot) = crate::params::register_slot("hangindent") {
            out.push(Cmd::SetCount(slot, Num::Literal(0)));
        }
    }

    /// §1257-§1260's `new_font`.
    fn st_new_font(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        let global = self.eng.take_global_prefix();
        let u = self.eng.get_r_token(lx, false)?;
        let id = u.name().to_string();
        // §1257: the identifier means `\nullfont` while the size is scanned.
        self.eng.define_font_identifier(u, 0, global);
        self.eng.skip_equals_file(lx)?;
        let file = self.scan_file_name(lx)?;
        // §1258.
        let size = if self.eng.scan_keyword(lx, "at", false)? {
            let s = match self.dimen_number(lx, false)? {
                Num::Literal(v) => v,
                _ => {
                    return Err(TexError(
                        "a font size must be known while lowering".into(),
                    ))
                }
            };
            // §1259.
            match s > 0 && s < 0o1000000000 {
                true => crate::stomach::font::Size::At(s),
                false => {
                    self.eng.report(
                        lx,
                        &format!(
                            "Improper `at' size ({}pt), replaced by 10pt",
                            crate::dimen::print_scaled(s)
                        ),
                    );
                    crate::stomach::font::Size::At(10 * crate::dimen::UNITY)
                }
            }
        } else if self.eng.scan_keyword(lx, "scaled", false)? {
            let m = self.eng.scan_number_file(lx)?;
            match m > 0 && m <= 32768 {
                true => crate::stomach::font::Size::Scaled(m),
                false => {
                    self.eng
                        .report(lx, "Illegal magnification has been changed to 1000");
                    crate::stomach::font::Size::Scaled(1000)
                }
            }
        } else {
            crate::stomach::font::Size::Scaled(1000)
        };
        match self.eng.fonts.load(&file, size, &id) {
            Some((f, fresh)) => {
                if !fresh {
                    // The newest name is the one of record (§1257), and the
                    // program shows it, so it is told again.
                    let font = self.eng.fonts.get(f);
                    let op = ListOp::DefineFont {
                        f,
                        name: font.name.clone(),
                        id: font.id_text.clone(),
                        size: font.size,
                    };
                    self.st_emit(out, op);
                }
                self.eng.define_font_identifier(u, f, global);
            }
            None => {
                // §561.
                let esc = self.eng.esc().to_string();
                self.eng.report(
                    lx,
                    &format!(
                        "Font {esc}{id}={file} not loadable: Metric (TFM) file not found"
                    ),
                );
            }
        }
        self.st_sync(out);
        self.st_after_assignment(lx);
        Ok(())
    }

    /// The glue of `\hskip`, `\hfil` and their kin (§1058-§1060), as the
    /// four slots a glue register holds.
    fn st_glue_spec(&mut self, lx: &mut Lexer, name: &str) -> R<[Num; 4]> {
        use crate::box_::fill;
        let fixed = match name {
            "hfil" | "vfil" => Some(fill::fil()),
            "hfill" | "vfill" => Some(fill::fill()),
            "hss" | "vss" => Some(fill::ss()),
            "hfilneg" | "vfilneg" => Some(fill::fil_neg()),
            _ => None,
        };
        if let Some(g) = fixed {
            return Ok([
                Num::Literal(g.natural),
                Num::Literal(g.stretch),
                Num::Literal(g.shrink),
                Num::Literal(g.stretch_order * 4 + g.shrink_order),
            ]);
        }
        if let Some((from, negative)) = self.peek_glue_register(lx, false)? {
            return Ok([
                super::signed(negative, from),
                super::signed(negative, from + 1),
                super::signed(negative, from + 2),
                Num::Count(from + 3),
            ]);
        }
        let g = self.eng.scan_glue_parts(lx)?;
        Ok([
            super::scaled_num(g.natural),
            super::scaled_num(g.stretch),
            super::scaled_num(g.shrink),
            Num::Literal(g.stretch_order * 4 + g.shrink_order),
        ])
    }

    /// §463's `scan_rule_spec`.
    fn st_rule(&mut self, lx: &mut Lexer, vrule: bool, leader: bool) -> R<ListOp> {
        let (mut width, mut height, mut depth) = (None, None, None);
        loop {
            if self.eng.scan_keyword(lx, "width", false)? {
                width = Some(self.dimen_number(lx, false)?);
            } else if self.eng.scan_keyword(lx, "height", false)? {
                height = Some(self.dimen_number(lx, false)?);
            } else if self.eng.scan_keyword(lx, "depth", false)? {
                depth = Some(self.dimen_number(lx, false)?);
            } else {
                break;
            }
        }
        Ok(ListOp::Rule {
            vrule,
            width,
            height,
            depth,
            leader,
        })
    }

    /// The next token that is not a space or `\relax`, expanded (§404's "Get
    /// the next non-blank non-relax non-call token").
    fn st_next_non_blank(&mut self, lx: &mut Lexer) -> R<Option<(Token, Option<CsId>)>> {
        loop {
            let Some((t, p)) = self.eng.next_unexpandable(lx)? else {
                return Ok(None);
            };
            if t.is_space() {
                continue;
            }
            if let Token::Cs(n) = t {
                if p.unwrap_or(n).name() == "relax" {
                    continue;
                }
            }
            return Ok(Some((t, p)));
        }
    }

    /// `scan_box` (§1084): the box after `\setbox n=`, `\raise d`,
    /// `\leaders` or `\shipout`, or the box command itself.
    fn st_scan_box(&mut self, lx: &mut Lexer, context: Context, out: &mut Vec<Cmd>) -> R<()> {
        let Some((t, p)) = self.st_next_non_blank(lx)? else {
            return Err(TexError("A <box> was supposed to be here".into()));
        };
        let prim = match t {
            Token::Cs(n) => p.unwrap_or(n).name().to_string(),
            Token::Char(..) => String::new(),
        };
        match prim.as_str() {
            "box" | "copy" | "lastbox" | "vsplit" => {
                let from = match prim.as_str() {
                    "box" => Fetch::Box(self.eng.scan_register_file(lx)? as u16),
                    "copy" => Fetch::Copy(self.eng.scan_register_file(lx)? as u16),
                    "lastbox" => Fetch::LastBox,
                    _ => {
                        // §1082.
                        let n = self.eng.scan_register_file(lx)? as u16;
                        if !self.eng.scan_keyword(lx, "to", false)? {
                            self.eng.report(lx, "Missing `to' inserted");
                        }
                        Fetch::Vsplit(n, self.dimen_number(lx, false)?)
                    }
                };
                self.st_emit(out, ListOp::Fetch { from, context });
                Ok(())
            }
            "hbox" | "vbox" | "vtop" => {
                let kind = match prim.as_str() {
                    "hbox" => BoxKind::H,
                    "vbox" => BoxKind::V,
                    _ => BoxKind::Top,
                };
                self.st_begin_box(lx, kind, context, out)
            }
            "hrule" | "vrule" if matches!(context, Context::Leaders(_)) => {
                let op = self.st_rule(lx, prim == "vrule", true)?;
                self.st_emit(out, op);
                Ok(())
            }
            _ => {
                lx.push_back(&[t]);
                self.eng.report(lx, "A <box> was supposed to be here");
                Ok(())
            }
        }
    }

    /// §1083: `\hbox`, `\vbox` or `\vtop`, its spec, its group, its list and
    /// its `}` (§1085-§1086).
    fn st_begin_box(
        &mut self,
        lx: &mut Lexer,
        kind: BoxKind,
        context: Context,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        // §645's `scan_spec`: `to <dimen>`, `spread <dimen>`, or neither.
        let (exactly, size) = if self.eng.scan_keyword(lx, "to", false)? {
            (true, self.dimen_number(lx, false)?)
        } else if self.eng.scan_keyword(lx, "spread", false)? {
            (false, self.dimen_number(lx, false)?)
        } else {
            (false, Num::Literal(0))
        };
        self.st_emit(
            out,
            ListOp::BeginBox {
                kind,
                exactly,
                size,
                context,
            },
        );
        let mode = match kind {
            BoxKind::H => CMode::RestrictedHorizontal,
            _ => CMode::InternalVertical,
        };
        self.st_list_body(lx, mode, Closer::Box, out)
    }

    /// The braces of a list command: `scan_left_brace` (§403), the group
    /// (§274), the `\everyhbox`/`\everyvbox` §1083 inserts, the body in its
    /// mode, and what the `}` does (§1085, §1100, §1119).
    fn st_list_body(
        &mut self,
        lx: &mut Lexer,
        mode: CMode,
        closer: Closer,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        // §403: the left brace, skipping spaces and `\relax`.
        match self.st_next_non_blank(lx)? {
            Some((Token::Char(_, Cat::BeginGroup), _)) => {}
            other => {
                if let Some((t, _)) = other {
                    lx.push_back(&[t]);
                }
                self.eng.report(lx, "Missing { inserted");
            }
        }
        let every = match (&closer, mode) {
            (Closer::Box, CMode::RestrictedHorizontal) => crate::params::toks_register("everyhbox"),
            (Closer::Box, CMode::InternalVertical) => crate::params::toks_register("everyvbox"),
            _ => None,
        };
        if let Some(reg) = every {
            if let Some(toks) = self.eng.toks.get(&reg).cloned() {
                lx.push_back(&toks);
            }
        }
        // A box's group closes on `}` as §1085 has it, which is the simple
        // group's rule here (`lower.rs` checks `innermost_group`).
        self.eng.compile_time_begin_group(GroupKind::Simple, lx.line());
        self.st.nest.push(mode);
        self.st.run_open = false;
        let mark = self.globals.len();
        let known_mark = self.eng.known.mark();
        let mut body = self.block(lx, Some(&["\u{0}endgroup"]))?;
        self.st.run_open = false;
        // §1086 reads `\boxmaxdepth` and §1100 the split parameters before
        // `unsave`, so they are read inside the group.
        let dimen = |name: &str| Num::Count(crate::params::register_slot(name).unwrap_or(0));
        match closer {
            Closer::Box => body.push(Cmd::List(ListOp::BoxMaxDepth(dimen("boxmaxdepth")))),
            Closer::Insert => {
                let s = crate::params::register_slot("splittopskip").unwrap_or(0);
                let floating = crate::intpar::index("floatingpenalty")
                    .map_or(0, |i| self.eng.intpars.get(i));
                body.push(Cmd::List(ListOp::EndInsert {
                    split_top_skip: [
                        Num::Count(s),
                        Num::Count(s + 1),
                        Num::Count(s + 2),
                        Num::Count(s + 3),
                    ],
                    split_max_depth: dimen("splitmaxdepth"),
                    floating_penalty: Num::Literal(floating),
                }));
            }
            Closer::Disc(_) => {}
        }
        self.st.nest.pop();
        self.eng.compile_time_end_group()?;
        let after = self.eng.take_after_group();
        lx.push_back(&after);
        let (saves, keeps) = self.group_parts(&mut body, mark)?;
        self.known_after_group(known_mark, &saves, &keeps);
        // The insertion's own `}` is inside its body: §1100 packages before
        // the group's registers come back.
        let insert = matches!(closer, Closer::Insert);
        out.push(Cmd::Group { saves, keeps, body });
        self.st_sync(out);
        match closer {
            Closer::Box => out.push(Cmd::List(ListOp::EndBox)),
            Closer::Disc(k) => out.push(Cmd::List(ListOp::EndDiscList(k))),
            Closer::Insert => {}
        }
        let _ = insert;
        Ok(())
    }

    /// §1078: the glue a leader box must be followed by.
    fn st_leader_glue(
        &mut self,
        lx: &mut Lexer,
        kind: crate::node::LeaderKind,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        let Some((t, p)) = self.st_next_non_blank(lx)? else {
            return Ok(());
        };
        let prim = match t {
            Token::Cs(n) => p.unwrap_or(n).name().to_string(),
            Token::Char(..) => String::new(),
        };
        let horizontal = matches!(prim.as_str(), "hskip" | "hfil" | "hfill" | "hss" | "hfilneg");
        let vertical = matches!(prim.as_str(), "vskip" | "vfil" | "vfill" | "vss" | "vfilneg");
        let fits = match self.st_mode().vertical() {
            true => vertical,
            false => horizontal,
        };
        if !fits {
            lx.push_back(&[t]);
            self.eng.report(lx, "Leaders not followed by proper glue");
            return Ok(());
        }
        let spec = self.st_glue_spec(lx, &prim)?;
        self.st_emit(
            out,
            ListOp::Glue {
                spec,
                mu: false,
                leaders: Some(kind),
            },
        );
        Ok(())
    }

    /// §1296's `\showbox` (and §1298's ending).
    fn st_show_box(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        let n = self.eng.scan_register_file(lx)? as u16;
        self.eng.note_mouth(lx);
        self.st_sync(out);
        let context = self.eng.error_context(lx).replace('\n', "");
        let online = self.eng.intpars.get(crate::intpar::TRACING_ONLINE) > 0;
        let ops = match online {
            true => vec![
                MsgOp::Report(format!("> {}box{n}=", self.eng.esc())),
                MsgOp::ShowBox(n),
                MsgOp::Report(format!("! OK.{context}")),
            ],
            // §1298: the display went to the log alone.
            false => vec![MsgOp::Report(format!(
                "! OK (see the transcript file).{context}"
            ))],
        };
        // Placed here, not with the next message: the box is shown as it
        // stands at this point of the run.
        self.queue_report(ops);
        self.flush_reports(out);
        Ok(())
    }
}

/// `-d`, for `\raise` and `\moveleft` (§1073 scans the dimension and
/// negates it).
fn negate(d: Num) -> R<Num> {
    Ok(match d {
        Num::Literal(v) => Num::Literal(-v),
        Num::Count(s) => Num::Neg(s),
        Num::Neg(s) => Num::Count(s),
        Num::Scaled { int, frac, reg } => Num::Scaled {
            int: -int,
            frac: -frac,
            reg,
        },
        Num::ByCount { factor, sign, unit } => Num::ByCount {
            factor,
            sign: -sign,
            unit,
        },
        // Anything computed is negated by computing `0-d`, with the range
        // check `\dimexpr` makes (etex.ch's `add_or_sub`).
        other @ (Num::Rust { .. } | Num::Expr { .. } | Num::ExprEnd { .. }) => Num::Expr {
            op: crate::etex_arith::Op::Sub,
            max: crate::node::MAX_DIMEN,
            args: vec![Num::Literal(0), other],
        },
    })
}

/// Whether `cmds` assigns a box register locally anywhere, which is what a
/// group around them must save and restore.
fn assigns_box_locally(cmds: &[Cmd]) -> bool {
    cmds.iter().any(|c| match c {
        Cmd::List(ListOp::BeginBox { context, .. }) | Cmd::List(ListOp::Fetch { context, .. }) => {
            matches!(context, Context::SetBox { global: false, .. })
        }
        Cmd::Group { body, .. } | Cmd::Loop { body, .. } | Cmd::Color { body, .. } => {
            assigns_box_locally(body)
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
        } => assigns_box_locally(then_branch) || assigns_box_locally(else_branch),
        _ => false,
    })
}
