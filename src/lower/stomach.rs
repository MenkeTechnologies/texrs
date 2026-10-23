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
//! The outer vertical list is built here when [`State::paragraphs`] is on
//! (a plain document, `Lowerer::with_list_path`); otherwise a character at the
//! outer level goes to the string path. Math mode's own cases are in
//! `math.rs` beside this file.

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
    /// `vmode`: the page's own list, when the paragraph builder is on.
    Vertical,
    /// `-vmode`: a `\vbox`, `\vtop`, `\insert`, `\vadjust` or the output
    /// routine.
    InternalVertical,
    /// `hmode`: a paragraph.
    Horizontal,
    /// `-hmode`: an `\hbox`, or one of a discretionary's lists.
    RestrictedHorizontal,
    /// `-mmode`: a formula in text, a subformula, or an equation number.
    Math,
    /// `mmode`: a displayed formula.
    DisplayMath,
}

impl CMode {
    pub(super) fn horizontal(self) -> bool {
        matches!(self, CMode::Horizontal | CMode::RestrictedHorizontal)
    }

    pub(super) fn vertical(self) -> bool {
        matches!(self, CMode::Unmanaged | CMode::Vertical | CMode::InternalVertical)
    }

    pub(super) fn math(self) -> bool {
        matches!(self, CMode::Math | CMode::DisplayMath)
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
    pub(super) nest: Vec<CMode>,
    /// The integer parameters and space factor codes the program has been
    /// told, so each list action carries only what changed.
    synced: Vec<i64>,
    synced_sf: rustc_hash::FxHashMap<char, i64>,
    /// How many fonts the program has been told about.
    fonts_told: usize,
    /// Whether the last command lowered was a run of characters that the
    /// next character may extend: §1038's lookahead ends at the first token
    /// that is not a character.
    pub(super) run_open: bool,
    /// A `\noboundary` (§1045) waiting for the run it affects.
    no_boundary: bool,
    /// The `\output` token list the program was last given, compiled.
    lowered_output: Option<String>,
    /// Whether a box, a rule or a paragraph has reached the page's own list,
    /// before which the page builder cannot fire.
    page_has_content: bool,
    /// Whether the output routine is being lowered, which no list action
    /// inside it may start again.
    lowering_output: bool,
    /// Set by the command that ends the block being lowered without a `}`:
    /// a formula's closing `$` (§1193) and `\right` (§1191).
    pub(super) end_block: bool,
    /// Whether a `\parshape` has been given, after which §1070's
    /// `normal_paragraph` has one to reset.
    par_shape_given: bool,
    /// Every mark text met so far and its tokens, numbered from 1 in this
    /// order: what `\topmark` and its siblings may hold when the program runs.
    marks: Vec<(String, Vec<Token>)>,
    /// The alignments being read, innermost last.
    pub(super) aligns: Vec<super::align::Align>,
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
            lowered_output: None,
            page_has_content: false,
            lowering_output: false,
            end_block: false,
            par_shape_given: false,
            marks: Vec::new(),
            aligns: Vec::new(),
        }
    }
}

/// A list command's right brace, waiting at the end of its body.
pub(super) enum Closer {
    /// §1086's `package`, for a box opened in a group of this kind (and
    /// §1168's `vcenter_group`).
    Box(GroupKind),
    /// §1100's `insert_group`.
    Insert,
    /// §1119's `build_discretionary` for list `k`.
    Disc(u8),
    /// §1186's `math_group`: a subformula in braces.
    Math,
    /// §1174's `build_choices`, for one of `\mathchoice`'s lists.
    Choice,
}

impl Closer {
    /// The group §1083, §1099, §1117, §1153 and §1172 open for it.
    fn group(&self) -> GroupKind {
        match self {
            Closer::Box(kind) => *kind,
            Closer::Insert => GroupKind::Insert,
            Closer::Disc(_) => GroupKind::Disc,
            Closer::Math => GroupKind::Math,
            Closer::Choice => GroupKind::MathChoice,
        }
    }
}

impl Lowerer {
    pub(super) fn st_mode(&self) -> CMode {
        *self.st.nest.last().unwrap_or(&CMode::Unmanaged)
    }

    /// Open a level of the semantic nest the lowerer follows.
    pub(super) fn st_push(&mut self, mode: CMode) {
        self.st.nest.push(mode);
        self.st_mode_changed();
    }

    /// Close one.
    pub(super) fn st_pop(&mut self) {
        self.st.nest.pop();
        self.st_mode_changed();
    }

    /// Tell the expander the mode, for the mode conditionals (§501).
    fn st_mode_changed(&mut self) {
        use crate::expand::ModeKind;
        self.eng.list_mode = match self.st_mode() {
            _ if !self.st.enabled => None,
            CMode::Unmanaged => None,
            CMode::Vertical => Some((ModeKind::Vertical, false)),
            CMode::InternalVertical => Some((ModeKind::Vertical, true)),
            CMode::Horizontal => Some((ModeKind::Horizontal, false)),
            CMode::RestrictedHorizontal => Some((ModeKind::Horizontal, true)),
            CMode::Math => Some((ModeKind::Math, true)),
            CMode::DisplayMath => Some((ModeKind::Math, false)),
        };
    }

    /// Whether list commands are lowered here at the current level.
    pub(super) fn st_managed(&self) -> bool {
        self.st.enabled && (self.st.paragraphs || self.st_mode() != CMode::Unmanaged)
    }

    /// Emit a list action, with whatever the program has not been told yet
    /// in front of it.
    pub(super) fn st_emit(&mut self, out: &mut Vec<Cmd>, op: ListOp) -> R<()> {
        self.st_sync_values(out);
        // The output routine is lowered where the page builder may next fire
        // it, which is after an action that can build the page (§1076,
        // §1091, §1094, §1100, §1103, §1054) -- as late as the lowerer can
        // place it, so that what it expands has been defined.
        let builds = matches!(
            op,
            ListOp::NewGraf(_)
                | ListOp::EndGraf
                | ListOp::BuildPage
                | ListOp::End
                | ListOp::Penalty(_)
                | ListOp::EndBox
                | ListOp::Fetch { .. }
                | ListOp::EndInsert { .. }
                | ListOp::InitMath { display: true, .. }
                | ListOp::ResumeAfterDisplay
        );
        // A box, a rule or a paragraph on the page's own list is what lets
        // the builder fire (§1000-§1001).
        let contributes = matches!(
            op,
            ListOp::NewGraf(_)
                | ListOp::Rule { leader: false, .. }
                | ListOp::EndBox
                | ListOp::Fetch { .. }
                | ListOp::Unpackage { .. }
        );
        if contributes && self.st_mode() == CMode::Vertical {
            self.st.page_has_content = true;
        }
        if builds {
            self.st_sync(out)?;
        }
        self.st.run_open = false;
        out.push(Cmd::List(op));
        Ok(())
    }

    /// Bring the program's integer parameters, fonts and output routine up
    /// to date.
    fn st_sync(&mut self, out: &mut Vec<Cmd>) -> R<()> {
        self.st_sync_values(out);
        // Before the page has a box or a rule on it the builder cannot fire
        // (§1000 discards glue, kerns and penalties at the top of an empty
        // page), so the routine is not lowered before then.
        if self.st.paragraphs && !self.st.lowering_output && self.st.page_has_content {
            let reg = crate::params::toks_register("output").unwrap_or(0);
            let toks = self.eng.toks.get(&reg).cloned().unwrap_or_default();
            self.st_lower_output(toks, out)?;
        }
        Ok(())
    }

    /// §1025's routine, lowered where the program is told about it: the
    /// `\output` text as one group in internal vertical mode, which §1026
    /// closes with `end_graf` if it left a paragraph open.
    ///
    /// The routine runs whenever the page builder fires it, so nothing the
    /// lowerer knows about the registers here holds when it does: it is
    /// lowered with the register tracker blind (`crate::known`), and what it
    /// assigns is unknown afterwards.
    fn st_lower_output(&mut self, toks: Vec<Token>, out: &mut Vec<Cmd>) -> R<()> {
        if toks.is_empty() {
            if self.st.lowered_output.as_deref() != Some("") {
                self.st.lowered_output = Some(String::new());
                out.push(Cmd::OutputRoutine(None));
            }
            return Ok(());
        }
        // Lowered again at every place the builder may fire it, because
        // what it expands may have been redefined since -- a `\footline`
        // changed after the first page -- and emitted only when what it
        // lowers to is not what the program already holds.
        let keep_slot = self.next_keep_slot;
        let globals = self.globals.len();
        self.st.lowering_output = true;
        let blind = self.eng.known.set_opaque(true);
        let open = std::mem::take(&mut self.st.run_open);
        self.eng.compile_time_begin_group(GroupKind::Output, 0);
        self.st_push(CMode::InternalVertical);
        let mark = self.globals.len();
        let known_mark = self.eng.known.mark();
        let mut body = Vec::new();
        let lowered = self.lower_into(&toks, &mut body);
        if lowered.is_ok() && self.st_mode() == CMode::Horizontal {
            self.st_end_graf(&mut body)?;
        }
        while self.st.nest.len() > 1 && self.st_mode() != CMode::InternalVertical {
            self.st_pop();
        }
        self.st_pop();
        let ended = self.eng.compile_time_end_group();
        self.st.run_open = open;
        self.eng.known.set_opaque(blind);
        self.st.lowering_output = false;
        lowered?;
        ended?;
        let key: String = crate::ir::render(&body)
            .lines()
            .filter(|l| {
                let l = l.trim_start();
                !l.starts_with("KeepGlobal") && !l.starts_with("Line")
            })
            .collect::<Vec<_>>()
            .join("\n");
        if self.st.lowered_output.as_deref() == Some(key.as_str()) {
            self.next_keep_slot = keep_slot;
            self.globals.truncate(globals);
            self.eng.known.undo_to(known_mark, None);
            return Ok(());
        }
        self.st.lowered_output = Some(key);
        let (saves, keeps) = self.group_parts(&mut body, mark)?;
        self.known_after_group(known_mark, &saves, &keeps);
        out.push(Cmd::OutputRoutine(Some(vec![Cmd::Group { saves, keeps, body }])));
        Ok(())
    }

    /// §1091's `new_graf` as the lowerer follows it: the paragraph opens,
    /// and `\everypar` is read next.
    pub(super) fn st_new_graf(&mut self, lx: &mut Lexer, indented: bool, out: &mut Vec<Cmd>) -> R<()> {
        self.st_emit(out, ListOp::NewGraf(indented))?;
        self.st_push(CMode::Horizontal);
        if let Some(reg) = crate::params::toks_register("everypar") {
            if let Some(toks) = self.eng.toks.get(&reg).cloned() {
                lx.push_back(&toks);
            }
        }
        Ok(())
    }

    /// §1096's `end_graf`, when a paragraph is open, with §1070's
    /// `normal_paragraph` after it.
    pub(super) fn st_end_graf(&mut self, out: &mut Vec<Cmd>) -> R<()> {
        if self.st_mode() != CMode::Horizontal {
            return Ok(());
        }
        self.st_emit(out, ListOp::EndGraf)?;
        self.st_pop();
        self.st_normal_paragraph(out);
        Ok(())
    }

    /// The integer parameters and fonts, without the output routine.
    fn st_sync_values(&mut self, out: &mut Vec<Cmd>) {
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
        // §425 and §578: a font's parameters are the lowerer's font memory.
        if let Some(text) = self.st_the_font_quantity(w, work)? {
            return Ok(Some(MsgOp::Text(text)));
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

    /// `\the\fontdimen n f`, `\the\hyphenchar f` and `\the\skewchar f`,
    /// printed: font memory is the lowerer's, so the value is known now.
    pub(super) fn st_the_font_quantity(&mut self, w: CsId, lx: &mut Lexer) -> R<Option<String>> {
        Ok(match w.name() {
            "fontdimen" => {
                let (f, n) = self.st_find_font_dimen(lx, false)?;
                let v = self.eng.fonts.get(f).param(n as usize);
                Some(format!("{}pt", crate::dimen::print_scaled(v)))
            }
            "hyphenchar" | "skewchar" => {
                let f = self.st_scan_font_ident(lx)?;
                let font = self.eng.fonts.get(f);
                Some(match w.name() {
                    "hyphenchar" => font.hyphen_char,
                    _ => font.skew_char,
                }
                .to_string())
            }
            _ => None,
        })
    }

    /// A `\let` copy of a character, as the character `main_control`
    /// dispatches on (§1045 reads `cur_cmd`, which is the category), where
    /// the stomach builds the lists; any other token as it is.
    pub(super) fn st_implicit(&self, t: Token) -> Token {
        if !self.st_managed() {
            return t;
        }
        match t {
            Token::Cs(n) => match self.eng.meanings.get(&n) {
                Some(Meaning::Char(c, cat)) => Token::Char(*c, *cat),
                _ => t,
            },
            _ => t,
        }
    }

    /// Whether a character token of category `cat` is this file's to lower.
    pub(super) fn st_takes(&self, cat: Cat) -> bool {
        self.st_managed() && matches!(cat, Cat::Letter | Cat::Other | Cat::Space)
    }

    /// A character token in a list this file manages; `false` when it is
    /// not this file's to lower.
    pub(super) fn st_char(
        &mut self,
        lx: &mut Lexer,
        c: char,
        cat: Cat,
        out: &mut Vec<Cmd>,
    ) -> R<bool> {
        if !self.st_managed() {
            return Ok(false);
        }
        match cat {
            Cat::Letter | Cat::Other => {}
            Cat::Space => {
                self.st.run_open = false;
                if self.st_mode().horizontal() {
                    let f = self.eng.cur_font;
                    self.st_emit(out, ListOp::Space { f, normal: false })?;
                }
                return Ok(true);
            }
            _ => return Ok(false),
        }
        self.st_letter(lx, c, out)?;
        Ok(true)
    }

    /// §1045's `hmode+letter` and `hmode+other_char` (and `char_given`):
    /// extend the run of characters the main loop is reading, or start one.
    fn st_letter(&mut self, lx: &mut Lexer, c: char, out: &mut Vec<Cmd>) -> R<()> {
        if self.st_mode().vertical() {
            // §1090: a letter in vertical mode starts a paragraph, and is
            // read again after `\everypar`.
            self.st_new_graf(lx, true, out)?;
        }
        let f = self.eng.cur_font;
        // The program must hear `c`'s space factor code before the main loop
        // reads `c` (§1034's `adjust_space_factor`), and it must NOT end the
        // run to do so: §1038's lookahead continues over any character, and
        // the ligature/kern program of the character before `c` is consulted
        // with `c` as its right neighbour (§1039) whatever the two
        // characters' `\sfcode`s are. So a code not yet told goes in FRONT of
        // the open run, where the program applies it before the run is read.
        // Ending the run instead ended the lookahead at the right boundary,
        // and cmr10's `y.` lost the kern between its two characters -- the
        // first time only, since the code was then synced.
        let sf = self.st_sf_change(c);
        if self.st.run_open {
            if let Some(at) = out.iter().rposition(|c| !matches!(c, Cmd::Line(_))) {
                if let Cmd::List(ListOp::Chars { f: g, text, .. }) = &mut out[at] {
                    if *g == f {
                        text.push(c);
                        if let Some(code) = sf {
                            out.insert(at, Cmd::List(ListOp::SfCodes(vec![code])));
                        }
                        return Ok(());
                    }
                }
            }
        }
        let cancel_left = std::mem::take(&mut self.st.no_boundary);
        self.st_sync(out)?;
        if let Some(code) = sf {
            out.push(Cmd::List(ListOp::SfCodes(vec![code])));
        }
        out.push(Cmd::List(ListOp::Chars {
            f,
            text: c.to_string(),
            cancel_left,
            no_right: false,
        }));
        self.st.run_open = true;
        Ok(())
    }

    /// `c`'s space factor code when it is not what the program last heard:
    /// the pair the program is to be told, `None` when it already knows.
    fn st_sf_change(&mut self, c: char) -> Option<(char, i64)> {
        let now = self.eng.charcodes.get(crate::charcodes::Table::Space, c);
        let initex = match c.is_ascii_uppercase() {
            true => 999,
            false => 1000,
        };
        let was = self.st.synced_sf.get(&c).copied().unwrap_or(initex);
        if now == was {
            return None;
        }
        self.st.synced_sf.insert(c, now);
        Some((c, now))
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
        if self.st_managed() {
            if let Some(Meaning::CharDef(code)) = self.eng.meanings.get(&name) {
                let code = *code;
                self.st.run_open = was_open;
                return self.st_char_code(lx, code, out).map(|_| true);
            }
        }
        // §1130: `\halign` and `\valign` where they begin an alignment, and
        // the alignment's commands where none is being read (§1127-§1129).
        if self.st_managed() && self.st_mode() != CMode::Unmanaged {
            match name.name() {
                "halign" if self.st_mode().vertical() => {
                    self.st_init_align(lx, true, out)?;
                    return Ok(true);
                }
                "valign" if self.st_mode().horizontal() => {
                    self.st_init_align(lx, false, out)?;
                    return Ok(true);
                }
                "halign" if self.st_mode() == CMode::DisplayMath => {
                    match self.eng.innermost_group() == Some(GroupKind::MathShift) {
                        true => self.st_init_align(lx, true, out)?,
                        false => {
                            lx.push_back(&[Token::Char('}', Cat::EndGroup), Token::Cs(name)]);
                            self.eng.report(lx, "Missing } inserted");
                        }
                    }
                    return Ok(true);
                }
                "cr" | "crcr" | "span" | "noalign" | "omit" => {
                    if self.st_misplaced_align(lx, name) {
                        return Ok(true);
                    }
                }
                _ => {}
            }
        }
        // §1046-§1047 and the cases of §1045 that only math mode has.
        if self.st_math() {
            if insert_dollar_sign(name.name()) {
                lx.push_back(&[Token::Char('$', Cat::MathShift), Token::Cs(name)]);
                self.eng.report(lx, "Missing $ inserted");
                return Ok(true);
            }
            if self.st_math_command(lx, name, out)? {
                return Ok(true);
            }
        }
        let managed = self.st_managed();
        let mode = self.st_mode();
        if managed && mode != CMode::Unmanaged {
            // §1090: a command that belongs to a paragraph begins one when it
            // meets vertical mode, and is read again inside it.
            if mode.vertical() && begins_paragraph(name.name()) {
                match name.name() {
                    "indent" | "noindent" => {
                        self.st_new_graf(lx, name.name() == "indent", out)?;
                    }
                    _ => {
                        lx.push_back(&[Token::Cs(name)]);
                        self.st_new_graf(lx, true, out)?;
                    }
                }
                return Ok(true);
            }
            // §1094-§1095's `head_for_vmode`: a command that belongs to the
            // vertical list ends the paragraph first -- by inserting `\par`,
            // whatever it means now -- or, in an `\hbox`, closes the box.
            if mode.horizontal() && ends_paragraph(name.name()) {
                if mode == CMode::Horizontal {
                    lx.push_back(&[Token::cs("par"), Token::Cs(name)]);
                } else if name.name() != "hrule" {
                    // §1064's `off_save`.
                    lx.push_back(&[Token::Char('}', Cat::EndGroup), Token::Cs(name)]);
                    self.eng.report(lx, "Missing } inserted");
                } else {
                    let esc = self.eng.esc().to_string();
                    self.eng.report(
                        lx,
                        &format!("You can't use `{esc}hrule' here except with leaders"),
                    );
                }
                return Ok(true);
            }
        }
        match name.name() {
            // §1094: `\par` ends the paragraph (§1096) and builds the page;
            // in vertical mode it is `normal_paragraph`, and the page is
            // built after it in the outer list.
            "par" if managed && mode == CMode::Horizontal => self.st_end_graf(out)?,
            "par" if managed && mode == CMode::Vertical => {
                self.st_normal_paragraph(out);
                self.st_emit(out, ListOp::BuildPage)?;
            }
            // §1054: `\end` on the page's own list ships what is left; the
            // rest of the lowerer then stops reading, as `its_all_over` does.
            "end" if managed && mode == CMode::Vertical => {
                self.st_emit(out, ListOp::End)?;
                self.st.run_open = false;
                return Ok(false);
            }
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
                self.st_emit(out, ListOp::SetBoxDimen { n, which, value })?;
                self.st_after_assignment(lx);
            }
            "showbox" => self.st_show_box(lx, out)?,
            // §1245-§1246: the page builder's quantities, assigned in place.
            "deadcycles" | "insertpenalties" if managed => {
                let _ = self.eng.take_global_prefix();
                self.eng.skip_equals_file(lx)?;
                let value = self.number(lx)?;
                let which = u8::from(name.name() == "insertpenalties");
                self.st_emit(out, ListOp::AlterInteger { which, value })?;
                self.st_after_assignment(lx);
            }
            "pagegoal" | "pagetotal" | "pagestretch" | "pagefilstretch" | "pagefillstretch" | "pagefilllstretch" | "pageshrink" | "pagedepth" if managed => {
                let _ = self.eng.take_global_prefix();
                self.eng.skip_equals_file(lx)?;
                let value = self.dimen_number(lx, false)?;
                let which = match name.name() {
                    "pagegoal" => 0,
                    "pagetotal" => 1,
                    "pagestretch" => 2,
                    "pagefilstretch" => 3,
                    "pagefillstretch" => 4,
                    "pagefilllstretch" => 5,
                    "pageshrink" => 6,
                    _ => 7,
                };
                self.st_emit(out, ListOp::AlterPageSoFar { which, value })?;
                self.st_after_assignment(lx);
            }
            "showlists" if managed && mode != CMode::Unmanaged => self.st_show_lists(lx, out)?,
            // §1248's `set_shape`.
            "parshape" => {
                let global = self.eng.take_global_prefix();
                self.eng.skip_equals_file(lx)?;
                let n = self.eng.scan_number_file(lx)?;
                let mut dims = Vec::new();
                for _ in 0..n.max(0) {
                    dims.push(self.dimen_number(lx, false)?);
                    dims.push(self.dimen_number(lx, false)?);
                }
                self.st.par_shape_given = true;
                self.st_emit(out, ListOp::ParShape { global, dims })?;
                self.st_after_assignment(lx);
            }
            // §960 and §934: the text between the braces, for the program's
            // hyphenation pass. §961 reads it with `get_x_token`; texrs reads
            // the characters, which is what a pattern file holds.
            "patterns" | "hyphenation" => {
                let text = self.group_chars(lx)?;
                let op = match name.name() {
                    "patterns" => ListOp::Patterns(text),
                    _ => ListOp::Hyphenation(text),
                };
                self.st_emit(out, op)?;
            }
            // §1253's `assign_font_int`: global, as every font assignment is.
            "hyphenchar" | "skewchar" => {
                let _ = self.eng.take_global_prefix();
                let f = self.st_scan_font_ident(lx)?;
                self.eng.skip_equals_file(lx)?;
                let value = self.number(lx)?;
                if let (Num::Literal(v), Some(font)) = (&value, self.eng.fonts.get_mut(f)) {
                    match name.name() {
                        "hyphenchar" => font.hyphen_char = *v,
                        _ => font.skew_char = *v,
                    }
                }
                let op = match name.name() {
                    "hyphenchar" => ListOp::SetHyphenChar { f, value },
                    _ => ListOp::SetSkewChar { f, value },
                };
                self.st_emit(out, op)?;
                self.st_after_assignment(lx);
            }
            // §1253's `assign_font_dimen`, through §578's `find_font_dimen`.
            "fontdimen" => {
                let _ = self.eng.take_global_prefix();
                let (f, n) = self.st_find_font_dimen(lx, true)?;
                self.eng.skip_equals_file(lx)?;
                let value = self.dimen_number(lx, false)?;
                if let (Num::Literal(v), Some(font)) = (&value, self.eng.fonts.get_mut(f)) {
                    let i = (n as usize).saturating_sub(1);
                    if i < font.params.len() {
                        font.params[i] = *v;
                    }
                }
                self.st_emit(out, ListOp::SetFontDimen { f, n, value })?;
                self.st_after_assignment(lx);
            }
            // §1234's `def_family`.
            "textfont" | "scriptfont" | "scriptscriptfont" => {
                let global = self.eng.take_global_prefix();
                let size = match name.name() {
                    "textfont" => 0,
                    "scriptfont" => 1,
                    _ => 2,
                };
                let fam = self.eng.scan_number_file(lx)?;
                if !(0..16).contains(&fam) {
                    self.eng.report(lx, &format!("Bad number ({fam})"));
                }
                self.eng.skip_equals_file(lx)?;
                let f = self.st_scan_font_ident(lx)?;
                self.eng.define_family(size, (fam & 15) as u8, f, global);
                self.st_after_assignment(lx);
            }
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
            "raise" | "lower" if managed && (mode.horizontal() || mode.math()) => {
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
            "hskip" | "hfil" | "hfill" | "hss" | "hfilneg" if managed && (mode.horizontal() || mode.math()) => {
                let spec = self.st_glue_spec(lx, name.name(), false)?;
                self.st_emit(
                    out,
                    ListOp::Glue {
                        spec,
                        mu: false,
                        leaders: None,
                    },
                )?;
            }
            "vskip" | "vfil" | "vfill" | "vss" | "vfilneg" if managed && mode.vertical() => {
                let spec = self.st_glue_spec(lx, name.name(), false)?;
                self.st_emit(
                    out,
                    ListOp::Glue {
                        spec,
                        mu: false,
                        leaders: None,
                    },
                )?;
            }
            "kern" if managed => {
                let w = self.dimen_number(lx, false)?;
                self.st_emit(out, ListOp::Kern(w))?;
            }
            "penalty" if managed => {
                let p = self.number(lx)?;
                self.st_emit(out, ListOp::Penalty(p))?;
            }
            "unpenalty" | "unkern" | "unskip" if managed => {
                let what = match name.name() {
                    "unpenalty" => crate::stomach::build::Removed::Penalty,
                    "unkern" => crate::stomach::build::Removed::Kern,
                    _ => crate::stomach::build::Removed::Glue,
                };
                self.st_emit(out, ListOp::DeleteLast(what))?;
            }
            "hrule" if managed && mode.vertical() => {
                let op = self.st_rule(lx, false, false)?;
                self.st_emit(out, op)?;
            }
            "vrule" if managed && (mode.horizontal() || mode.math()) => {
                let op = self.st_rule(lx, true, false)?;
                self.st_emit(out, op)?;
            }
            "unhbox" | "unhcopy" if managed && mode.horizontal() => {
                let n = self.eng.scan_register_file(lx)? as u16;
                let copy = name.name() == "unhcopy";
                self.st_emit(out, ListOp::Unpackage { n, copy })?;
            }
            "unvbox" | "unvcopy" if managed && mode.vertical() => {
                let n = self.eng.scan_register_file(lx)? as u16;
                let copy = name.name() == "unvcopy";
                self.st_emit(out, ListOp::Unpackage { n, copy })?;
            }
            "number" | "romannumeral" | "the" if managed && mode != CMode::Unmanaged => {
                self.st.run_open = was_open;
                return self.st_converted(lx, name.name(), out);
            }
            "/" if managed && (mode.horizontal() || mode.math()) => {
                self.st_emit(out, ListOp::ItalicCorrection)?
            }
            "-" if managed && (mode.horizontal() || mode.math()) => {
                let f = self.eng.cur_font;
                self.st_emit(out, ListOp::Hyphen(f))?;
            }
            "discretionary" if managed && (mode.horizontal() || mode.math()) => {
                self.st_emit(out, ListOp::BeginDiscretionary)?;
                for k in 0..3u8 {
                    self.st_list_body(lx, CMode::RestrictedHorizontal, Closer::Disc(k), out)?;
                }
            }
            "mark" if managed => self.st_make_mark(lx, out)?,
            "topmark" | "firstmark" | "botmark" | "splitfirstmark" | "splitbotmark"
                if managed && mode != CMode::Unmanaged =>
            {
                self.st.run_open = false;
                self.st_insert_mark(name.name(), out)?;
                self.st.run_open = false;
            }
            "insert" if managed => {
                let mut n = self.eng.scan_register_file(lx)? as u16;
                if n == 255 {
                    self.eng.report(lx, "You can't \\insert255");
                    n = 0;
                }
                self.st_emit(out, ListOp::BeginInsert(n))?;
                self.st_list_body(lx, CMode::InternalVertical, Closer::Insert, out)?;
            }
            "vadjust" if managed && (mode.horizontal() || mode.math()) => {
                self.st_emit(out, ListOp::BeginInsert(255))?;
                self.st_list_body(lx, CMode::InternalVertical, Closer::Insert, out)?;
            }
            // §1093: an empty box `\parindent` wide.
            "indent" if managed && (mode.horizontal() || mode.math()) => {
                self.st_emit(out, ListOp::Indent)?
            }
            "noindent" if managed && (mode.horizontal() || mode.math()) => {}
            "noboundary" if managed && mode.math() => {}
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
            "accent" if managed && mode.horizontal() => self.st_make_accent(lx, out)?,
            "char" if managed && mode.horizontal() => {
                let Num::Literal(code) = self.number(lx)? else {
                    return Err(TexError(
                        "\\char needs a number known while lowering".into(),
                    ));
                };
                self.st.run_open = was_open;
                self.st_char_code(lx, code, out)?;
            }
            " " if managed && (mode.horizontal() || mode.math()) => {
                let f = self.eng.cur_font;
                self.st_emit(out, ListOp::Space { f, normal: true })?;
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
                self.st_emit(out, op)?;
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
    fn st_char_code(&mut self, lx: &mut Lexer, code: i64, out: &mut Vec<Cmd>) -> R<()> {
        let c = u32::try_from(code)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| TexError(format!("Bad character code ({code})")))?;
        self.st_letter(lx, c, out)
    }

    /// §1269 after an assignment this file made.
    fn st_after_assignment(&mut self, lx: &mut Lexer) {
        if let Some(t) = self.eng.take_after_assignment(true) {
            lx.push_back(&[t]);
        }
    }

    /// §1070's `normal_paragraph`.
    pub(super) fn st_normal_paragraph(&mut self, out: &mut Vec<Cmd>) {
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
        if self.st.par_shape_given {
            out.push(Cmd::List(ListOp::ParShape {
                global: false,
                dims: Vec::new(),
            }));
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
            // A size that reads registers is their followed value: expl3's
            // intarrays are `\font...at\g__intarray_font_int\c__intarray_sp_dim`.
            let size = self.dimen_number(lx, false)?;
            let s = match self.eng.known.num(&size) {
                Some(v) => v,
                None => {
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
        // LuaTeX's `define_font` callback answers first, when one is
        // registered (`crate::node::hooks`).
        let lua_size = match size {
            crate::stomach::font::Size::At(s) => s,
            crate::stomach::font::Size::Scaled(m) => -m,
        };
        let defined = match crate::node::hooks::define_font(&mut self.eng, lx, &file, lua_size) {
            Some(crate::node::hooks::Defined::Font(font)) => {
                let mut font = *font;
                font.id_text = id.clone();
                Some((self.eng.fonts.add(font), true))
            }
            Some(crate::node::hooks::Defined::Existing(f))
                if (f as usize) < self.eng.fonts.fonts.len() =>
            {
                Some((f, false))
            }
            _ => self.eng.fonts.load(&file, size, &id),
        };
        match defined {
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
                    self.st_emit(out, op)?;
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
        self.st_sync(out)?;
        self.st_after_assignment(lx);
        Ok(())
    }

    /// The glue of `\hskip`, `\hfil` and their kin (§1058-§1060), as the
    /// four slots a glue register holds.
    pub(super) fn st_glue_spec(&mut self, lx: &mut Lexer, name: &str, mu: bool) -> R<[Num; 4]> {
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
        if let Some((from, negative)) = self.peek_glue_register(lx, mu)? {
            return Ok([
                super::signed(negative, from),
                super::signed(negative, from + 1),
                super::signed(negative, from + 2),
                Num::Count(from + 3),
            ]);
        }
        let g = match mu {
            true => self.eng.scan_muglue_parts(lx)?,
            false => self.eng.scan_glue_parts(lx)?,
        };
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
    pub(super) fn st_next_non_blank(&mut self, lx: &mut Lexer) -> R<Option<(Token, Option<CsId>)>> {
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
                self.st_emit(out, ListOp::Fetch { from, context })?;
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
                self.st_emit(out, op)?;
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
    pub(super) fn st_begin_box(
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
        let moved = matches!(context, Context::Shift(_));
        self.st_emit(
            out,
            ListOp::BeginBox {
                kind,
                exactly,
                size,
                context,
            },
        )?;
        let mode = match kind {
            BoxKind::H => CMode::RestrictedHorizontal,
            _ => CMode::InternalVertical,
        };
        // §1083: an `\hbox` moved or appended in vertical mode is an adjusted
        // one, whose `\vadjust` and `\insert` material migrates out (§1076).
        let group = match kind {
            BoxKind::H if self.st_mode().vertical() && moved => {
                GroupKind::AdjustedHbox
            }
            BoxKind::H => GroupKind::Hbox,
            BoxKind::V => GroupKind::Vbox,
            BoxKind::Top => GroupKind::Vtop,
            BoxKind::VCenter => GroupKind::VCenter,
        };
        self.st_list_body(lx, mode, Closer::Box(group), out)
    }

    /// The braces of a list command: `scan_left_brace` (§403), the group
    /// (§274), the `\everyhbox`/`\everyvbox` §1083 inserts, the body in its
    /// mode, and what the `}` does (§1085, §1100, §1119).
    pub(super) fn st_list_body(
        &mut self,
        lx: &mut Lexer,
        mode: CMode,
        closer: Closer,
        out: &mut Vec<Cmd>,
    ) -> R<()> {
        // §403: the left brace, skipping spaces and `\relax`.
        match self.st_next_non_blank(lx)? {
            Some((Token::Char(_, Cat::BeginGroup), _)) => self.st_align_brace(),
            Some((t, p)) if matches!(self.st_implicit(t), Token::Char(_, Cat::BeginGroup)) => {
                let _ = p;
            }
            other => {
                if let Some((t, _)) = other {
                    lx.push_back(&[t]);
                }
                self.eng.report(lx, "Missing { inserted");
            }
        }
        let every = match (&closer, mode) {
            (Closer::Box(_), CMode::RestrictedHorizontal) => crate::params::toks_register("everyhbox"),
            (Closer::Box(_), CMode::InternalVertical) => crate::params::toks_register("everyvbox"),
            _ => None,
        };
        if let Some(reg) = every {
            if let Some(toks) = self.eng.toks.get(&reg).cloned() {
                lx.push_back(&toks);
            }
        }
        self.eng.compile_time_begin_group(closer.group(), lx.line());
        self.st_push(mode);
        self.st.run_open = false;
        let mark = self.globals.len();
        let known_mark = self.eng.known.mark();
        let mut body = self.block(lx, Some(&["\u{0}endgroup"]))?;
        self.st.run_open = false;
        // §1085 and §1100: a `\vbox`, `\vtop` or insertion ends the paragraph
        // it holds before it is packaged.
        if self.st_mode() == CMode::Horizontal {
            self.st_end_graf(&mut body)?;
        }
        // §1086 reads `\boxmaxdepth` and §1100 the split parameters before
        // `unsave`, so they are read inside the group.
        let dimen = |name: &str| Num::Count(crate::params::register_slot(name).unwrap_or(0));
        match closer {
            Closer::Box(_) => body.push(Cmd::List(ListOp::BoxMaxDepth(dimen("boxmaxdepth")))),
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
            Closer::Disc(_) | Closer::Math | Closer::Choice => {}
        }
        self.st_pop();
        self.eng.compile_time_end_group()?;
        let after = self.eng.take_after_group();
        lx.push_back(&after);
        let (saves, keeps) = self.group_parts(&mut body, mark)?;
        self.known_after_group(known_mark, &saves, &keeps);
        // The insertion's own `}` is inside its body: §1100 packages before
        // the group's registers come back.
        out.push(Cmd::Group { saves, keeps, body });
        self.st_sync(out)?;
        match closer {
            Closer::Box(_) => out.push(Cmd::List(ListOp::EndBox)),
            Closer::Disc(k) => out.push(Cmd::List(ListOp::EndDiscList(k))),
            Closer::Math => out.push(Cmd::List(ListOp::EndMathGroup)),
            Closer::Choice => out.push(Cmd::List(ListOp::EndChoiceList)),
            Closer::Insert => {}
        }
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
        let spec = self.st_glue_spec(lx, &prim, false)?;
        self.st_emit(
            out,
            ListOp::Glue {
                spec,
                mu: false,
                leaders: Some(kind),
            },
        )?;
        Ok(())
    }

    /// §1293's `\showlists` (and §1298's ending): the nest as the program
    /// holds it where the command stands.
    fn st_show_lists(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        self.eng.note_mouth(lx);
        self.st_sync(out)?;
        let context = self.eng.error_context(lx).replace('\n', "");
        let online = self.eng.intpars.get(crate::intpar::TRACING_ONLINE) > 0;
        let ops = match online {
            true => vec![MsgOp::ShowLists, MsgOp::Report(format!("! OK.{context}"))],
            false => vec![MsgOp::Report(format!("! OK (see the transcript file).{context}"))],
        };
        self.queue_report(ops);
        self.flush_reports(out);
        Ok(())
    }

    /// §1296's `\showbox` (and §1298's ending).
    fn st_show_box(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        let n = self.eng.scan_register_file(lx)? as u16;
        self.eng.note_mouth(lx);
        self.st_sync(out)?;
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
        Cmd::List(ListOp::ParShape { global: false, .. }) => true,
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

/// §1090's commands: in vertical mode each begins a paragraph and is read
/// again inside it (letters and `\chardef` characters are the main loop's).
fn begins_paragraph(name: &str) -> bool {
    matches!(
        name,
        "char"
            | "unhbox"
            | "unhcopy"
            | "vrule"
            | "accent"
            | "discretionary"
            | "-"
            | "hskip"
            | "hfil"
            | "hfill"
            | "hss"
            | "hfilneg"
            | "valign"
            | "noboundary"
            | " "
            | "indent"
            | "noindent"
    )
}

/// §1046's `insert_dollar_sign`: a command of vertical mode met in a
/// formula, before which `$` is inserted.
fn insert_dollar_sign(name: &str) -> bool {
    matches!(
        name,
        "par" | "end" | "vskip" | "vfil" | "vfill" | "vss" | "vfilneg" | "hrule" | "unvbox"
            | "unvcopy" | "moveleft" | "moveright" | "dump"
    )
}

/// §1094's commands: in horizontal mode each ends the paragraph first
/// (`head_for_vmode`, §1095).
fn ends_paragraph(name: &str) -> bool {
    matches!(
        name,
        "end" | "vskip" | "vfil" | "vfill" | "vss" | "vfilneg" | "hrule" | "unvbox" | "unvcopy" | "halign"
    )
}

impl Lowerer {
    /// Turn the outer list over to the stomach: see [`Lowerer::with_list_path`].
    pub(super) fn st_enable_paragraphs(&mut self) {
        self.st.paragraphs = true;
        if let Some(outer) = self.st.nest.first_mut() {
            *outer = CMode::Vertical;
        }
        self.st_mode_changed();
    }
}

impl Lowerer {
    /// §577's `scan_font_ident`: a font identifier, `\font` (the current
    /// font), or a family member; anything else is `Missing font identifier`,
    /// read again, and `\nullfont`.
    fn st_scan_font_ident(&mut self, lx: &mut Lexer) -> R<u16> {
        let (t, p) = loop {
            let Some((t, p)) = self.eng.next_unexpandable(lx)? else {
                return Ok(0);
            };
            if !t.is_space() {
                break (t, p);
            }
        };
        if let Token::Cs(n) = t {
            if let Some(Meaning::Font(f)) = self.eng.meanings.get(&n) {
                return Ok(*f);
            }
            let size = match p.unwrap_or(n).name() {
                "font" => return Ok(self.eng.cur_font),
                "nullfont" => return Ok(0),
                "textfont" => Some(0),
                "scriptfont" => Some(1),
                "scriptscriptfont" => Some(2),
                _ => None,
            };
            if let Some(size) = size {
                let fam = self.eng.scan_number_file(lx)?;
                return Ok(self.eng.fam_fonts[size][(fam & 15) as usize]);
            }
        }
        lx.back_input(&[t]);
        self.eng.report(lx, "Missing font identifier");
        Ok(0)
    }

    /// §578's `find_font_dimen`: `\fontdimen n f`, as the font and the
    /// parameter number. Writing past the last parameter of the font loaded
    /// last makes room for it; past any other font's is §579's error.
    fn st_find_font_dimen(&mut self, lx: &mut Lexer, writing: bool) -> R<(u16, u16)> {
        let n = self.eng.scan_number_file(lx)?;
        let f = self.st_scan_font_ident(lx)?;
        let last = self.eng.fonts.fonts.len().saturating_sub(1) as u16;
        let have = self.eng.fonts.get(f).params.len() as i64;
        let fits = n > 0 && (n <= have || (writing && f == last && f != 0));
        if !fits {
            let esc = self.eng.esc().to_string();
            let id = self.eng.fonts.get(f).id_text.clone();
            self.eng.report(
                lx,
                &format!("Font {esc}{id} has only {have} fontdimen parameters"),
            );
            return Ok((0, 1));
        }
        if n > have {
            if let Some(font) = self.eng.fonts.get_mut(f) {
                font.params.resize(n as usize, 0);
            }
        }
        Ok((f, n as u16))
    }
}

impl Lowerer {
    /// `\number`, `\romannumeral` and `\the` of a register, met in a list.
    ///
    /// They expand to characters (§465, §470), and when the lowerer knows the
    /// register's value the characters are put back to be read as any others
    /// are. When only the program knows it -- `\folio` in an output routine,
    /// whose `\pageno` changes every page -- the characters are made when the
    /// program runs ([`ListOp::Converted`]). Anything else is left to the
    /// expander.
    fn st_converted(&mut self, lx: &mut Lexer, name: &str, out: &mut Vec<Cmd>) -> R<bool> {
        use crate::compiler::{is_glue_slot, is_mu_slot, DIMEN_BASE};
        let (form, values): (u8, [Num; 4]) = match name {
            "the" => {
                let Some(t) = lx.next_token(&self.eng.cats) else {
                    return Ok(false);
                };
                let Token::Cs(n) = t else {
                    lx.push_back(&[t]);
                    return Ok(false);
                };
                let Some((slot, _)) = self.eng.register_slot(lx, n, false)? else {
                    lx.push_back(&[t]);
                    return Ok(false);
                };
                let internal_int = Internal::of_name(n.name()).is_some_and(|i| !i.is_dimen());
                if is_glue_slot(slot) {
                    let form = if is_mu_slot(slot) { 4 } else { 3 };
                    (
                        form,
                        [
                            Num::Count(slot),
                            Num::Count(slot + 1),
                            Num::Count(slot + 2),
                            Num::Count(slot + 3),
                        ],
                    )
                } else if slot < DIMEN_BASE || internal_int {
                    (0, [Num::Count(slot), lit(0), lit(0), lit(0)])
                } else {
                    (2, [Num::Count(slot), lit(0), lit(0), lit(0)])
                }
            }
            _ => {
                let Some(v) = self.peek_int_register(lx, false)? else {
                    return Ok(false);
                };
                let form = if name == "romannumeral" { 1 } else { 0 };
                (form, [v, lit(0), lit(0), lit(0)])
            }
        };
        let known: Option<Vec<i64>> = values.iter().map(|v| self.eng.known.num(v)).collect();
        if let Some(k) = known {
            // Known now: the characters, read next as the mouth would read them.
            let text = match form {
                0 => k[0].to_string(),
                1 => crate::expand::roman_int(k[0]),
                2 => format!("{}pt", crate::dimen::print_scaled(k[0])),
                _ => crate::glue::print_glue_in(
                    if form == 4 { "mu" } else { "pt" },
                    k[0],
                    k[1],
                    k[3] / 4,
                    k[2],
                    k[3] % 4,
                ),
            };
            let toks: Vec<Token> = text
                .chars()
                .map(|c| match c {
                    ' ' => Token::Char(' ', Cat::Space),
                    c => Token::Char(c, Cat::Other),
                })
                .collect();
            lx.push_back(&toks);
            return Ok(true);
        }
        if self.st_mode().vertical() {
            self.st_new_graf(lx, true, out)?;
        }
        let f = self.eng.cur_font;
        self.st_emit(out, ListOp::Converted { f, form, values })?;
        Ok(true)
    }
}

fn lit(v: i64) -> Num {
    Num::Literal(v)
}

impl Lowerer {
    /// Lower a format's source -- plain.tex -- the way INITEX reads it before
    /// `\dump`: everything it defines and assigns is kept for the document,
    /// and what it printed is dropped, as a preloaded format prints nothing.
    pub fn preload_format(&mut self, src: &str) -> R<()> {
        let out = self.lower_located(src);
        self.reports.clear();
        self.reported = false;
        match out {
            Ok(cmds) => {
                super::state_only(&cmds, &mut self.prologue);
                Ok(())
            }
            Err((e, _)) => Err(e),
        }
    }
}

impl Lowerer {
    /// `make_accent` (§1123-§1124) as the lowerer reads it: the accent's
    /// code, §1270's `do_assignments`, and the character it goes over.
    ///
    /// The assignments `do_assignments` carries out here are font selections
    /// and integer parameters, which is what an accent is written with
    /// (`\accent"7F\it e` is `\fam\itfam\tenit`); any other command ends
    /// the scan and is read again after the accent.
    fn st_make_accent(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        let Num::Literal(code) = self.number(lx)? else {
            return Err(TexError("\\accent needs a number known while lowering".into()));
        };
        let accent = u32::try_from(code).unwrap_or(0);
        let f = self.eng.cur_font;
        let next = loop {
            let Some((t, p)) = self.st_next_non_blank(lx)? else {
                break None;
            };
            if let Token::Cs(n) = t {
                let font = match self.eng.meanings.get(&n) {
                    Some(Meaning::Font(g)) => Some(*g),
                    _ if p.unwrap_or(n).name() == "nullfont" => Some(0),
                    _ => None,
                };
                if let Some(g) = font {
                    let global = self.eng.take_global_prefix();
                    self.eng.select_font(g, global);
                    continue;
                }
                if let Some(i) = self.eng.intpar_cs(n) {
                    self.eng.compile_time_intpar(lx, i)?;
                    continue;
                }
            }
            break Some((self.st_implicit(t), p));
        };
        let g = self.eng.cur_font;
        // §1124: a character to put the accent over, or none.
        let over: i64 = match next {
            Some((Token::Char(c, Cat::Letter | Cat::Other), _)) => i64::from(u32::from(c)),
            Some((Token::Cs(n), p)) => match self.eng.meanings.get(&n) {
                Some(Meaning::CharDef(code)) => *code,
                _ if p.unwrap_or(n).name() == "char" => match self.number(lx)? {
                    Num::Literal(v) => v,
                    _ => return Err(TexError("\\char needs a number known while lowering".into())),
                },
                _ => {
                    lx.push_back(&[Token::Cs(n)]);
                    -1
                }
            },
            Some((t, _)) => {
                lx.push_back(&[t]);
                -1
            }
            None => -1,
        };
        self.st_emit(out, ListOp::Accent { f, accent, g, over })
    }
}

impl Lowerer {
    /// `make_mark` (§1101): the text expanded as `scan_toks(false,true)`
    /// expands it, kept as tokens and numbered, for `\topmark` and its
    /// siblings to read again (§386).
    fn st_make_mark(&mut self, lx: &mut Lexer, out: &mut Vec<Cmd>) -> R<()> {
        let (mut body, site) = self.eng.read_message_body_sited(lx)?;
        if matches!(body.last(), Some(Token::Cs(n)) if n.name() == crate::expand::MESSAGE_END) {
            body.pop();
        }
        let mark = self.eng.report_mark();
        let expanded = self.eng.expand_edef_body(&body);
        let (id, text) = match expanded {
            Ok(toks) => {
                let text = self.eng.tokens_text(&toks);
                let id = match self.st.marks.iter().position(|(t, _)| *t == text) {
                    Some(i) => i + 1,
                    None => {
                        self.st.marks.push((text.clone(), toks));
                        self.st.marks.len()
                    }
                };
                (id as u32, text)
            }
            // A text that reads what only the run knows is the message's,
            // and has no tokens to read again.
            Err(_) => {
                self.eng.drain_diagnostics(mark);
                let mut work = Lexer::new("");
                work.push_back(&body);
                work.attach_site(site);
                let parts = self.msg_ops(&mut work, &[])?;
                (0, self.static_text(&parts))
            }
        };
        self.st_emit(out, ListOp::Mark { id, text })
    }

    /// `\topmark` and its siblings (§386) in a list: whichever mark the
    /// program holds then, read as its tokens. Every mark met so far may be
    /// the one, so each is lowered as a branch on the number the stomach
    /// mirrors (`Internal::Mark`).
    fn st_insert_mark(&mut self, which: &str, out: &mut Vec<Cmd>) -> R<()> {
        let k = match which {
            "topmark" => 0,
            "firstmark" => 1,
            "botmark" => 2,
            "splitfirstmark" => 3,
            "splitbotmark" => 4,
            _ => return Ok(()),
        };
        let slot = Internal::Mark(k).slot();
        let marks = self.st.marks.clone();
        if marks.is_empty() {
            return Ok(());
        }
        // In vertical mode a mark's characters would begin a paragraph in one
        // branch and not in another, which the lowerer's one mode cannot follow.
        if self.st_mode().vertical() {
            return Err(TexError(format!(
                "\\{which} in vertical mode, once a mark has been written, is not supported"
            )));
        }
        let nest = self.st.nest.clone();
        let mut chain: Vec<Cmd> = Vec::new();
        for (i, (_, toks)) in marks.iter().enumerate().rev() {
            let known = self.eng.known.mark();
            let mut body = Vec::new();
            self.st.run_open = false;
            self.lower_into(toks, &mut body)?;
            self.eng.known.undo_to(known, None);
            if self.st.nest != nest {
                return Err(TexError(format!(
                    "a mark read by \\{which} that changes the mode is not supported"
                )));
            }
            chain = vec![Cmd::IfNum {
                left: Num::Count(slot),
                rel: crate::ir::Rel::Equal,
                right: Num::Literal(i as i64 + 1),
                then_branch: body,
                else_branch: chain,
            }];
        }
        out.extend(chain);
        Ok(())
    }
}
