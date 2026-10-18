//! The stomach at run time: the current list, the box registers, the fonts
//! and the page, acted on as `tex.web`'s `main_control` acts on them.
//!
//! texrs splits `main_control` (§1029-§1054) at the one seam a compiler can
//! split it. The DISPATCH -- which of §1045's mode-and-command cases a token
//! falls in -- is decided while lowering, because that is where expansion
//! happens: §1094 inserts the CURRENT meaning of `\par` in front of a `\vskip`
//! met in horizontal mode, and §1091 inserts `\everypar` when a paragraph
//! starts, and both are token insertions only the mouth can make. What each
//! case DOES to a list is decided here, when the program runs, because that is
//! where the registers are: a box's width depends on a `\hsize` a run-time
//! `\advance` may have changed, and `\ifdim\wd0>\hsize` is a branch on both.
//!
//! So the lowered program carries [`crate::ir::ListOp`]s, the VM hands each
//! to [`Stomach::apply`], and this module is the port of the action routines:
//!
//! - the semantic nest (§211-§219): one [`ListState`] per open list, the
//!   outer vertical list at the bottom, each with the `aux` field its mode
//!   reads (`\prevdepth` in vertical mode, `\spacefactor` in horizontal);
//! - the box registers (§230's `box_base`), with §268-§283's save stack, so a
//!   `\setbox` inside a group is undone at its end and a `\global` one is not;
//! - the fonts (§539-§582), each with its metrics at its own size;
//! - the action routines of §1055-§1135 (in [`build`] and [`font`]), the
//!   paragraph builder's entry and exit (§1091-§1096, in [`par`]), and the
//!   page builder and `\shipout` (§980-§1028 and §638-§640, in [`output`]).
//!
//! A register a document can READ -- `\wd0`, `\lastpenalty`, `\prevdepth`,
//! `\pagegoal` -- is mirrored into a VM slot after every action (see
//! [`Mirror`]), so `\ifdim\wd0>\hsize` and `\the\lastpenalty` compile to the
//! slot reads every other register is.

use crate::glue::Glue;
use crate::node::{BoxNode, Node, Scaled, IGNORE_DEPTH};

pub mod build;
pub mod font;
pub mod hyph;
pub mod math;
pub mod output;
pub mod par;
pub mod run;

pub use font::Font;

/// `mode` (§211), with its sign: negative is the restricted or internal
/// variant of the same list kind. `abs(mode)` is [`Mode::kind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `vmode`: the page's own list.
    Vertical,
    /// `-vmode`: inside a `\vbox`, `\vtop`, `\vadjust`, `\insert` or the
    /// output routine.
    InternalVertical,
    /// `hmode`: a paragraph.
    Horizontal,
    /// `-hmode`: inside an `\hbox`.
    RestrictedHorizontal,
    /// `-mmode`: a formula in text.
    Math,
    /// `mmode`: a displayed formula.
    DisplayMath,
}

/// `abs(mode)`: which kind of list a mode builds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    V,
    H,
    M,
}

impl Mode {
    pub fn kind(self) -> Kind {
        match self {
            Mode::Vertical | Mode::InternalVertical => Kind::V,
            Mode::Horizontal | Mode::RestrictedHorizontal => Kind::H,
            Mode::Math | Mode::DisplayMath => Kind::M,
        }
    }

    /// `mode>0`: the unrestricted variant.
    pub fn outer(self) -> bool {
        matches!(self, Mode::Vertical | Mode::Horizontal | Mode::DisplayMath)
    }
}

/// One level of the semantic nest (§212's `list_state_record`).
#[derive(Clone, Debug)]
pub struct ListState {
    pub mode: Mode,
    /// `link(head)` .. `tail`.
    pub list: Vec<Node>,
    /// `prev_depth` (vertical modes) -- §213's `aux.sc`.
    pub prev_depth: Scaled,
    /// `space_factor` (horizontal modes) -- §213's `aux.hh.lh`.
    pub space_factor: i64,
    /// `prev_graf` (§213): lines in the most recent paragraph.
    pub prev_graf: i64,
    /// `mode_line` (§213): where the list began, negative when it began in
    /// the output routine or in an `\insert`/`\vadjust` (§1025, §1099).
    pub mode_line: i64,
    /// What closes this list: the box under construction and how it was
    /// asked for, or the insertion or paragraph it is.
    pub opened: Opened,
    /// The mlist of a math mode (§1136): `link(head)`..`tail` when the list
    /// is a formula's, where [`ListState::list`] stays empty.
    pub mlist: Vec<crate::math::noad::Noad>,
    /// `incompleat_noad` (§1178): a fraction waiting for its denominator.
    pub incompleat: Option<crate::math::noad::Fraction>,
    /// A display's equation number, boxed by §1194 when its `\eqno` part
    /// ended, and whether it was `\leqno`.
    pub eq_no: Option<(BoxNode, bool)>,
}

impl ListState {
    pub fn new(mode: Mode, opened: Opened) -> ListState {
        ListState {
            mode,
            list: Vec::new(),
            prev_depth: IGNORE_DEPTH,
            space_factor: 1000,
            prev_graf: 0,
            mode_line: 0,
            opened,
            mlist: Vec::new(),
            incompleat: None,
            eq_no: None,
        }
    }
}

/// What a list was opened for (§1083's `saved(0)` and the group code it
/// carries, §269).
#[derive(Clone, Debug, PartialEq)]
pub enum Opened {
    /// The outer vertical list, or a paragraph on it.
    Page,
    /// A box construction: its kind, the `to`/`spread` it asked for, where it
    /// goes when it closes, and the `\boxmaxdepth` the group ended with
    /// (§1086 reads it before `unsave`).
    Box {
        kind: BoxKind,
        spec: crate::pack::Spec,
        context: BoxContext,
        max_depth: Scaled,
    },
    /// `\vadjust` or `\insert n` (§1099).
    Insert(u16),
    /// The output routine (§1025).
    Output,
    /// A formula, `$` to `$` (§1139) or `$$` to `$$` (§1145): a display
    /// carries §1145's dimensions, and the formula an `\eqno` or `\leqno`
    /// opened (§1142) is `eqno: Some(leqno)`.
    MathShift {
        display: Option<math::Display>,
        eqno: Option<bool>,
    },
    /// A subformula in braces (§1153), and the field of the enclosing list's
    /// tail it fills.
    MathGroup(math::Target),
    /// `\left` .. `\right` (§1191).
    MathLeft,
    /// One of `\mathchoice`'s four lists (§1172).
    MathChoice(u8),
}

/// `\hbox`, `\vbox` or `\vtop` (§1083's `k`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoxKind {
    H,
    V,
    Top,
    /// `\vcenter` (§1167), which becomes a noad rather than a box.
    VCenter,
}

/// §1071's box context: where a finished box goes.
#[derive(Clone, Debug, PartialEq)]
pub enum BoxContext {
    /// `box_context<box_flag`: onto the current list, shifted by this much
    /// (`\raise`, `\lower`, `\moveleft`, `\moveright`, or zero).
    Shift(Scaled),
    /// `box_flag+n` and `box_flag+256+n`: `\setbox n`, `\global` or not.
    SetBox { n: u16, global: bool },
    /// `ship_out_flag`.
    ShipOut,
    /// `leader_flag+k`: `\leaders`, `\cleaders`, `\xleaders`; the glue that
    /// must follow is the lowerer's to read, so the box waits in
    /// [`Stomach::leader_box`].
    Leaders(crate::node::LeaderKind),
}

/// The run-time values the stomach reads that live in VM slots: the
/// dimension and glue parameters and every register (`crate::compiler`'s
/// slot file).
pub trait Eqtb {
    /// The value in slot `slot`.
    fn slot(&self, slot: i64) -> i64;
}

/// A dimension parameter (§247), read from its slot.
pub fn dimen_par(e: &dyn Eqtb, name: &str) -> Scaled {
    crate::params::register_slot(name).map_or(0, |s| e.slot(s))
}

/// A glue parameter (§224), read from its four slots: natural, stretch,
/// shrink, and the two orders packed as `stretch*4+shrink`.
pub fn glue_par(e: &dyn Eqtb, name: &str) -> Glue {
    crate::params::register_slot(name).map_or(Glue::default(), |s| glue_at(e, s))
}

/// The glue whose first slot is `s`.
pub fn glue_at(e: &dyn Eqtb, s: i64) -> Glue {
    let orders = e.slot(s + 3);
    Glue {
        natural: e.slot(s),
        stretch: e.slot(s + 1),
        stretch_order: orders / 4,
        shrink: e.slot(s + 2),
        shrink_order: orders % 4,
    }
}

/// One box register (§230's `box(n)`) and the level it was assigned at
/// (§221's `eq_level`).
#[derive(Clone, Debug, Default)]
struct Register {
    value: Option<BoxNode>,
    level: usize,
}

/// A save-stack entry (§268's `restore_old_value`): a box register, or
/// `\parshape` with the level it was assigned at.
#[derive(Clone, Debug)]
enum Saved {
    Box { n: u16, old: Register },
    ParShape { old: Vec<(Scaled, Scaled)>, level: usize },
}

/// The stomach's state for one run.
pub struct Stomach {
    /// The semantic nest; `nest[0]` is the outer vertical list, which is
    /// §995's contribution list.
    pub nest: Vec<ListState>,
    registers: Vec<Register>,
    /// One frame per open group (§269's `cur_level-level_one`).
    saves: Vec<Vec<Saved>>,
    /// §539's font memory. Font 0 is `\nullfont`.
    pub fonts: Vec<Font>,
    /// §236's integer parameters as the lowerer last reported them.
    pub int_pars: Vec<i64>,
    /// §230's `sf_code`, for the characters whose code is not INITEX's.
    pub sf_codes: rustc_hash::FxHashMap<char, i64>,
    /// §230's `lc_code`, for the characters whose code is not INITEX's.
    pub lc_codes: rustc_hash::FxHashMap<char, i64>,
    /// `last_badness` (§646): what `\badness` reads.
    pub last_badness: i64,
    /// A leader box waiting for its glue (§1078).
    pub leader_box: Option<Node>,
    /// The page builder, the shipped pages, and the output routine's state.
    pub output: output::Output,
    /// The page builder (§980-§1028).
    pub pages: output::Pages,
    /// The `\patterns` and `\hyphenation` the document gave (§934, §960), as
    /// the source `crate::linebreak::Hyphenator` reads, and the hyphenator
    /// built from them when the hyphenation pass first needs it.
    pub hyphenation_source: String,
    pub hyphenator: Option<std::rc::Rc<crate::linebreak::Hyphenator>>,
    /// Registers whose slots are stale, for [`Mirror`].
    dirty: Vec<u16>,
    /// What the stomach has to say, in order, for the run to print: see
    /// [`Said`].
    pub said: Vec<Said>,
    /// Set by an action after which §994's `build_page` runs.
    build_page_wanted: bool,
    /// Why, for `buildpage_filter`.
    pub build_page_reason: &'static str,
    /// The source line of the command being carried out, for §663's
    /// `detected at line`.
    pub line: i64,
    /// Parameters an action reads, refreshed from the slots before each one
    /// (see [`Stomach::refresh`]).
    pub baselines: crate::pack::Baselines,
    pub split_top_skip: Glue,
    pub split_max_depth: Scaled,
    /// §661's `pack_begin_line`: the line a paragraph began on while its
    /// lines are packaged, negative for an alignment, zero otherwise.
    pub pack_begin_line: i64,
    /// `\count0..9`, which `ship_out` writes into each page (§640).
    pub counts: [i64; 10],
    /// `\hoffset` and `\voffset`, which §640 adds to every page.
    pub h_offset: Scaled,
    pub v_offset: Scaled,
    /// `par_shape_ptr` (§230): `\parshape`'s (indent, width) pairs, and the
    /// level they were assigned at.
    pub par_shape: Vec<(Scaled, Scaled)>,
    par_shape_level: usize,
    /// The field of the tail noad `scan_math` fills next (§1151).
    pub math_target: Option<math::Target>,
    /// `just_box` (§814): the last line `post_line_break` made, which §1146
    /// measures when a display interrupts its paragraph.
    pub just_box: Option<BoxNode>,
    /// Slots an action assigned that are not mirrors of anything else:
    /// §1145's `\predisplaysize`, `\displaywidth`, `\displayindent`.
    pub slot_writes: Vec<(i64, i64)>,
    /// The number the lowerer gave each mark's text (`ListOp::Mark`).
    pub mark_ids: rustc_hash::FxHashMap<String, u32>,
    /// The `exten` table of each font a formula has been set from (§544).
    pub math_exten: rustc_hash::FxHashMap<usize, Vec<[u8; 4]>>,
}

impl Default for Stomach {
    fn default() -> Self {
        Self::new()
    }
}

impl Stomach {
    /// §215's initial state: the outer vertical list, empty; `\nullfont`
    /// alone in font memory (§552); every register void; INITEX's integer
    /// parameters.
    pub fn new() -> Stomach {
        Stomach {
            nest: vec![ListState::new(Mode::Vertical, Opened::Page)],
            registers: vec![Register::default(); 256],
            saves: Vec::new(),
            fonts: vec![Font::null()],
            int_pars: initex_int_pars(),
            sf_codes: rustc_hash::FxHashMap::default(),
            lc_codes: rustc_hash::FxHashMap::default(),
            last_badness: 0,
            leader_box: None,
            output: output::Output::default(),
            pages: output::Pages::default(),
            hyphenation_source: String::new(),
            hyphenator: None,
            dirty: Vec::new(),
            said: Vec::new(),
            build_page_wanted: false,
            build_page_reason: "",
            line: 0,
            baselines: crate::pack::Baselines::default(),
            split_top_skip: Glue::default(),
            split_max_depth: 0,
            counts: [0; 10],
            pack_begin_line: 0,
            h_offset: 0,
            v_offset: 0,
            par_shape: Vec::new(),
            par_shape_level: 1,
            math_target: None,
            just_box: None,
            slot_writes: Vec::new(),
            math_exten: rustc_hash::FxHashMap::default(),
            mark_ids: rustc_hash::FxHashMap::default(),
        }
    }

    /// The innermost list (§213's `cur_list`).
    pub fn cur(&self) -> &ListState {
        self.nest.last().expect("the outer list is never popped")
    }

    pub fn cur_mut(&mut self) -> &mut ListState {
        self.nest.last_mut().expect("the outer list is never popped")
    }

    pub fn mode(&self) -> Mode {
        self.cur().mode
    }

    /// `tail_append` (§214). In math mode the node goes into the mlist,
    /// among the noads (§730).
    pub fn tail_append(&mut self, node: Node) {
        let cur = self.cur_mut();
        match cur.mode.kind() {
            Kind::M => cur.mlist.push(crate::math::noad::Noad::Node(node)),
            _ => cur.list.push(node),
        }
    }

    /// An integer parameter by its §236 name.
    pub fn int_par(&self, name: &str) -> i64 {
        crate::intpar::index(name)
            .and_then(|i| self.int_pars.get(i).copied())
            .unwrap_or(0)
    }

    /// `sf_code(c)` (§232): 999 for an uppercase letter, 1000 otherwise,
    /// unless the document said differently.
    pub fn sf_code(&self, c: char) -> i64 {
        match self.sf_codes.get(&c) {
            Some(v) => *v,
            None if c.is_ascii_uppercase() => 999,
            None => 1000,
        }
    }

    /// `box(n)`, without emptying it.
    pub fn box_register(&self, n: u16) -> Option<&BoxNode> {
        self.registers.get(n as usize).and_then(|r| r.value.as_ref())
    }

    /// `box(n):=null` -- `\box n` takes the box and leaves the register void
    /// "at the same level" (§1079): no save-stack entry, no level change.
    pub fn take_box(&mut self, n: u16) -> Option<BoxNode> {
        let taken = self.registers.get_mut(n as usize)?.value.take();
        self.dirty.push(n);
        taken
    }

    /// The register's box, to change in place (`\wd n=` and its siblings,
    /// §1247: a box dimension is not an `eqtb` entry, so no save is made).
    pub fn box_register_mut(&mut self, n: u16) -> Option<&mut BoxNode> {
        self.dirty.push(n);
        self.registers.get_mut(n as usize)?.value.as_mut()
    }

    /// `eq_define`/`geq_define` of a box register (§277, §279): `\setbox n`.
    pub fn set_box(&mut self, n: u16, value: Option<BoxNode>, global: bool) {
        if n as usize >= self.registers.len() {
            self.registers.resize(n as usize + 1, Register::default());
        }
        self.dirty.push(n);
        let level = self.saves.len() + 1;
        let r = &mut self.registers[n as usize];
        if global {
            // §279: `geq_define` -- the value, at level one, with no save.
            *r = Register { value, level: 1 };
            return;
        }
        // §277: `eq_define` saves the old value unless it was assigned at
        // this level already.
        if r.level != level {
            if let Some(frame) = self.saves.last_mut() {
                frame.push(Saved::Box { n, old: r.clone() });
            }
        }
        *r = Register { value, level };
    }

    /// `new_save_level` (§274), for the box registers.
    pub fn group_begin(&mut self) {
        self.saves.push(Vec::new());
    }

    /// `unsave` (§281-§283), for the box registers: an entry assigned at
    /// level one since the save -- a `\global\setbox` -- keeps its value and
    /// the save is discarded; anything else gets its old value back.
    pub fn group_end(&mut self) {
        let Some(frame) = self.saves.pop() else {
            return;
        };
        for s in frame.into_iter().rev() {
            match s {
                Saved::Box { n, old } => {
                    let r = &mut self.registers[n as usize];
                    if r.level != 1 {
                        *r = old;
                    }
                    self.dirty.push(n);
                }
                Saved::ParShape { old, level } => {
                    if self.par_shape_level != 1 {
                        self.par_shape = old;
                        self.par_shape_level = level;
                    }
                }
            }
        }
    }

    /// `\parshape` (§1248): `eq_define` or `geq_define` of `par_shape_loc`.
    pub fn set_par_shape(&mut self, shape: Vec<(Scaled, Scaled)>, global: bool) {
        let level = self.saves.len() + 1;
        if global {
            self.par_shape = shape;
            self.par_shape_level = 1;
            return;
        }
        if self.par_shape_level != level {
            if let Some(frame) = self.saves.last_mut() {
                frame.push(Saved::ParShape {
                    old: std::mem::take(&mut self.par_shape),
                    level: self.par_shape_level,
                });
            }
        }
        self.par_shape = shape;
        self.par_shape_level = level;
    }

    /// The registers changed since the last [`Mirror`] pass.
    fn take_dirty(&mut self) -> Vec<u16> {
        let mut d = std::mem::take(&mut self.dirty);
        d.sort_unstable();
        d.dedup();
        d
    }

    /// The font `f`, or `\nullfont` for a number font memory does not hold.
    pub fn font(&self, f: usize) -> &Font {
        self.fonts.get(f).unwrap_or(&self.fonts[0])
    }
}

/// Something the stomach reports, for the run to print where it stands in
/// the terminal stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Said {
    /// §82's `print_err` and `error`: `! TEXT.` and the context of the
    /// command being carried out.
    Error(String),
    /// §245's `begin_diagnostic`..`end_diagnostic`: on the terminal when
    /// `\tracingonline` is positive, in the log alone otherwise -- which the
    /// terminal learns of only as `history=warning_issued` (§1335's closing
    /// "see the transcript file" line).
    Diagnostic(String),
    /// Printed on the terminal on a line of its own, hard against what came
    /// before (§62's `print_nl`): an `Overfull \hbox` warning (§663).
    Report(String),
    /// Printed on the terminal as a message of its own: the `[1]` §638
    /// writes as each page ships.
    Terminal(String),
}

impl Stomach {
    /// Report something; see [`Said`].
    pub fn say(&mut self, what: Said) {
        self.said.push(what);
    }

    /// A diagnostic, routed by `\tracingonline` as §245 routes it.
    pub fn diagnostic(&mut self, text: &str) {
        self.say(Said::Diagnostic(text.to_string()));
    }

    /// `output_active` (§989): whether the output routine is running.
    pub fn output_active(&self) -> bool {
        self.nest.iter().any(|l| l.opened == Opened::Output)
    }

    /// §63's escape character as it stands, for a display.
    pub fn esc_text(&self) -> String {
        let e = self.int_par("escapechar");
        match (0..256).contains(&e) {
            true => char::from_u32(e as u32).map(String::from).unwrap_or_default(),
            false => String::new(),
        }
    }

    /// Ask for §994's `build_page` once the action is finished.
    /// `reason` is what LuaTeX's `buildpage_filter` is told: the
    /// contribution that called `build_page` (`"new_graf"`, `"box"`, ...).
    pub fn want_build_page(&mut self, reason: &'static str) {
        self.build_page_wanted = true;
        self.build_page_reason = reason;
    }

    /// Whether an action asked for `build_page`, clearing the request.
    pub fn take_build_page(&mut self) -> bool {
        std::mem::take(&mut self.build_page_wanted)
    }

    /// Read the parameters an action may need from the slot file, as they
    /// stand when the action runs.
    pub fn refresh(&mut self, e: &dyn Eqtb) {
        self.baselines = crate::pack::Baselines {
            baseline_skip: glue_par(e, "baselineskip"),
            line_skip: glue_par(e, "lineskip"),
            line_skip_limit: dimen_par(e, "lineskiplimit"),
        };
        self.split_top_skip = glue_par(e, "splittopskip");
        self.split_max_depth = dimen_par(e, "splitmaxdepth");
        self.h_offset = dimen_par(e, "hoffset");
        self.v_offset = dimen_par(e, "voffset");
        for (i, c) in self.counts.iter_mut().enumerate() {
            *c = e.slot(i as i64);
        }
    }
}

/// INITEX's integer parameters (§240): all zero except `\mag`,
/// `\tolerance`, `\hangafter`, `\maxdeadcycles`, `\escapechar` and
/// `\endlinechar`.
pub fn initex_int_pars() -> Vec<i64> {
    let mut v = vec![0; crate::intpar::NAMES.len()];
    for (name, value) in [
        ("mag", 1000),
        ("tolerance", 10000),
        ("hangafter", 1),
        ("maxdeadcycles", 25),
        ("escapechar", 92),
        ("endlinechar", 13),
    ] {
        if let Some(i) = crate::intpar::index(name) {
            v[i] = value;
        }
    }
    v
}

/// Where the registers a document can read live in the VM's slot file: the
/// three dimensions and the kind of every box register, and the internal
/// quantities of §413 the stomach computes.
///
/// The slots are written by [`Mirror::flush`] after every list action and
/// read by the program like any register, so the lowerer never needs to know
/// whether `\wd0` changed -- it only reads the slot.
pub struct Mirror;

/// What a box register holds, as the slot at [`box_kind_slot`] records it:
/// the answer to `\ifvoid`, `\ifhbox` and `\ifvbox` (§505).
pub const KIND_VOID: i64 = 0;
pub const KIND_HBOX: i64 = 1;
pub const KIND_VBOX: i64 = 2;

/// Which of a box's three dimensions (§420's `width_offset`, `depth_offset`,
/// `height_offset`, in that order as `\wd`, `\dp`, `\ht`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoxDimen {
    Wd,
    Ht,
    Dp,
}

/// The slot `\wd n`, `\ht n` or `\dp n` is mirrored in.
pub fn box_dimen_slot(n: u16, which: BoxDimen) -> i64 {
    let k = match which {
        BoxDimen::Wd => 0,
        BoxDimen::Ht => 1,
        BoxDimen::Dp => 2,
    };
    crate::compiler::BOXDIM_BASE + 3 * i64::from(n) + k
}

/// The slot the kind of box register `n` is mirrored in.
pub fn box_kind_slot(n: u16) -> i64 {
    crate::compiler::BOXKIND_BASE + i64::from(n)
}

/// §413's internal quantities that live in the stomach, each with a slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Internal {
    LastPenalty,
    LastKern,
    /// `\lastskip`: four slots, like a glue register.
    LastSkip,
    Badness,
    PrevDepth,
    SpaceFactor,
    PrevGraf,
    PageGoal,
    PageTotal,
    /// `\pagestretch` and its three infinite orders.
    PageStretch(u8),
    PageShrink,
    PageDepth,
    DeadCycles,
    InsertPenalties,
    /// `\outputpenalty`: an integer parameter, but one `fire_up` (§1013)
    /// assigns while the program runs, so it is read from here.
    OutputPenalty,
    /// Which mark `\topmark`, `\firstmark`, `\botmark`, `\splitfirstmark`
    /// and `\splitbotmark` (0..4) hold (§382): the number the lowerer gave
    /// the mark's text, 0 for none.
    Mark(u8),
}

impl Internal {
    /// Its slot (the first of four for `\lastskip`).
    pub fn slot(self) -> i64 {
        let k = match self {
            Internal::LastPenalty => 0,
            Internal::LastKern => 1,
            Internal::LastSkip => 2,
            Internal::Badness => 6,
            Internal::PrevDepth => 7,
            Internal::SpaceFactor => 8,
            Internal::PrevGraf => 9,
            Internal::PageGoal => 10,
            Internal::PageTotal => 11,
            Internal::PageStretch(o) => 12 + i64::from(o.min(3)),
            Internal::PageShrink => 16,
            Internal::PageDepth => 17,
            Internal::DeadCycles => 18,
            Internal::InsertPenalties => 19,
            Internal::OutputPenalty => 20,
            Internal::Mark(k) => 21 + i64::from(k.min(4)),
        };
        crate::compiler::INTERNAL_BASE + k
    }

    /// The quantity a primitive names, if it is one of these.
    pub fn of_name(name: &str) -> Option<Internal> {
        Some(match name {
            "lastpenalty" => Internal::LastPenalty,
            "lastkern" => Internal::LastKern,
            "lastskip" => Internal::LastSkip,
            "badness" => Internal::Badness,
            "prevdepth" => Internal::PrevDepth,
            "spacefactor" => Internal::SpaceFactor,
            "prevgraf" => Internal::PrevGraf,
            "pagegoal" => Internal::PageGoal,
            "pagetotal" => Internal::PageTotal,
            "pagestretch" => Internal::PageStretch(0),
            "pagefilstretch" => Internal::PageStretch(1),
            "pagefillstretch" => Internal::PageStretch(2),
            "pagefilllstretch" => Internal::PageStretch(3),
            "pageshrink" => Internal::PageShrink,
            "pagedepth" => Internal::PageDepth,
            "deadcycles" => Internal::DeadCycles,
            "insertpenalties" => Internal::InsertPenalties,
            "outputpenalty" => Internal::OutputPenalty,
            _ => return None,
        })
    }

    /// Whether `\the` writes it as a dimension.
    pub fn is_dimen(self) -> bool {
        matches!(
            self,
            Internal::LastKern
                | Internal::PrevDepth
                | Internal::PageGoal
                | Internal::PageTotal
                | Internal::PageStretch(_)
                | Internal::PageShrink
                | Internal::PageDepth
        )
    }
}

/// How many slots [`Internal`] occupies.
pub const INTERNAL_SLOTS: i64 = 26;

/// Somewhere the mirrored values can be written: the VM's slot file.
pub trait SlotSink {
    fn set(&mut self, slot: i64, value: i64);
}

impl Mirror {
    /// Write every value a document can read and the last action may have
    /// changed.
    pub fn flush(s: &mut Stomach, out: &mut dyn SlotSink) {
        for n in s.take_dirty() {
            let (kind, w, h, d) = match s.box_register(n) {
                None => (KIND_VOID, 0, 0, 0),
                Some(b) => (
                    if b.vertical { KIND_VBOX } else { KIND_HBOX },
                    b.width,
                    b.height,
                    b.depth,
                ),
            };
            if n < 256 {
                out.set(box_kind_slot(n), kind);
                out.set(box_dimen_slot(n, BoxDimen::Wd), w);
                out.set(box_dimen_slot(n, BoxDimen::Ht), h);
                out.set(box_dimen_slot(n, BoxDimen::Dp), d);
            }
        }
        // §424: `\lastpenalty`, `\lastkern` and `\lastskip` look at the tail
        // of the current list -- zero if it is not that kind of node, and
        // zero in the outer vertical list where the page builder has taken
        // the nodes (§424 reads `last_penalty` etc. there instead).
        let (pen, kern, skip) = s.last_item();
        out.set(Internal::LastPenalty.slot(), pen);
        out.set(Internal::LastKern.slot(), kern);
        let base = Internal::LastSkip.slot();
        out.set(base, skip.natural);
        out.set(base + 1, skip.stretch);
        out.set(base + 2, skip.shrink);
        out.set(base + 3, skip.stretch_order * 4 + skip.shrink_order);
        out.set(Internal::Badness.slot(), s.last_badness);
        let cur = s.cur();
        out.set(Internal::PrevDepth.slot(), cur.prev_depth);
        out.set(Internal::SpaceFactor.slot(), cur.space_factor);
        out.set(Internal::PrevGraf.slot(), s.prev_graf());
        s.output.mirror(out);
        for (slot, value) in std::mem::take(&mut s.slot_writes) {
            out.set(slot, value);
        }
    }
}

impl Stomach {
    /// §424's `last_item` for `\lastpenalty`, `\lastkern` and `\lastskip`.
    fn last_item(&self) -> (i64, Scaled, Glue) {
        let cur = self.cur();
        if self.nest.len() == 1 && cur.list.is_empty() {
            return self.output.last_item();
        }
        match cur.list.last() {
            Some(Node::Penalty(p)) => (*p, 0, Glue::default()),
            Some(Node::Kern { width, .. }) => (0, *width, Glue::default()),
            Some(Node::Glue(g)) => (0, 0, g.spec),
            _ => (0, 0, Glue::default()),
        }
    }

    /// `\prevgraf` (§422): the nearest enclosing list that is not math --
    /// `nest[p]` with `abs(nest[p].mode_field)=vmode` searched outwards.
    pub fn prev_graf(&self) -> i64 {
        self.nest
            .iter()
            .rev()
            .find(|l| l.mode.kind() == Kind::V)
            .map_or(0, |l| l.prev_graf)
    }

    /// `\prevgraf=` (§1244): written to that same list.
    pub fn set_prev_graf(&mut self, value: i64) {
        if let Some(l) = self.nest.iter_mut().rev().find(|l| l.mode.kind() == Kind::V) {
            l.prev_graf = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hbox(width: Scaled) -> BoxNode {
        BoxNode {
            width,
            ..BoxNode::null()
        }
    }

    /// §283: a `\setbox` inside a group is undone at the `}`; a `\global`
    /// one survives it, even when a local one came first.
    #[test]
    fn box_registers_obey_the_save_stack() {
        let mut s = Stomach::new();
        s.set_box(0, Some(hbox(1)), false);
        s.group_begin();
        s.set_box(0, Some(hbox(2)), false);
        s.set_box(1, Some(hbox(3)), false);
        s.set_box(1, Some(hbox(4)), true);
        s.group_end();
        assert_eq!(s.box_register(0).map(|b| b.width), Some(1));
        assert_eq!(s.box_register(1).map(|b| b.width), Some(4));
    }

    /// §1079: `\box n` voids the register without a save, so the group end
    /// does not bring the box back.
    #[test]
    fn taking_a_box_is_not_undone_by_a_group() {
        let mut s = Stomach::new();
        s.set_box(5, Some(hbox(7)), false);
        s.group_begin();
        assert!(s.take_box(5).is_some());
        s.group_end();
        assert!(s.box_register(5).is_none());
    }

    struct Slots(rustc_hash::FxHashMap<i64, i64>);
    impl SlotSink for Slots {
        fn set(&mut self, slot: i64, value: i64) {
            self.0.insert(slot, value);
        }
    }

    /// A register's kind and dimensions reach its slots, and a register
    /// emptied reads as void and zero.
    #[test]
    fn the_mirror_writes_what_ifvoid_and_wd_read() {
        let mut s = Stomach::new();
        let mut slots = Slots(Default::default());
        s.set_box(3, Some(hbox(65536)), false);
        Mirror::flush(&mut s, &mut slots);
        assert_eq!(slots.0[&box_kind_slot(3)], KIND_HBOX);
        assert_eq!(slots.0[&box_dimen_slot(3, BoxDimen::Wd)], 65536);
        s.take_box(3);
        Mirror::flush(&mut s, &mut slots);
        assert_eq!(slots.0[&box_kind_slot(3)], KIND_VOID);
        assert_eq!(slots.0[&box_dimen_slot(3, BoxDimen::Wd)], 0);
    }
}
