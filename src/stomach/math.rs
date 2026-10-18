//! Math mode: §1136-§1206's action routines, which build an mlist, and
//! `after_math`, which turns one into an hlist with `crate::math::mlist`'s
//! `mlist_to_hlist` and puts it in the paragraph or on the page.
//!
//! The lowerer has read what tex reads -- a math code is already a number,
//! a delimiter its 24-bit code, a subformula's braces matched -- so what is
//! here is what each routine does to the semantic nest when the program
//! runs. A field of a noad (§681) that `scan_math` fills is named by
//! [`Target`]: the action before the scan chooses it (`\mathop` its
//! nucleus, `^` a superscript), and the character or subformula after it
//! fills it.

use super::{dimen_par, glue_par, Eqtb, Kind, ListState, Mode, Opened, Said, Stomach};
use crate::math::font::{MathFont, MathFonts, SCRIPT_SCRIPT_SIZE, SCRIPT_SIZE, TEXT_SIZE};
use crate::math::mlist::Converter;
use crate::math::noad::*;
use crate::node::{BoxNode, GlueNode, GlueSource, Node, Scaled, MAX_DIMEN};
use crate::pack::{hpack, Spec, NATURAL};

/// §1151's `var_code`: a math code whose family is `\fam`'s when that is a
/// family.
const VAR_CODE: i64 = 0x7000;

/// §700 and §701: the parameters a symbol and an extension font must have.
const TOTAL_MATHSY_PARAMS: usize = 22;
const TOTAL_MATHEX_PARAMS: usize = 13;

/// Which field of the tail noad `scan_math` fills (§1151's `p`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Nucleus,
    Supscr,
    Subscr,
}

/// §1145's three dimensions of a display, which `\predisplaysize`,
/// `\displaywidth` and `\displayindent` hold while it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub pre_size: Scaled,
    pub width: Scaled,
    pub indent: Scaled,
}

/// `fam_in_range` (§1151) and the family a math code names.
fn math_char(c: i64, cur_fam: i64) -> MathChar {
    let fam = match c >= VAR_CODE && (0..16).contains(&cur_fam) {
        true => cur_fam as usize,
        false => ((c / 256) % 16) as usize,
    };
    MathChar {
        fam,
        character: (c % 256) as u8,
    }
}

/// A delimiter field from a 24-bit delimiter code (§1160).
fn delimiter(code: i64) -> Delimiter {
    Delimiter {
        small_fam: ((code / 0o4000000) % 16) as usize,
        small_char: ((code / 0o10000) % 256) as u8,
        large_fam: ((code / 256) % 16) as usize,
        large_char: (code % 256) as u8,
    }
}

/// The field `t` of noad `q`, if `q` has one (`scripts_allowed`, §687, for
/// the scripts).
fn field_mut(q: &mut Noad, t: Target) -> Option<&mut Field> {
    let atom = match q {
        Noad::Atom(a) | Noad::Over(a) | Noad::Under(a) | Noad::VCenter(a) => a,
        Noad::Radical(r) => &mut r.nucleus,
        Noad::Accent(acc) => &mut acc.atom,
        _ => return None,
    };
    Some(match t {
        Target::Nucleus => &mut atom.nucleus,
        Target::Supscr => &mut atom.supscr,
        Target::Subscr => &mut atom.subscr,
    })
}

impl Stomach {
    /// `push_math(c)` (§1136), with §1139's `new_save_level`.
    fn push_math(&mut self, mode: Mode, opened: Opened) {
        self.group_begin();
        let mut l = ListState::new(mode, opened);
        l.space_factor = self.cur().space_factor;
        l.prev_depth = self.cur().prev_depth;
        l.mode_line = self.line;
        self.nest.push(l);
    }

    /// `init_math` (§1138): a formula in text, or (§1145) a display, which
    /// first breaks the paragraph it interrupts. `f` is the current font,
    /// whose quad §1146 adds to the last line's width.
    pub fn init_math(&mut self, display: bool, f: usize, e: &dyn Eqtb) {
        if !display {
            self.push_math(
                Mode::Math,
                Opened::MathShift {
                    display: None,
                    eqno: None,
                },
            );
            return;
        }
        // §1145.
        let w = if self.cur().list.is_empty() {
            self.pop_list();
            -MAX_DIMEN
        } else {
            let widow = self.int_par("displaywidowpenalty");
            self.line_break(widow, e);
            self.pre_display_size(f)
        };
        let (width, indent) = self.display_line(e);
        let d = Display {
            pre_size: w,
            width,
            indent,
        };
        self.push_math(
            Mode::DisplayMath,
            Opened::MathShift {
                display: Some(d),
                eqno: None,
            },
        );
        // `eq_word_define` of the three dimensions: the program's slots, which
        // the group around the display restores.
        for (name, v) in [
            ("predisplaysize", w),
            ("displaywidth", width),
            ("displayindent", indent),
        ] {
            if let Some(slot) = crate::params::register_slot(name) {
                self.slot_writes.push((slot, v));
            }
        }
        if self.nest.len() == 2 {
            self.want_build_page("before_display");
        }
    }

    /// §1146-§1148: how far the last line's material reaches, plus two ems,
    /// or `max_dimen` when what is visible on it was stretched or shrunk.
    fn pre_display_size(&self, f: usize) -> Scaled {
        let Some(just) = &self.just_box else {
            return -MAX_DIMEN;
        };
        let quad = self.font(f).param(6);
        let mut v = just.shift_amount + 2 * quad;
        let mut w = -MAX_DIMEN;
        for p in &just.list {
            // §1147: `d` and whether `p` is visible.
            let (d, visible) = match p {
                Node::Char(c) | Node::Ligature(c, _) => (c.width, true),
                Node::Box(b) => (b.width, true),
                Node::Rule(r) => (r.width, true),
                Node::Kern { width, .. } | Node::Math(width) | Node::MathOff(width) => {
                    (*width, false)
                }
                // §1148, which reads no `glue_set`.
                Node::Glue(g) => {
                    let q = g.spec;
                    let stretched = match just.glue_sign {
                        crate::node::GlueSign::Stretching => {
                            just.glue_order == q.stretch_order && q.stretch != 0
                        }
                        crate::node::GlueSign::Shrinking => {
                            just.glue_order == q.shrink_order && q.shrink != 0
                        }
                        crate::node::GlueSign::Normal => false,
                    };
                    if stretched {
                        v = MAX_DIMEN;
                    }
                    (q.natural, g.kind != crate::node::LeaderKind::Normal)
                }
                _ => (0, false),
            };
            if !visible {
                if v < MAX_DIMEN {
                    v += d;
                }
                continue;
            }
            // `found:`.
            if v < MAX_DIMEN {
                v += d;
                w = v;
            } else {
                return MAX_DIMEN;
            }
        }
        w
    }

    /// §1149: the length and shift of line `prev_graf+2`, which a display is
    /// treated as, from `\parshape`, or `\hangindent` and `\hangafter`.
    fn display_line(&self, e: &dyn Eqtb) -> (Scaled, Scaled) {
        if let Some(&last) = self.par_shape.last() {
            let n = self.par_shape.len() as i64;
            let k = (self.cur().prev_graf + 2).min(n).max(1) as usize;
            let (indent, width) = self.par_shape.get(k - 1).copied().unwrap_or(last);
            return (width, indent);
        }
        let hsize = dimen_par(e, "hsize");
        let hang_indent = dimen_par(e, "hangindent");
        let hang_after = self.int_par("hangafter");
        let prev_graf = self.cur().prev_graf;
        let hanging = hang_indent != 0
            && ((hang_after >= 0 && prev_graf + 2 > hang_after) || prev_graf + 1 < -hang_after);
        match hanging {
            true => (
                hsize - hang_indent.abs(),
                if hang_indent > 0 { hang_indent } else { 0 },
            ),
            false => (hsize, 0),
        }
    }

    /// `start_eq_no` (§1142): an ordinary formula inside the display.
    pub fn start_eq_no(&mut self, leqno: bool) {
        self.push_math(
            Mode::Math,
            Opened::MathShift {
                display: None,
                eqno: Some(leqno),
            },
        );
    }

    fn cur_fam(&self) -> i64 {
        self.int_par("fam")
    }

    /// `set_math_char(c)` (§1155).
    pub fn set_math_char(&mut self, c: i64) {
        let class = match c >= VAR_CODE {
            true => Class::Ord,
            false => Class::from_code(c / 0o10000),
        };
        let nucleus = Field::Char(math_char(c, self.cur_fam()));
        self.cur_mut().mlist.push(Noad::Atom(Atom::new(class, nucleus)));
    }

    /// The end of `scan_math` (§1151): the chosen field gets a character.
    pub fn math_field_char(&mut self, c: i64) {
        let ch = math_char(c, self.cur_fam());
        let target = self.math_target.take().unwrap_or(Target::Nucleus);
        if let Some(q) = self.cur_mut().mlist.last_mut() {
            if let Some(field) = field_mut(q, target) {
                *field = Field::Char(ch);
            }
        }
    }

    /// §1150 and §1158: a new noad whose nucleus is scanned next.
    pub fn new_noad(&mut self, kind: u8) {
        let noad = match kind {
            8 => Noad::Under(Atom::default()),
            9 => Noad::Over(Atom::default()),
            k => Noad::Atom(Atom::new(Class::from_code(i64::from(k)), Field::Empty)),
        };
        self.cur_mut().mlist.push(noad);
        self.math_target = Some(Target::Nucleus);
    }

    /// `math_limit_switch` (§1159).
    pub fn math_limit_switch(&mut self, c: u8) {
        if let Some(Noad::Atom(a)) = self.cur_mut().mlist.last_mut() {
            if a.class() == Class::Op {
                a.limits = match c {
                    1 => Limits::Above,
                    2 => Limits::Beside,
                    _ => Limits::Normal,
                };
                return;
            }
        }
        self.say(Said::Error("Limit controls must follow a math operator".into()));
    }

    /// `math_radical` (§1163).
    pub fn math_radical(&mut self, code: i64) {
        let r = Radical {
            left_delimiter: delimiter(code % 0o100000000),
            nucleus: Atom::default(),
            index: None,
        };
        self.cur_mut().mlist.push(Noad::Radical(r));
        self.math_target = Some(Target::Nucleus);
    }

    /// `math_ac` (§1165).
    pub fn math_ac(&mut self, code: i64) {
        let acc = Accent {
            accent: math_char(code, self.cur_fam()),
            atom: Atom::default(),
        };
        self.cur_mut().mlist.push(Noad::Accent(acc));
        self.math_target = Some(Target::Nucleus);
    }

    /// `sub_sup` (§1176-§1177).
    pub fn sub_sup(&mut self, sup: bool) {
        let target = match sup {
            true => Target::Supscr,
            false => Target::Subscr,
        };
        let mut occupied = false;
        let fits = match self.cur_mut().mlist.last_mut() {
            Some(q) => match field_mut(q, target) {
                Some(f) => {
                    occupied = !f.is_empty();
                    !occupied
                }
                None => false,
            },
            None => false,
        };
        if !fits {
            self.cur_mut().mlist.push(Noad::Atom(Atom::default()));
            if occupied {
                let msg = match sup {
                    true => "Double superscript",
                    false => "Double subscript",
                };
                self.say(Said::Error(msg.into()));
            }
        }
        self.math_target = Some(target);
    }

    /// §1153: a subformula in braces, for the field chosen before.
    pub fn begin_math_group(&mut self) {
        let target = self.math_target.take().unwrap_or(Target::Nucleus);
        self.push_math(Mode::Math, Opened::MathGroup(target));
    }

    /// `fin_mlist(p)` (§1184-§1185): the finished mlist, with the nest
    /// popped.
    fn fin_mlist(&mut self, right: Option<Noad>) -> Vec<Noad> {
        let Some(mut l) = self.pop_list() else {
            return Vec::new();
        };
        match l.incompleat.take() {
            Some(mut f) => {
                f.denominator = l.mlist;
                match right {
                    None => vec![Noad::Fraction(f)],
                    Some(r) => {
                        // §1185: the `\left` stays outside the fraction.
                        let left = match f.numerator.is_empty() {
                            true => None,
                            false => Some(f.numerator.remove(0)),
                        };
                        let mut q: Vec<Noad> = left.into_iter().collect();
                        q.push(Noad::Fraction(f));
                        q.push(r);
                        q
                    }
                }
            }
            None => {
                let mut q = l.mlist;
                q.extend(right);
                q
            }
        }
    }

    /// §1186: the `}` of a subformula, after its group has ended.
    pub fn end_math_group(&mut self) {
        let Opened::MathGroup(target) = self.cur().opened.clone() else {
            return;
        };
        self.group_end();
        let mut p = self.fin_mlist(None);
        let tail = self.cur_mut().mlist.last_mut();
        let Some(q) = tail else {
            return;
        };
        // §1186's two simplifications: braces around a single Ord without
        // scripts are not there, and an accent alone in the nucleus of an
        // Ord replaces it.
        if p.len() == 1 {
            match &p[0] {
                Noad::Atom(a)
                    if a.class() == Class::Ord && a.subscr.is_empty() && a.supscr.is_empty() =>
                {
                    let nucleus = std::mem::take(&mut match p.remove(0) {
                        Noad::Atom(a) => a,
                        _ => unreachable!(),
                    }
                    .nucleus);
                    if let Some(field) = field_mut(q, target) {
                        *field = nucleus;
                    }
                    return;
                }
                Noad::Accent(_)
                    if target == Target::Nucleus
                        && matches!(q, Noad::Atom(a) if a.class() == Class::Ord) =>
                {
                    *q = p.remove(0);
                    return;
                }
                _ => {}
            }
        }
        if let Some(field) = field_mut(q, target) {
            *field = Field::List(p);
        }
    }

    /// `append_choices` (§1172).
    pub fn append_choices(&mut self) {
        self.cur_mut()
            .mlist
            .push(Noad::Choice(Box::new(Choice::default())));
        self.push_math(Mode::Math, Opened::MathChoice(0));
    }

    /// `build_choices` (§1174), for the list just ended.
    pub fn build_choices(&mut self) {
        let Opened::MathChoice(k) = self.cur().opened.clone() else {
            return;
        };
        self.group_end();
        let p = self.fin_mlist(None);
        if let Some(Noad::Choice(c)) = self.cur_mut().mlist.last_mut() {
            match k {
                0 => c.display = p,
                1 => c.text = p,
                2 => c.script = p,
                _ => c.script_script = p,
            }
        }
        if k < 3 {
            self.push_math(Mode::Math, Opened::MathChoice(k + 1));
        }
    }

    /// §1171's `\displaystyle` and its siblings.
    pub fn math_style(&mut self, s: u8) {
        self.cur_mut().mlist.push(Noad::Style(Style::from(s)));
    }

    /// §1171's `\nonscript`: a glue of nothing that §732 reads as an order
    /// to delete the glue after it in the script styles.
    pub fn non_script(&mut self) {
        let g = GlueNode::param(crate::glue::Glue::default(), GlueSource::CondMath);
        self.tail_append(Node::Glue(g));
    }

    /// `\mkern` (§1061, §717).
    pub fn append_mu_kern(&mut self, w: Scaled) {
        self.cur_mut().mlist.push(Noad::MuKern(w));
    }

    /// `math_fraction` (§1181-§1183).
    pub fn math_fraction(&mut self, code: u8, thickness: Scaled, left: i64, right: i64) {
        const DELIMITED: u8 = 3;
        if self.cur().incompleat.is_some() {
            self.say(Said::Error("Ambiguous; you need another { and }".into()));
            return;
        }
        let (left_delimiter, right_delimiter) = match code >= DELIMITED {
            true => (delimiter(left), delimiter(right)),
            false => (Delimiter::null(), Delimiter::null()),
        };
        let thickness = match code % DELIMITED {
            0 => Some(thickness),
            1 => None,
            _ => Some(0),
        };
        let cur = self.cur_mut();
        let numerator = std::mem::take(&mut cur.mlist);
        cur.incompleat = Some(Fraction {
            thickness,
            numerator,
            denominator: Vec::new(),
            left_delimiter,
            right_delimiter,
        });
    }

    /// `math_left_right` (§1191): the lowerer has matched `\right` to its
    /// `\left`.
    pub fn math_left_right(&mut self, right: bool, code: i64) {
        let d = delimiter(code);
        if !right {
            self.push_math(Mode::Math, Opened::MathLeft);
            self.cur_mut().mlist.push(Noad::Left(d));
            return;
        }
        if self.cur().opened != Opened::MathLeft {
            return;
        }
        let p = self.fin_mlist(Some(Noad::Right(d)));
        self.group_end();
        let inner = Atom::new(Class::Inner, Field::List(p));
        self.cur_mut().mlist.push(Noad::Atom(inner));
    }

    /// §1168: a `\vcenter` packaged, as a noad.
    pub fn vcenter_noad(&mut self, b: BoxNode) {
        let a = Atom::new(Class::Ord, Field::Box(Box::new(b)));
        self.cur_mut().mlist.push(Noad::VCenter(a));
    }

    /// §1076's math case: a box in a formula is the nucleus of an Ord.
    pub fn box_noad(&mut self, b: BoxNode) {
        let a = Atom::new(Class::Ord, Field::Box(Box::new(b)));
        self.cur_mut().mlist.push(Noad::Atom(a));
    }

    /// The families in `fams` (sixteen per size) as `mlist_to_hlist` reads
    /// them, with the parameters it reads from `eqtb`.
    fn math_fonts(&mut self, fams: &[u16], e: &dyn Eqtb) -> MathFonts {
        let mut fonts: Vec<[Option<MathFont>; 3]> = (0..16).map(|_| Default::default()).collect();
        for size in [TEXT_SIZE, SCRIPT_SIZE, SCRIPT_SCRIPT_SIZE] {
            for (fam, slot) in fonts.iter_mut().enumerate() {
                let f = fams.get(size * 16 + fam).copied().unwrap_or(0) as usize;
                slot[size] = self.math_font(f);
            }
        }
        let mu = |name: &str| {
            let g = glue_par(e, name);
            MuGlue {
                natural: g.natural,
                stretch: g.stretch,
                shrink: g.shrink,
                stretch_order: g.stretch_order,
                shrink_order: g.shrink_order,
            }
        };
        let params = MathParams {
            thin_mu_skip: mu("thinmuskip"),
            med_mu_skip: mu("medmuskip"),
            thick_mu_skip: mu("thickmuskip"),
            script_space: dimen_par(e, "scriptspace"),
            delimiter_factor: self.int_par("delimiterfactor"),
            delimiter_shortfall: dimen_par(e, "delimitershortfall"),
            null_delimiter_space: dimen_par(e, "nulldelimiterspace"),
            bin_op_penalty: self.int_par("binoppenalty"),
            rel_penalty: self.int_par("relpenalty"),
        };
        MathFonts::from_families(fonts, params)
    }

    /// Font `f` as a math font: its metrics, its parameters as `\fontdimen`
    /// left them, its `\skewchar`, and the number its characters carry.
    fn math_font(&mut self, f: usize) -> Option<MathFont> {
        let font = self.fonts.get(f)?;
        let tfm = font.tfm()?.clone();
        let name = font.name.clone();
        let exten = self
            .math_exten
            .entry(f)
            .or_insert_with(|| {
                crate::typeset::find_font(&name)
                    .map(|p| MathFont::exten_of(&p))
                    .unwrap_or_default()
            })
            .clone();
        let font = &self.fonts[f];
        let mut m = MathFont::new(tfm, font.size, exten);
        m.params = font.params.clone();
        m.skew_char = font.skew_char;
        m.node_font = Some(f);
        Some(m)
    }

    /// §1195: whether families 2 and 3 have the parameters a formula needs;
    /// if not, the formula is flushed (`flush_math`) and `danger` is set.
    fn math_fonts_insufficient(&mut self, fams: &[u16]) -> bool {
        let params = |fam: usize, size: usize| {
            let f = fams.get(size * 16 + fam).copied().unwrap_or(0) as usize;
            self.fonts.get(f).map_or(0, |font| font.params.len())
        };
        let sizes = [TEXT_SIZE, SCRIPT_SIZE, SCRIPT_SCRIPT_SIZE];
        let msg = if sizes.iter().any(|&s| params(2, s) < TOTAL_MATHSY_PARAMS) {
            "Math formula deleted: Insufficient symbol fonts"
        } else if sizes.iter().any(|&s| params(3, s) < TOTAL_MATHEX_PARAMS) {
            "Math formula deleted: Insufficient extension fonts"
        } else {
            return false;
        };
        self.say(Said::Error(msg.into()));
        // `flush_math` (§719).
        let cur = self.cur_mut();
        cur.mlist.clear();
        cur.incompleat = None;
        true
    }

    /// `mlist_to_hlist` on `p` in `style`.
    fn mlist_to_hlist(
        &mut self,
        p: &[Noad],
        style: Style,
        penalties: bool,
        fams: &[u16],
        e: &dyn Eqtb,
    ) -> Vec<Node> {
        let fonts = self.math_fonts(fams, e);
        Converter::new(&fonts)
            .with_penalties(penalties)
            .convert(p, style)
    }

    /// `after_math` (§1194), at the `$` that ends a formula. An equation
    /// number is finished here and the display waits for the next call,
    /// which the lowerer makes after the number's group has ended.
    pub fn after_math(&mut self, fams: &[u16], e: &dyn Eqtb) {
        let Opened::MathShift { eqno, display } = self.cur().opened.clone() else {
            return;
        };
        let danger = self.math_fonts_insufficient(fams);
        let m = self.mode();
        let eq_no = self.cur_mut().eq_no.take();
        let p = self.fin_mlist(None);
        if let Some(leqno) = eqno {
            // §1194's "end of equation number".
            let list = self.mlist_to_hlist(&p, TEXT_STYLE, false, fams, e);
            let a = hpack(list, NATURAL, self.tolerances(e), None).node;
            self.group_end();
            if !danger {
                self.cur_mut().eq_no = Some((a, leqno));
            }
            return;
        }
        match m {
            Mode::Math => {
                // §1196.
                let surround = dimen_par(e, "mathsurround");
                self.tail_append(Node::Math(surround));
                let penalties = self.mode().outer();
                let list = self.mlist_to_hlist(&p, TEXT_STYLE, penalties, fams, e);
                for n in list {
                    self.tail_append(n);
                }
                self.tail_append(Node::MathOff(surround));
                self.cur_mut().space_factor = 1000;
                self.group_end();
            }
            _ => {
                let d = display.unwrap_or(Display {
                    pre_size: -MAX_DIMEN,
                    width: 0,
                    indent: 0,
                });
                let eq = eq_no.filter(|_| !danger);
                self.finish_display(p, d, eq, fams, e);
            }
        }
    }

    /// "Finish displayed math" (§1199-§1205), with the display's level
    /// already popped by `fin_mlist`.
    fn finish_display(
        &mut self,
        p: Vec<Noad>,
        d: Display,
        eq: Option<(BoxNode, bool)>,
        fams: &[u16],
        e: &dyn Eqtb,
    ) {
        let (eq, l) = match eq {
            Some((a, l)) => (Some(a), l),
            None => (None, false),
        };
        let fonts = self.math_fonts(fams, e);
        let math_quad = fonts.math_quad(TEXT_SIZE);
        let list = Converter::new(&fonts).convert(&p, DISPLAY_STYLE);
        let tol = self.tolerances(e);
        let mut adjust: Vec<Node> = Vec::new();
        let packed = hpack(list.clone(), NATURAL, tol, Some(&mut adjust));
        let mut b = packed.node;
        let total_shrink = packed.total_shrink;
        let mut w = b.width;
        let z = d.width;
        let s = d.indent;
        let (mut e_w, q) = match &eq {
            Some(a) => (a.width, a.width + math_quad),
            None => (0, 0),
        };
        // §1201.
        if w + q > z {
            let shrinkable = w - total_shrink[0] + q <= z
                || total_shrink[1] != 0
                || total_shrink[2] != 0
                || total_shrink[3] != 0;
            if e_w != 0 && shrinkable {
                b = self.pack_display(b.list, z - q, tol);
            } else {
                e_w = 0;
                if w > z {
                    b = self.pack_display(b.list, z, tol);
                }
            }
            w = b.width;
        }
        // §1202.
        let mut dd = half(z - w);
        if e_w > 0 && dd < 2 * e_w {
            dd = half(z - w - e_w);
            if matches!(b.list.first(), Some(Node::Glue(_))) {
                dd = 0;
            }
        }
        // §1203.
        let pre = self.int_par("predisplaypenalty");
        self.tail_append(Node::Penalty(pre));
        let (g1, mut g2) = match dd + s <= d.pre_size || l {
            true => (GlueSource::AboveDisplaySkip, Some(GlueSource::BelowDisplaySkip)),
            false => (
                GlueSource::AboveDisplayShortSkip,
                Some(GlueSource::BelowDisplayShortSkip),
            ),
        };
        let param_glue = |source: GlueSource| {
            let name = match source {
                GlueSource::AboveDisplaySkip => "abovedisplayskip",
                GlueSource::BelowDisplaySkip => "belowdisplayskip",
                GlueSource::AboveDisplayShortSkip => "abovedisplayshortskip",
                _ => "belowdisplayshortskip",
            };
            Node::Glue(GlueNode::param(glue_par(e, name), source))
        };
        let mut eq = eq;
        if l && e_w == 0 {
            if let Some(mut a) = eq.take() {
                a.shift_amount = s;
                self.append_to_vlist(a);
                self.tail_append(Node::Penalty(INF_PENALTY));
            }
        } else {
            self.tail_append(param_glue(g1));
        }
        // §1204.
        if e_w != 0 {
            if let Some(a) = eq.take() {
                let r = Node::Kern {
                    width: z - w - e_w - dd,
                    explicit: false,
                };
                let list = match l {
                    true => {
                        dd = 0;
                        vec![Node::Box(a), r, Node::Box(b)]
                    }
                    false => vec![Node::Box(b), r, Node::Box(a)],
                };
                b = hpack(list, NATURAL, tol, None).node;
            }
        }
        b.shift_amount = s + dd;
        self.append_to_vlist(b);
        // §1205.
        if let Some(mut a) = eq.take() {
            if e_w == 0 && !l {
                self.tail_append(Node::Penalty(INF_PENALTY));
                a.shift_amount = s + z - a.width;
                self.append_to_vlist(a);
                g2 = None;
            }
        }
        for n in adjust {
            self.tail_append(n);
        }
        let post = self.int_par("postdisplaypenalty");
        self.tail_append(Node::Penalty(post));
        if let Some(g2) = g2 {
            self.tail_append(param_glue(g2));
        }
    }

    /// §1201's `hpack(p,w,exactly)`, reporting as `hpack` reports.
    fn pack_display(&mut self, list: Vec<Node>, w: Scaled, tol: crate::pack::Tolerances) -> BoxNode {
        let packed = hpack(list, Spec::Exactly(w), tol, None);
        self.last_badness = packed.badness;
        if let Some(r) = packed.report {
            self.box_report(&r, false, &packed.node);
        }
        packed.node
    }

    /// `resume_after_display` (§1200), after the display's group has ended.
    pub fn resume_after_display(&mut self) {
        self.group_end();
        self.cur_mut().prev_graf += 3;
        let mut l = ListState::new(Mode::Horizontal, Opened::Page);
        l.space_factor = 1000;
        l.mode_line = self.line;
        l.prev_graf = self.language_data();
        self.nest.push(l);
        if self.nest.len() == 2 {
            self.want_build_page("after_display");
        }
    }
}

/// `half(x)` (§100).
fn half(x: Scaled) -> Scaled {
    match x % 2 != 0 {
        true => (x + 1) / 2,
        false => x / 2,
    }
}

impl Stomach {
    /// Whether the current list is a formula's (`abs(mode)=mmode`).
    pub fn in_math(&self) -> bool {
        self.mode().kind() == Kind::M
    }
}
