//! Alignment: §768-§812's `\halign` and `\valign`, as the program carries
//! them out.
//!
//! The lowerer reads the preamble and feeds each entry's templates into the
//! mouth (§777-§790), so what reaches the stomach is the shape of the
//! alignment: the tabskip glue and an alignrecord per column, then rows of
//! entries, each a list built in its own mode and ended by `fin_col`. Here
//! each entry becomes an unset box whose natural size is recorded against
//! its column (and against the columns it spans, §798), each row an unset
//! box, and `fin_align` (§800-§812) works out the column widths and sets
//! every unset box to them.
//!
//! An unset box (§159) is a [`BoxNode`] with subtype [`UNSET`] or
//! [`UNSET_ROW`]; what §159 overlays on a box's fields -- the span count and
//! the total stretch and shrink with their orders -- is kept in
//! [`Align::unsets`], indexed by the box's `glue_set` (where `tex.web` keeps
//! `glue_stretch`); its `shift_amount` stays zero, as `hpack` reads an unset
//! node's (§653). No unset box leaves the alignment.

use super::{ListState, Mode, Opened, Stomach};
use crate::glue::Glue;
use crate::node::{BoxNode, GlueNode, GlueSign, GlueSource, Node, RuleNode, Scaled};
use crate::pack::{hpack, vpackage, Spec, Tolerances, NATURAL};

/// The subtype of an unset entry and of an unset row.
pub const UNSET: u8 = 255;
pub const UNSET_ROW: u8 = 254;
/// LuaTeX's hlist subtypes for a set row and a set entry.
const ALIGNMENT: u8 = 4;
const CELL: u8 = 5;

/// §801's `null_flag`: a width no entry has given yet.
const NULL_FLAG: Scaled = -0o10000000000;

/// What §159 overlays on an unset entry.
#[derive(Clone, Copy, Debug, Default)]
struct Unset {
    /// `span_count`: the columns the entry spans, less one.
    span: usize,
    stretch: Scaled,
    stretch_order: i64,
    shrink: Scaled,
    shrink_order: i64,
}

/// One alignrecord (§769): the widest entry of the column so far, and the
/// widest of the entries that start here and span further, by how many
/// columns past this one they reach (§797's span nodes).
#[derive(Clone, Debug)]
struct Record {
    width: Scaled,
    spans: std::collections::BTreeMap<usize, Scaled>,
}

/// One alignment in progress (§770's globals, pushed by §772).
#[derive(Clone, Debug)]
pub struct Align {
    halign: bool,
    spec: Spec,
    /// `tabskip[k]` is the glue before column `k` (and the last after the
    /// last column), as §778 recorded it.
    tabskip: Vec<Glue>,
    records: Vec<Record>,
    /// `cur_align` and `cur_span`, as column numbers.
    cur_align: usize,
    cur_span: usize,
    /// §783's `cur_loop`, as the column whose tabskip begins the periodic part.
    cur_loop: Option<usize>,
    /// The adjustment material moved out of the current row's entries
    /// (§770's `cur_head`..`cur_tail`).
    adjust: Vec<Node>,
    unsets: Vec<Unset>,
    /// `mode_line` of the alignment's list, for §804's report.
    mode_line: i64,
}

impl Stomach {
    fn align(&mut self) -> Option<&mut Align> {
        self.aligns.last_mut()
    }

    /// `init_align` (§774-§775) once the lowerer has read the spec:
    /// `push_alignment`, and the alignment's own list in `-vmode` for
    /// `\halign` or `-hmode` for `\valign`.
    pub fn init_align(&mut self, halign: bool, spec: Spec) {
        self.group_begin();
        let display = self.mode() == Mode::DisplayMath;
        // §775: in a display the baselines are the enclosing vertical list's.
        let prev_depth = match display {
            true => self.nest.len().checked_sub(2).map_or(0, |i| self.nest[i].prev_depth),
            false => self.cur().prev_depth,
        };
        let mode = match halign {
            true => Mode::InternalVertical,
            false => Mode::RestrictedHorizontal,
        };
        // §776: a display alignment is the display's only material.
        if display && (!self.cur().mlist.is_empty() || self.cur().incompleat.is_some()) {
            self.say(super::Said::Error("Improper \\halign inside $$'s".into()));
            let cur = self.cur_mut();
            cur.mlist.clear();
            cur.incompleat = None;
        }
        let mut l = ListState::new(mode, Opened::Align);
        l.prev_depth = prev_depth;
        l.space_factor = self.cur().space_factor;
        l.mode_line = self.line;
        self.nest.push(l);
        self.aligns.push(Align {
            halign,
            spec,
            tabskip: Vec::new(),
            records: Vec::new(),
            cur_align: 0,
            cur_span: 0,
            cur_loop: None,
            adjust: Vec::new(),
            unsets: Vec::new(),
            mode_line: self.line,
        });
    }

    /// §778: the tabskip glue in force, appended to the preamble.
    pub fn align_tabskip(&mut self, g: Glue) {
        if let Some(a) = self.align() {
            a.tabskip.push(g);
        }
    }

    /// §779: a new alignrecord, and (§783) whether the preamble repeats from
    /// it on.
    pub fn align_record(&mut self, periodic: bool) {
        if let Some(a) = self.align() {
            if periodic {
                a.cur_loop = Some(a.records.len());
            }
            a.records.push(Record {
                width: NULL_FLAG,
                spans: Default::default(),
            });
        }
    }

    /// The end of the preamble (§774): the group the entries are read in.
    pub fn align_begin_entries(&mut self) {
        self.group_begin();
    }

    /// `init_row` (§786) and the first `init_span` (§787).
    pub fn init_row(&mut self) {
        let Some(a) = self.aligns.last() else {
            return;
        };
        let halign = a.halign;
        let first = a.tabskip.first().copied().unwrap_or_default();
        let mode = match halign {
            true => Mode::RestrictedHorizontal,
            false => Mode::InternalVertical,
        };
        let mut l = ListState::new(mode, Opened::Align);
        l.space_factor = 0;
        l.prev_depth = 0;
        self.nest.push(l);
        self.tail_append(Node::Glue(GlueNode::param(first, GlueSource::TabSkip)));
        if let Some(a) = self.align() {
            a.cur_align = 0;
            a.adjust.clear();
        }
        self.init_span(0);
    }

    /// `init_span(p)` (§787).
    fn init_span(&mut self, p: usize) {
        let Some(a) = self.aligns.last() else {
            return;
        };
        let mode = match a.halign {
            true => Mode::RestrictedHorizontal,
            false => Mode::InternalVertical,
        };
        let mut l = ListState::new(mode, Opened::Align);
        match a.halign {
            true => l.space_factor = 1000,
            false => l.prev_depth = crate::node::IGNORE_DEPTH,
        }
        self.nest.push(l);
        if let Some(a) = self.align() {
            a.cur_span = p;
        }
    }

    /// §793: lengthen a periodic preamble by the record after `cur_loop`
    /// and the tabskip after that; the lowerer copies the templates.
    pub fn align_lengthen(&mut self) {
        let Some(a) = self.align() else {
            return;
        };
        let Some(lp) = a.cur_loop else {
            return;
        };
        a.records.push(Record {
            width: NULL_FLAG,
            spans: Default::default(),
        });
        let g = a.tabskip.get(lp + 1).copied().unwrap_or_default();
        a.tabskip.push(g);
        a.cur_loop = Some(lp + 1);
    }

    /// `fin_col` (§791-§798) after the entry's group has ended: `kind` is 0
    /// for `&`, 1 for `\span`, 2 for `\cr` or `\crcr`.
    pub fn fin_col(&mut self, kind: u8, tol: Tolerances) {
        let Some(a) = self.aligns.last() else {
            return;
        };
        let halign = a.halign;
        let next = a.cur_align + 1;
        if kind != 1 {
            // §791: `unsave; new_save_level(align_group)`.
            self.group_end();
            self.group_begin();
            // §796: the entry as an unset box.
            let Some(l) = self.pop_list() else {
                return;
            };
            let (u, w, stretch, shrink) = match halign {
                true => {
                    let mut adjust = Vec::new();
                    let packed = hpack(l.list, NATURAL, tol, Some(&mut adjust));
                    if let Some(a) = self.align() {
                        a.adjust.append(&mut adjust);
                    }
                    let w = packed.node.width;
                    (packed.node, w, packed.total_stretch, packed.total_shrink)
                }
                false => {
                    let packed = vpackage(l.list, NATURAL, 0, tol);
                    let w = packed.node.height;
                    (packed.node, w, packed.total_stretch, packed.total_shrink)
                }
            };
            let a = self.aligns.last_mut().expect("checked above");
            let n = a.cur_align - a.cur_span;
            match n {
                0 => {
                    let r = &mut a.records[a.cur_align];
                    r.width = r.width.max(w);
                }
                // §798: the widest entry spanning these columns.
                _ => {
                    let r = &mut a.records[a.cur_span];
                    let e = r.spans.entry(n).or_insert(w);
                    *e = (*e).max(w);
                }
            }
            let order = |t: &[Scaled; 4]| (0..4).rev().find(|&o| t[o] != 0).unwrap_or(0);
            let (so, ho) = (order(&stretch), order(&shrink));
            a.unsets.push(Unset {
                span: n,
                stretch: stretch[so],
                stretch_order: so as i64,
                shrink: shrink[ho],
                shrink_order: ho as i64,
            });
            let mut u = u;
            u.subtype = UNSET;
            u.glue_set = (a.unsets.len() - 1) as f64;
            // §795: the tabskip glue after the column.
            let g = a.tabskip.get(a.cur_align + 1).copied().unwrap_or_default();
            self.tail_append(Node::Box(u));
            self.tail_append(Node::Glue(GlueNode::param(g, GlueSource::TabSkip)));
            if kind == 2 {
                return;
            }
            self.init_span(next);
        }
        if let Some(a) = self.align() {
            a.cur_align = next;
        }
    }

    /// `fin_row` (§799).
    pub fn fin_row(&mut self, tol: Tolerances) {
        let Some(a) = self.aligns.last() else {
            return;
        };
        let halign = a.halign;
        let Some(l) = self.pop_list() else {
            return;
        };
        let mut p = match halign {
            true => hpack(l.list, NATURAL, tol, None).node,
            false => crate::pack::vpack(l.list, NATURAL, tol).node,
        };
        p.subtype = UNSET_ROW;
        p.shift_amount = 0;
        match halign {
            true => {
                self.append_to_vlist(p);
                let adjust = self.align().map(|a| std::mem::take(&mut a.adjust)).unwrap_or_default();
                for n in adjust {
                    self.tail_append(n);
                }
            }
            false => {
                self.tail_append(Node::Box(p));
                self.cur_mut().space_factor = 1000;
            }
        }
    }

    /// `fin_align` (§800-§812), after both of the alignment's groups have
    /// ended.
    pub fn fin_align(&mut self, display_indent: Scaled, tol: Tolerances) {
        let Some(mut a) = self.aligns.pop() else {
            return;
        };
        // §800: the group of the entries, then the alignment's own.
        self.group_end();
        self.group_end();
        let o = display_indent;
        // §801-§803: the column widths, spans merged left to right.
        let cols = a.records.len();
        for q in 0..cols {
            if a.records[q].width == NULL_FLAG {
                a.records[q].width = 0;
                if let Some(g) = a.tabskip.get_mut(q + 1) {
                    *g = Glue::default();
                }
            }
            let spans = std::mem::take(&mut a.records[q].spans);
            if spans.is_empty() {
                continue;
            }
            let t = a.records[q].width + a.tabskip.get(q + 1).map_or(0, |g| g.natural);
            for (n, w) in spans {
                let w = w - t;
                let Some(p) = a.records.get_mut(q + 1) else {
                    continue;
                };
                match n - 1 {
                    0 => p.width = p.width.max(w),
                    m => {
                        let e = p.spans.entry(m).or_insert(w);
                        *e = (*e).max(w);
                    }
                }
            }
        }
        // §804: the prototype row, packaged to the spec.
        let mut proto: Vec<Node> = Vec::new();
        for (k, r) in a.records.iter().enumerate() {
            let g = a.tabskip.get(k).copied().unwrap_or_default();
            proto.push(Node::Glue(GlueNode::param(g, GlueSource::TabSkip)));
            let b = match a.halign {
                true => BoxNode {
                    width: r.width,
                    ..BoxNode::null()
                },
                false => BoxNode {
                    height: r.width,
                    vertical: true,
                    ..BoxNode::null()
                },
            };
            proto.push(Node::Box(b));
        }
        let g = a.tabskip.get(cols).copied().unwrap_or_default();
        proto.push(Node::Glue(GlueNode::param(g, GlueSource::TabSkip)));
        let saved_begin = self.pack_begin_line;
        self.pack_begin_line = -a.mode_line;
        let packed = match a.halign {
            true => {
                let mut t = tol;
                t.overfull_rule = 0;
                hpack(proto, a.spec, t, None)
            }
            false => crate::pack::vpack(proto, a.spec, tol),
        };
        self.last_badness = packed.badness;
        if let Some(r) = packed.report {
            self.box_report(&r, !a.halign, &packed.node);
        }
        self.pack_begin_line = saved_begin;
        let p = packed.node;
        // §805-§811: every unset box in the alignment's list, set.
        let Some(mut l) = self.pop_list() else {
            return;
        };
        let list = std::mem::take(&mut l.list);
        let mut set = Vec::with_capacity(list.len());
        for q in list {
            match q {
                Node::Box(row) if row.subtype == UNSET_ROW => {
                    set.push(Node::Box(set_row(&a, row, &p, o)));
                }
                Node::Rule(r) => set.push(extend_rule(r, &p, o, tol)),
                other => set.push(other),
            }
        }
        // §812.
        let aux_depth = l.prev_depth;
        let aux_sf = l.space_factor;
        if self.mode() == Mode::DisplayMath {
            self.align_display = Some((set, aux_depth));
            return;
        }
        let cur = self.cur_mut();
        match a.halign {
            true => cur.prev_depth = aux_depth,
            false => cur.space_factor = aux_sf,
        }
        for n in set {
            self.tail_append(n);
        }
        if self.mode() == Mode::Vertical {
            self.want_build_page("alignment");
        }
    }

    /// §1206: an alignment that was a display, between its penalties and
    /// skips, once the lowerer has read the `$$` after it.
    pub fn finish_display_alignment(&mut self, e: &dyn super::Eqtb) {
        let Some((list, aux_depth)) = self.align_display.take() else {
            return;
        };
        // `pop_nest`: the display's mlist level.
        self.pop_list();
        let pre = self.int_par("predisplaypenalty");
        self.tail_append(Node::Penalty(pre));
        let above = super::glue_par(e, "abovedisplayskip");
        self.tail_append(Node::Glue(GlueNode::param(above, GlueSource::AboveDisplaySkip)));
        for n in list {
            self.tail_append(n);
        }
        let post = self.int_par("postdisplaypenalty");
        self.tail_append(Node::Penalty(post));
        let below = super::glue_par(e, "belowdisplayskip");
        self.tail_append(Node::Glue(GlueNode::param(below, GlueSource::BelowDisplaySkip)));
        self.cur_mut().prev_depth = aux_depth;
    }
}

/// §806: a rule in the alignment's list runs to the prototype's size, and is
/// shifted with the rows in a display.
fn extend_rule(mut r: RuleNode, p: &BoxNode, o: Scaled, tol: Tolerances) -> Node {
    if RuleNode::is_running(r.width) {
        r.width = p.width;
    }
    if RuleNode::is_running(r.height) {
        r.height = p.height;
    }
    if RuleNode::is_running(r.depth) {
        r.depth = p.depth;
    }
    if o == 0 {
        return Node::Rule(r);
    }
    let mut b = hpack(vec![Node::Rule(r)], NATURAL, tol, None).node;
    b.shift_amount = o;
    Node::Box(b)
}

/// §807-§811: an unset row and the unset entries in it, set to the
/// prototype `p`.
fn set_row(a: &Align, mut q: BoxNode, p: &BoxNode, o: Scaled) -> BoxNode {
    match a.halign {
        true => {
            q.vertical = false;
            q.width = p.width;
        }
        false => {
            q.vertical = true;
            q.height = p.height;
        }
    }
    q.glue_order = p.glue_order;
    q.glue_sign = p.glue_sign;
    q.glue_set = p.glue_set;
    q.shift_amount = o;
    q.subtype = ALIGNMENT;
    let (row_h, row_d, row_w) = (q.height, q.depth, q.width);
    // The prototype's nodes after its first tabskip: box, glue, box, ...
    let proto: Vec<&Node> = p.list.iter().skip(1).collect();
    let mut out = Vec::with_capacity(q.list.len());
    let mut s = 0usize;
    for node in std::mem::take(&mut q.list) {
        let Node::Box(mut r) = node else {
            out.push(node);
            continue;
        };
        if r.subtype != UNSET {
            out.push(Node::Box(r));
            continue;
        }
        let info = a.unsets.get(r.glue_set as usize).copied().unwrap_or_default();
        let size = |n: &Node| match n {
            Node::Box(b) => match a.halign {
                true => b.width,
                false => b.height,
            },
            _ => 0,
        };
        let mut t = proto.get(s).map_or(0, |n| size(n));
        let w = t;
        // §808-§809: tabskip glue and an empty box for each column spanned.
        let mut tail: Vec<Node> = Vec::new();
        for _ in 0..info.span {
            s += 1;
            let v = match proto.get(s) {
                Some(Node::Glue(g)) => g.spec,
                _ => Glue::default(),
            };
            tail.push(Node::Glue(GlueNode::param(v, GlueSource::TabSkip)));
            t += v.natural;
            match p.glue_sign {
                GlueSign::Stretching if v.stretch_order == p.glue_order => {
                    t += (p.glue_set * v.stretch as f64).round() as Scaled;
                }
                GlueSign::Shrinking if v.shrink_order == p.glue_order => {
                    t -= (p.glue_set * v.shrink as f64).round() as Scaled;
                }
                _ => {}
            }
            s += 1;
            let width = proto.get(s).map_or(0, |n| size(n));
            t += width;
            let empty = match a.halign {
                true => BoxNode {
                    width,
                    ..BoxNode::null()
                },
                false => BoxNode {
                    height: width,
                    vertical: true,
                    ..BoxNode::null()
                },
            };
            tail.push(Node::Box(empty));
        }
        // §810 and §811.
        let natural = match a.halign {
            true => r.width,
            false => r.height,
        };
        match a.halign {
            true => {
                r.height = row_h;
                r.depth = row_d;
            }
            false => r.width = row_w,
        }
        r.glue_set = 0.0;
        if t == natural {
            r.glue_sign = GlueSign::Normal;
            r.glue_order = 0;
        } else if t > natural {
            r.glue_sign = GlueSign::Stretching;
            r.glue_order = info.stretch_order;
            if info.stretch != 0 {
                r.glue_set = (t - natural) as f64 / info.stretch as f64;
            }
        } else {
            r.glue_order = info.shrink_order;
            r.glue_sign = GlueSign::Shrinking;
            if info.shrink == 0 {
                r.glue_set = 0.0;
            } else if info.shrink_order == 0 && natural - t > info.shrink {
                r.glue_set = 1.0;
            } else {
                r.glue_set = (natural - t) as f64 / info.shrink as f64;
            }
        }
        match a.halign {
            true => {
                r.width = w;
                r.vertical = false;
            }
            false => {
                r.height = w;
                r.vertical = true;
            }
        }
        r.shift_amount = 0;
        r.subtype = CELL;
        out.push(Node::Box(r));
        out.extend(tail);
        // Past this entry's box and the tabskip after it.
        s += 2;
    }
    q.list = out;
    q
}
