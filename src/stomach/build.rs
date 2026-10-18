//! The action routines that build lists: §1055-§1135.
//!
//! Each method here is one of `main_control`'s cases after the lowerer has
//! decided which case it is and scanned its arguments: `\hskip 3pt plus 1fil`
//! arrives as the glue, `\hrule height 2pt` as the rule, `\setbox3=\hbox to
//! 5pt{` as a box kind, a spec and a context. What is left is what §1055-§1135
//! do with them, which is decided by the mode the list is in when the program
//! gets there.

use super::{BoxContext, BoxKind, Kind, ListState, Mode, Opened, Said, Stomach};
use crate::glue::Glue;
use crate::node::{
    BoxNode, DiscNode, GlueNode, GlueSource, InsNode, LeaderKind, Node, RuleNode, Scaled,
    IGNORE_DEPTH,
};
use crate::pack::{hpack, vpackage, vtop, Spec, Tolerances, NATURAL};

/// Where `begin_box` (§1079) takes its box from, when it is not building one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// `\box n`.
    Box(u16),
    /// `\copy n`.
    Copy(u16),
    /// `\lastbox`.
    LastBox,
    /// `\vsplit n to h`.
    Vsplit(u16, Scaled),
}

/// Which of §1105's `remove_item` commands: the node type it removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removed {
    Penalty,
    Kern,
    Glue,
}

impl Stomach {
    /// The tolerances `hpack`/`vpack` read: `\hbadness`, `\vbadness`,
    /// `\hfuzz`, `\vfuzz` (§660, §674).
    pub fn tolerances(&self, e: &dyn super::Eqtb) -> Tolerances {
        Tolerances {
            hbadness: self.int_par("hbadness"),
            vbadness: self.int_par("vbadness"),
            hfuzz: super::dimen_par(e, "hfuzz"),
            vfuzz: super::dimen_par(e, "vfuzz"),
            overfull_rule: super::dimen_par(e, "overfullrule"),
        }
    }

    /// §1056: `\hrule` in vertical mode, `\vrule` in horizontal or math.
    pub fn append_rule(&mut self, rule: RuleNode) {
        self.tail_append(Node::Rule(rule));
        let cur = self.cur_mut();
        match cur.mode.kind() {
            Kind::V => cur.prev_depth = IGNORE_DEPTH,
            Kind::H => cur.space_factor = 1000,
            Kind::M => {}
        }
    }

    /// `append_glue` (§1060): `\hskip`, `\vskip`, `\hfil` and the rest, with
    /// `mu` for `\mskip`. Leaders (§1078) take the box waiting in
    /// [`Stomach::leader_box`].
    pub fn append_glue(&mut self, spec: Glue, mu: bool, leaders: Option<LeaderKind>) {
        let mut g = GlueNode::new(spec);
        if mu {
            g.source = GlueSource::MuGlue;
        }
        if let Some(kind) = leaders {
            g.kind = kind;
            g.leader = self.leader_box.take().map(Box::new);
        }
        self.tail_append(Node::Glue(g));
    }

    /// `append_kern` (§1061). texrs's kern node records `explicit` only; a
    /// `\mkern` is turned into an ordinary kern when its formula is set, as
    /// §732 does, so it is carried as explicit here.
    pub fn append_kern(&mut self, width: Scaled) {
        self.tail_append(Node::Kern {
            width,
            explicit: true,
        });
    }

    /// `append_penalty` (§1103).
    pub fn append_penalty(&mut self, value: i64) {
        self.tail_append(Node::Penalty(value));
        if self.mode() == Mode::Vertical {
            self.want_build_page("penalty");
        }
    }

    /// `make_mark` (§1101), with the mark's text already expanded.
    pub fn make_mark(&mut self, text: String) {
        self.tail_append(Node::Mark(text));
    }

    /// §1083: open the list of an `\hbox`, `\vbox` or `\vtop`, with §1083's
    /// `saved(0)` -- the context -- and §645's spec carried on the nest
    /// rather than on the save stack.
    pub fn begin_box(&mut self, kind: BoxKind, spec: Spec, context: BoxContext) {
        self.group_begin();
        let mode = match kind {
            BoxKind::H => Mode::RestrictedHorizontal,
            BoxKind::V | BoxKind::Top => Mode::InternalVertical,
        };
        let mut l = ListState::new(
            mode,
            Opened::Box {
                kind,
                spec,
                context,
                max_depth: crate::node::MAX_DIMEN,
            },
        );
        l.prev_depth = IGNORE_DEPTH;
        l.space_factor = 1000;
        self.nest.push(l);
    }

    /// The `\boxmaxdepth` in force as the box's group ends, which §1086
    /// reads before `unsave`.
    pub fn note_box_max_depth(&mut self, d: Scaled) {
        if let Opened::Box { max_depth, .. } = &mut self.cur_mut().opened {
            *max_depth = d;
        }
    }

    /// `package(c)` (§1086), after the list's paragraph (if any) has ended:
    /// `unsave`, pack, pop, and `box_end` with the context the box began
    /// with.
    pub fn package(&mut self, tol: Tolerances) {
        let Opened::Box {
            kind,
            spec,
            context,
            max_depth,
        } = self.cur().opened.clone()
        else {
            return;
        };
        self.group_end();
        let Some(l) = self.pop_list() else {
            return;
        };
        let packed = match kind {
            BoxKind::H => self.hpack_filtered(l.list, spec, tol),
            BoxKind::V => vpackage(l.list, spec, max_depth, tol),
            BoxKind::Top => vtop(vpackage(l.list, spec, max_depth, tol)),
        };
        self.last_badness = packed.badness;
        if let Some(r) = packed.report {
            self.box_report(&r, kind != BoxKind::H, &packed.node);
        }
        let mut b = packed.node;
        b.subtype = crate::node::BOX_BOX;
        self.box_end(Some(b), context);
    }

    /// `hpack` with LuaTeX's `hpack_filter` callback in front of it (see
    /// `crate::node::hooks`).
    pub fn hpack_filtered(
        &mut self,
        mut list: Vec<Node>,
        spec: Spec,
        tol: Tolerances,
    ) -> crate::pack::Packed {
        crate::node::hooks::hpack_filter(&mut list, spec);
        hpack(list, spec, tol, None)
    }

    /// `pop_nest` (§217), returning the list.
    pub fn pop_list(&mut self) -> Option<ListState> {
        match self.nest.len() > 1 {
            true => self.nest.pop(),
            false => None,
        }
    }

    /// §660-§663 and §674-§675: what tex prints about a box it found bad --
    /// the report and where it happened on the terminal, the list in §174's
    /// short form for an hbox, and the whole box as a diagnostic.
    pub fn box_report(&mut self, r: &crate::pack::Report, vertical: bool, b: &BoxNode) {
        let mut text = r.message(vertical);
        // `message` closes the parenthesis; §663 closes it with the place.
        text.pop();
        let pack_begin_line = self.pack_begin_line;
        if self.output_active() {
            text.push_str(") has occurred while \\output is active");
        } else {
            match pack_begin_line {
                0 => text.push_str(") detected at line "),
                l if l > 0 && !vertical => text.push_str(") in paragraph at lines "),
                _ => text.push_str(") in alignment at lines "),
            }
            if pack_begin_line != 0 {
                text.push_str(&format!("{}--", pack_begin_line.abs()));
            }
            text.push_str(&self.line.to_string());
        }
        if !vertical {
            let esc = self.esc_text();
            let d = crate::node::show::Display {
                depth: 0,
                breadth: 0,
                esc: &esc,
                fonts: self,
            };
            let mut font = None;
            d.short_display(&mut text, &b.list, &mut font);
        }
        self.say(Said::Report(text));
        let shown = self.show_box_text(std::slice::from_ref(&Node::Box(b.clone())));
        self.diagnostic(&shown);
    }

    /// `box_end` (§1075-§1078).
    pub fn box_end(&mut self, cur_box: Option<BoxNode>, context: BoxContext) {
        match context {
            // §1076.
            BoxContext::Shift(s) => {
                let Some(mut b) = cur_box else {
                    return;
                };
                b.shift_amount = s;
                match self.mode().kind() {
                    Kind::V => {
                        self.append_to_vlist(b);
                        if self.mode().outer() {
                            self.want_build_page("box");
                        }
                    }
                    Kind::H => {
                        self.cur_mut().space_factor = 1000;
                        self.tail_append(Node::Box(b));
                    }
                    // A box in a formula is an `ord_noad` holding it; the
                    // formula machinery is the math lowering's, so the box
                    // goes in as the box.
                    Kind::M => self.tail_append(Node::Box(b)),
                }
            }
            // §1077.
            BoxContext::SetBox { n, global } => self.set_box(n, cur_box, global),
            BoxContext::ShipOut => {
                if let Some(b) = cur_box {
                    self.ship_out_box(b);
                }
            }
            // §1078: the glue comes next and is the lowerer's to read.
            BoxContext::Leaders(_) => {
                self.leader_box = cur_box.map(Node::Box);
            }
        }
    }

    /// A rule standing where a leader box is wanted (§1084).
    pub fn leader_rule(&mut self, rule: RuleNode) {
        self.leader_box = Some(Node::Rule(rule));
    }

    /// `append_to_vlist` (§679), reading the current list's `\prevdepth`
    /// and the interline glue parameters as they stand.
    pub fn append_to_vlist(&mut self, b: BoxNode) {
        let baselines = self.baselines;
        let cur = self.cur_mut();
        crate::pack::append_to_vlist(&mut cur.list, b, &mut cur.prev_depth, baselines);
    }

    /// `begin_box` (§1079) for the four commands that do not build a list:
    /// the box comes from a register or from the current list, and goes to
    /// `box_end` at once.
    pub fn fetch_box(&mut self, source: Source, context: BoxContext, tol: Tolerances) {
        let cur_box = match source {
            Source::Box(n) => self.take_box(n),
            Source::Copy(n) => self.box_register(n).cloned(),
            Source::LastBox => self.last_box(),
            Source::Vsplit(n, h) => self.vsplit(n, h, tol),
        };
        self.box_end(cur_box, context);
    }

    /// §1080-§1081: `\lastbox`.
    fn last_box(&mut self) -> Option<BoxNode> {
        let cur = self.cur();
        if cur.mode.kind() == Kind::M {
            self.say(Said::Error("You can't use `\\lastbox' in math mode".into()));
            return None;
        }
        if cur.mode == Mode::Vertical && cur.list.is_empty() {
            self.say(Said::Error("You can't use `\\lastbox' in vertical mode".into()));
            return None;
        }
        if !matches!(cur.list.last(), Some(Node::Box(_))) {
            return None;
        }
        if self.tail_is_replaced() {
            return None;
        }
        match self.cur_mut().list.pop() {
            Some(Node::Box(mut b)) => {
                b.shift_amount = 0;
                Some(b)
            }
            _ => None,
        }
    }

    /// §1081's and §1105's walk: whether the tail of the current list is
    /// one of the nodes a discretionary REPLACES, which neither `\lastbox`
    /// nor `\unskip` may take.
    fn tail_is_replaced(&self) -> bool {
        let list = &self.cur().list;
        let last = list.len().saturating_sub(1);
        let mut i = 0;
        while i < last {
            if let Node::Disc(d) = &list[i] {
                if d.replace_count > 0 && i + d.replace_count >= last {
                    return true;
                }
                i += d.replace_count;
            }
            i += 1;
        }
        false
    }

    /// `vsplit(n,h)` (§977-§979).
    fn vsplit(&mut self, n: u16, h: Scaled, tol: Tolerances) -> Option<BoxNode> {
        self.output.split_first_mark = None;
        self.output.split_bot_mark = None;
        let v = self.box_register(n)?.clone();
        if !v.vertical {
            self.say(Said::Error("\\vsplit needs a \\vbox".into()));
            return None;
        }
        let split = crate::page::vsplit(
            v,
            h,
            self.split_max_depth,
            self.split_top_skip,
            tol,
        )?;
        self.output.split_first_mark = split.first_mark;
        self.output.split_bot_mark = split.bot_mark;
        self.last_badness = split.badness;
        if let Some(r) = split.report {
            self.box_report(&r, true, &split.extracted);
        }
        // §977: the register keeps the remainder at the level it has.
        self.dirty.push(n);
        if let Some(r) = self.registers.get_mut(n as usize) {
            r.value = split.remainder;
        }
        Some(split.extracted)
    }

    /// `unpackage` (§1110): `\unhbox`, `\unhcopy`, `\unvbox`, `\unvcopy`.
    pub fn unpackage(&mut self, n: u16, copy: bool) {
        let Some(p) = self.box_register(n) else {
            return;
        };
        let incompatible = match self.mode().kind() {
            Kind::M => true,
            Kind::V => !p.vertical,
            Kind::H => p.vertical,
        };
        if incompatible {
            self.say(Said::Error("Incompatible list can't be unboxed".into()));
            return;
        }
        let list = match copy {
            true => p.list.clone(),
            false => self.take_box(n).map(|b| b.list).unwrap_or_default(),
        };
        self.cur_mut().list.extend(list);
    }

    /// `delete_last` (§1105): `\unpenalty`, `\unkern`, `\unskip`.
    pub fn delete_last(&mut self, what: Removed) {
        let cur = self.cur();
        if cur.mode == Mode::Vertical && cur.list.is_empty() {
            // §1106: the page builder has the node; only `\unskip` with no
            // glue there to remove is allowed to say nothing.
            let quiet = what == Removed::Glue && self.output.last_glue_is_none();
            if !quiet {
                let name = match what {
                    Removed::Penalty => "\\unpenalty",
                    Removed::Kern => "\\unkern",
                    Removed::Glue => "\\unskip",
                };
                self.say(Said::Error(format!("You can't use `{name}' in vertical mode")));
            }
            return;
        }
        let matches = match (cur.list.last(), what) {
            (Some(Node::Penalty(_)), Removed::Penalty) => true,
            (Some(Node::Kern { .. }), Removed::Kern) => true,
            (Some(Node::Glue(_)), Removed::Glue) => true,
            _ => false,
        };
        if matches && !self.tail_is_replaced() {
            self.cur_mut().list.pop();
        }
    }

    /// `append_discretionary` (§1117) for `\-`: an empty discretionary
    /// whose pre-break text is the font's hyphen character.
    pub fn append_hyphen(&mut self, f: usize) {
        let font = self.font(f);
        let mut d = DiscNode::default();
        let c = font.hyphen_char;
        if (0..256).contains(&c) {
            let ch = char::from(c as u8);
            if let Some(m) = font.metrics(ch) {
                d.pre_break.push(Node::Char(crate::node::CharNode {
                    font: f,
                    character: ch,
                    width: m.width,
                    height: m.height,
                    depth: m.depth,
                }));
            }
        }
        self.tail_append(Node::Disc(d));
    }

    /// `\discretionary` (§1117-§1121): the node, then three lists built in
    /// turn in restricted horizontal mode. `begin_disc_list` opens each and
    /// `end_disc_list` (§1119) closes it.
    pub fn begin_discretionary(&mut self) {
        self.tail_append(Node::Disc(DiscNode::default()));
        self.begin_disc_list();
    }

    fn begin_disc_list(&mut self) {
        self.group_begin();
        let mut l = ListState::new(Mode::RestrictedHorizontal, Opened::Page);
        l.space_factor = 1000;
        self.nest.push(l);
    }

    /// `build_discretionary` (§1119-§1121) for list `which` (0 pre-break, 1
    /// post-break, 2 replacement). Returns whether another list follows.
    pub fn end_disc_list(&mut self, which: u8) -> bool {
        self.group_end();
        let Some(l) = self.pop_list() else {
            return false;
        };
        // §1121: only characters, boxes, rules, kerns and ligatures.
        let mut list = Vec::with_capacity(l.list.len());
        for node in l.list {
            let fine = node.is_char()
                || matches!(
                    node,
                    Node::Box(_) | Node::Rule(_) | Node::Kern { .. } | Node::Ligature(..)
                );
            if !fine {
                self.say(Said::Error("Improper discretionary list".into()));
                break;
            }
            list.push(node);
        }
        let n = list.len();
        let mode = self.mode();
        let Some(Node::Disc(d)) = self.cur_mut().list.last_mut() else {
            return false;
        };
        match which {
            0 => d.pre_break = list,
            1 => d.post_break = list,
            _ => {
                // §1120.
                if n > 0 && mode.kind() == Kind::M {
                    self.say(Said::Error("Illegal math \\discretionary".into()));
                    return false;
                }
                d.replace_count = n;
                self.cur_mut().list.extend(list);
                return false;
            }
        }
        self.begin_disc_list();
        true
    }

    /// `begin_insert_or_adjust` (§1099): `n` is 255 for `\vadjust`.
    pub fn begin_insert(&mut self, n: u16) {
        self.group_begin();
        let mut l = ListState::new(Mode::InternalVertical, Opened::Insert(n));
        l.prev_depth = IGNORE_DEPTH;
        self.nest.push(l);
    }

    /// §1100's `insert_group` case: the list becomes an insertion node, or
    /// an adjust node for `\vadjust`. The three parameters are read before
    /// `unsave`, as §1100 reads them.
    pub fn end_insert(
        &mut self,
        split_top_skip: Glue,
        split_max_depth: Scaled,
        floating_penalty: i64,
        tol: Tolerances,
    ) {
        let Opened::Insert(n) = self.cur().opened else {
            return;
        };
        self.group_end();
        let Some(l) = self.pop_list() else {
            return;
        };
        let p = crate::pack::vpack(l.list, NATURAL, tol).node;
        let node = match n < 255 {
            true => Node::Ins(InsNode {
                number: n as u8,
                height: p.height + p.depth,
                depth: split_max_depth,
                float_cost: floating_penalty,
                split_top_skip,
                list: p.list,
            }),
            false => Node::Adjust(p.list),
        };
        self.tail_append(node);
        if self.nest.len() == 1 {
            self.want_build_page(if n < 255 { "insert" } else { "adjust" });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §1076: a box appended in horizontal mode resets the space factor,
    /// and in vertical mode gets interline glue from the box before it.
    #[test]
    fn a_box_appended_to_a_vertical_list_gets_baselineskip() {
        let mut s = Stomach::new();
        s.baselines = crate::pack::Baselines::plain();
        let line = |h, d| BoxNode {
            height: h,
            depth: d,
            ..BoxNode::null()
        };
        s.box_end(Some(line(7 * 65536, 2 * 65536)), BoxContext::Shift(0));
        s.box_end(Some(line(7 * 65536, 0)), BoxContext::Shift(0));
        let list = &s.cur().list;
        assert_eq!(list.len(), 3);
        match &list[1] {
            Node::Glue(g) => assert_eq!(g.spec.natural, 3 * 65536),
            other => panic!("expected baselineskip glue, got {other:?}"),
        }
    }

    /// §1110: unboxing an hbox in vertical mode is refused, and the register
    /// keeps its box.
    #[test]
    fn an_hbox_cannot_be_unboxed_in_vertical_mode() {
        let mut s = Stomach::new();
        s.set_box(1, Some(BoxNode::null()), false);
        s.unpackage(1, false);
        assert!(s.box_register(1).is_some());
        assert!(matches!(s.said.last(), Some(Said::Error(e)) if e.contains("Incompatible")));
    }

    /// §1105: `\unskip` takes glue off the end and nothing else.
    #[test]
    fn unskip_removes_only_glue() {
        let mut s = Stomach::new();
        s.begin_box(BoxKind::H, NATURAL, BoxContext::Shift(0));
        s.append_kern(5);
        s.delete_last(Removed::Glue);
        assert_eq!(s.cur().list.len(), 1);
        s.append_glue(Glue::fixed(1), false, None);
        s.delete_last(Removed::Glue);
        assert_eq!(s.cur().list.len(), 1);
    }

    /// §1086: `\setbox` inside a box's own group is undone when the box
    /// closes, because the box's `}` ends a group.
    #[test]
    fn a_setbox_inside_a_box_is_local_to_it() {
        let mut s = Stomach::new();
        s.begin_box(BoxKind::H, NATURAL, BoxContext::SetBox { n: 2, global: false });
        s.set_box(3, Some(BoxNode::null()), false);
        s.package(Tolerances::default());
        assert!(s.box_register(3).is_none());
        assert!(s.box_register(2).is_some());
    }
}
