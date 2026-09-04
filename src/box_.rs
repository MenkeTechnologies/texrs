//! The boxes a document can nest: `\hbox`, `\vbox`, `\vtop`, the rules, the
//! shifts, the unboxing, and the glue and kerns that go between them.
//!
//! `tex.web` §1056-§1110. Everything here is a thin layer over
//! [`crate::pack`]: `\hbox to 100pt{...}` IS `hpack(list, 100pt, exactly)`,
//! and `\raise 3pt\hbox{...}` is that box with `shift_amount = -3pt`. The
//! value of writing it out is that the SIGNS and the DEFAULTS are where the
//! mistakes live:
//!
//! - `\raise` and `\moveleft` are NEGATIVE shifts, `\lower` and `\moveright`
//!   positive (§1076, and `primitive("raise",vmove,1)` against
//!   `primitive("lower",vmove,0)` at §1071). A sign error here is a page that
//!   looks nearly right.
//! - An `\hrule` is 0.4pt tall, no deeper, and as wide as whatever holds it; a
//!   `\vrule` is 0.4pt wide and as tall as whatever holds it (§463's
//!   `default_rule` and `scan_rule_spec`). "Running" is not zero: it is
//!   `null_flag`, resolved when the enclosing box is packaged.
//! - An `\hrule` in a vertical list sets `\prevdepth` to `ignore_depth`, so
//!   the box after it gets NO interline glue (§1056's note). A rule with
//!   baselineskip glue under it would sit at the wrong distance from the text
//!   below, which is exactly the `\hline` in a table.
//! - `\unhbox` empties the register; `\unhcopy` does not (§1110). Unboxing an
//!   hbox in vertical mode is an error rather than a coercion.
//!
//! The register state — `\box`, `\dimen`, `\skip` — is reached through
//! [`Registers`] rather than being owned here, so the engine's own register
//! table can be plugged in without this module knowing what it is.

use crate::dimen::UNITY;
use crate::glue::Glue;
use crate::node::{BoxNode, GlueNode, LeaderKind, Node, RuleNode, Scaled, IGNORE_DEPTH, NULL_FLAG};
use crate::pack::{hpack, vpack, vpackage, vtop, Baselines, Packed, Spec, Tolerances};
use std::collections::BTreeMap;

/// `default_rule` (§463): 0.4pt, the thickness of every rule TeX draws unless
/// the document says otherwise.
pub const DEFAULT_RULE: Scaled = 26214;

/// The register state a box construction reads and writes.
///
/// A trait rather than a struct because `\box`, `\dimen` and `\skip` are the
/// ENGINE's registers, scoped by the same save stack as `\count` and
/// `\def` — they belong with the rest of the register table, not here. Any
/// type that answers these seven questions can drive this module.
pub trait Registers {
    /// `\box⟨n⟩` as it stands, without emptying it.
    fn box_register(&self, n: u8) -> Option<&BoxNode>;
    /// `\box⟨n⟩`, leaving the register void — which is what `\box` and
    /// `\unhbox` do and what `\copy` and `\unhcopy` do not (§1079, §1110).
    fn take_box(&mut self, n: u8) -> Option<BoxNode>;
    /// `\setbox⟨n⟩=`.
    fn set_box(&mut self, n: u8, value: Option<BoxNode>);
    /// `\dimen⟨n⟩`.
    fn dimen(&self, n: u8) -> Scaled;
    fn set_dimen(&mut self, n: u8, value: Scaled);
    /// `\skip⟨n⟩`.
    fn skip(&self, n: u8) -> Glue;
    fn set_skip(&mut self, n: u8, value: Glue);
}

/// A plain register table, for a caller that has none of its own.
#[derive(Clone, Debug, Default)]
pub struct Boxes {
    boxes: BTreeMap<u8, BoxNode>,
    dimens: BTreeMap<u8, Scaled>,
    skips: BTreeMap<u8, Glue>,
}

impl Registers for Boxes {
    fn box_register(&self, n: u8) -> Option<&BoxNode> {
        self.boxes.get(&n)
    }

    fn take_box(&mut self, n: u8) -> Option<BoxNode> {
        self.boxes.remove(&n)
    }

    fn set_box(&mut self, n: u8, value: Option<BoxNode>) {
        match value {
            Some(b) => self.boxes.insert(n, b),
            None => self.boxes.remove(&n),
        };
    }

    fn dimen(&self, n: u8) -> Scaled {
        self.dimens.get(&n).copied().unwrap_or(0)
    }

    fn set_dimen(&mut self, n: u8, value: Scaled) {
        self.dimens.insert(n, value);
    }

    fn skip(&self, n: u8) -> Glue {
        self.skips.get(&n).copied().unwrap_or_default()
    }

    fn set_skip(&mut self, n: u8, value: Glue) {
        self.skips.insert(n, value);
    }
}

/// Which of the three box constructions is being closed (§1085).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// `\hbox`.
    H,
    /// `\vbox`: the reference point is the last baseline.
    V,
    /// `\vtop`: the reference point is the FIRST baseline.
    Top,
}

/// `package(c)` (§1086): close a box construction.
///
/// `max_depth` is `\boxmaxdepth`, which `\vbox` and `\vtop` obey and `\hbox`
/// has no use for.
pub fn package(
    kind: Kind,
    list: Vec<Node>,
    spec: Spec,
    max_depth: Scaled,
    tol: Tolerances,
) -> Packed {
    match kind {
        Kind::H => hpack(list, spec, tol, None),
        Kind::V => vpackage(list, spec, max_depth, tol),
        Kind::Top => vtop(vpackage(list, spec, max_depth, tol)),
    }
}

/// `\raise⟨d⟩⟨box⟩` (§1071, §1076): up is a NEGATIVE shift, because
/// `hlist_out` sets `cur_v := base_line + shift_amount` and DVI's vertical
/// axis points down.
pub fn raise(mut b: BoxNode, by: Scaled) -> BoxNode {
    b.shift_amount = -by;
    b
}

/// `\lower⟨d⟩⟨box⟩`.
pub fn lower(mut b: BoxNode, by: Scaled) -> BoxNode {
    b.shift_amount = by;
    b
}

/// `\moveleft⟨d⟩⟨box⟩`: a negative shift, `vlist_out` setting
/// `cur_h := left_edge + shift_amount`.
pub fn move_left(mut b: BoxNode, by: Scaled) -> BoxNode {
    b.shift_amount = -by;
    b
}

/// `\moveright⟨d⟩⟨box⟩`.
pub fn move_right(mut b: BoxNode, by: Scaled) -> BoxNode {
    b.shift_amount = by;
    b
}

/// `scan_rule_spec` for `\hrule` (§463): 0.4pt tall, no depth, running width.
pub fn hrule() -> RuleNode {
    RuleNode {
        width: NULL_FLAG,
        height: DEFAULT_RULE,
        depth: 0,
    }
}

/// `scan_rule_spec` for `\vrule` (§463): 0.4pt wide, running height and depth.
pub fn vrule() -> RuleNode {
    RuleNode {
        width: DEFAULT_RULE,
        height: NULL_FLAG,
        depth: NULL_FLAG,
    }
}

/// `\hfil`, `\hfill`, `\hss`, `\hfilneg` and their vertical twins (§1058).
///
/// The four are glue specifications, not commands: `\hfil` is
/// `0pt plus 1fil` and `\hss` is `0pt plus 1fil minus 1fil`, which is why
/// `\hss` can pull a box back past the measure and `\hfil` cannot.
pub mod fill {
    use super::{Glue, UNITY};

    /// `fil_glue` (§224): `0pt plus 1fil`.
    pub fn fil() -> Glue {
        Glue {
            stretch: UNITY,
            stretch_order: 1,
            ..Glue::default()
        }
    }

    /// `fill_glue`: `0pt plus 1fill`.
    pub fn fill() -> Glue {
        Glue {
            stretch: UNITY,
            stretch_order: 2,
            ..Glue::default()
        }
    }

    /// `\hfilll`/`\vfilll` are not primitives; plain.tex defines `\hfilll` as
    /// `\hskip 0pt plus 1filll`. Here for the callers that need the order.
    pub fn filll() -> Glue {
        Glue {
            stretch: UNITY,
            stretch_order: 3,
            ..Glue::default()
        }
    }

    /// `ss_glue`: `0pt plus 1fil minus 1fil`.
    pub fn ss() -> Glue {
        Glue {
            stretch: UNITY,
            stretch_order: 1,
            shrink: UNITY,
            shrink_order: 1,
            ..Glue::default()
        }
    }

    /// `fil_neg_glue`: `0pt plus -1fil`, which cancels one `\hfil` exactly.
    pub fn fil_neg() -> Glue {
        Glue {
            stretch: -UNITY,
            stretch_order: 1,
            ..Glue::default()
        }
    }
}

/// What went wrong with a box operation the document asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BoxError {
    /// §1110: `\unhbox` in vertical mode, or `\unvbox` in horizontal mode.
    IncompatibleList,
}

impl std::fmt::Display for BoxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoxError::IncompatibleList => write!(f, "Incompatible list can't be unboxed"),
        }
    }
}

/// Which list is being built (§211's `mode`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Horizontal,
    Vertical,
}

/// A list under construction, with the vertical-mode bookkeeping `tex.web`
/// keeps in `nest` (§212-§218).
///
/// The only state a horizontal list needs is the list; a vertical one also
/// carries `\prevdepth`, because the glue above the next box is computed from
/// the depth of the last one (§679).
pub struct ListBuilder {
    pub mode: Mode,
    pub list: Vec<Node>,
    /// `\prevdepth`. `ignore_depth` until the first box, and reset to it after
    /// a rule (§1056).
    pub prev_depth: Scaled,
    pub baselines: Baselines,
    pub tolerances: Tolerances,
    /// `\boxmaxdepth`.
    pub box_max_depth: Scaled,
}

impl ListBuilder {
    pub fn new(mode: Mode) -> ListBuilder {
        ListBuilder {
            mode,
            list: Vec::new(),
            prev_depth: IGNORE_DEPTH,
            baselines: Baselines::plain(),
            tolerances: Tolerances::plain(),
            box_max_depth: crate::node::MAX_DIMEN,
        }
    }

    /// Put a box on the list.
    ///
    /// In vertical mode this is `append_to_vlist` (§679) and inserts the
    /// interline glue; in horizontal mode a box simply goes next to what is
    /// already there.
    pub fn box_(&mut self, b: BoxNode) {
        match self.mode {
            Mode::Vertical => crate::pack::append_to_vlist(
                &mut self.list,
                b,
                &mut self.prev_depth,
                self.baselines,
            ),
            Mode::Horizontal => self.list.push(Node::Box(b)),
        }
    }

    /// `\hrule` or `\vrule`.
    ///
    /// A rule in a VERTICAL list disables the next interline calculation
    /// (§1056: "baselineskip calculations are disabled after a rule in
    /// vertical mode, by setting `prev_depth:=ignore_depth`"), which is what
    /// puts an `\hline` hard against the row under it.
    pub fn rule(&mut self, rule: RuleNode) {
        self.list.push(Node::Rule(rule));
        if self.mode == Mode::Vertical {
            self.prev_depth = IGNORE_DEPTH;
        }
    }

    /// `\hskip`, `\vskip`, `\hfil` and the rest: glue.
    pub fn glue(&mut self, spec: Glue) {
        self.list.push(Node::Glue(GlueNode::new(spec)));
    }

    /// `\leaders`, `\cleaders`, `\xleaders` (§1078): glue that is filled with
    /// copies of a box or a rule instead of being left blank.
    pub fn leaders(&mut self, spec: Glue, kind: LeaderKind, leader: Node) {
        self.list.push(Node::Glue(GlueNode {
            spec,
            kind,
            leader: Some(Box::new(leader)),
            ..GlueNode::default()
        }));
    }

    /// `\kern⟨d⟩` (§1057): a rigid distance, and — unlike glue — one that is
    /// NOT a legal breakpoint unless glue follows it.
    pub fn kern(&mut self, width: Scaled) {
        self.list.push(Node::Kern {
            width,
            explicit: true,
        });
    }

    /// `\penalty⟨n⟩`, `\break` (`\penalty-10000`), `\nobreak`
    /// (`\penalty10000`).
    pub fn penalty(&mut self, value: i64) {
        self.list.push(Node::Penalty(value));
    }

    /// `\mark{...}`.
    pub fn mark(&mut self, text: String) {
        self.list.push(Node::Mark(text));
    }

    /// `\unhbox⟨n⟩`, `\unhcopy⟨n⟩`, `\unvbox⟨n⟩`, `\unvcopy⟨n⟩` (§1110).
    ///
    /// The contents of the register are spilled onto the list WITHOUT the box
    /// around them, so the glue inside becomes the current list's glue and can
    /// be set again — which is the whole reason `\unvbox` exists rather than
    /// `\box`. The box itself is discarded, so its own `glue_set` is lost.
    ///
    /// `copy` is `\unhcopy`/`\unvcopy`: the register keeps its contents.
    pub fn unbox(
        &mut self,
        registers: &mut dyn Registers,
        n: u8,
        copy: bool,
    ) -> Result<(), BoxError> {
        let Some(b) = registers.box_register(n) else {
            // §1110: a void register unboxes to nothing at all, silently.
            return Ok(());
        };
        // §1110: an hbox may only be unboxed in horizontal mode and a vbox in
        // vertical mode.
        let compatible = match self.mode {
            Mode::Horizontal => !b.vertical,
            Mode::Vertical => b.vertical,
        };
        if !compatible {
            return Err(BoxError::IncompatibleList);
        }
        let contents = match copy {
            true => b.list.clone(),
            false => registers.take_box(n).map(|b| b.list).unwrap_or_default(),
        };
        self.list.extend(contents);
        Ok(())
    }

    /// `\lastbox` (§1080, §1081): take the last box off the list and hand it
    /// back, leaving the list one node shorter.
    ///
    /// This is how `\unskip\setbox0=\lastbox` reaches into a finished line to
    /// re-measure it, and it is the one place a document can look INSIDE the
    /// list it is building. Three refusals are `tex.web`'s and all three
    /// matter:
    ///
    /// - The last node must actually be a box; a rule, a kern or a character
    ///   yields nothing (§1081: "if not is_char_node(tail) then if
    ///   type(tail)=hlist_node or vlist_node").
    /// - A box that is one of a discretionary's `replace_count` nodes is NOT
    ///   taken, because it belongs to the discretionary rather than to the
    ///   list, and removing it would leave the disc pointing past the end.
    /// - The shift is cleared (`shift_amount(cur_box):=0`): a `\raise`d box
    ///   taken by `\lastbox` comes back level, because the raise was a
    ///   property of where it sat and not of the box.
    ///
    /// `tex.web` also refuses `\lastbox` on the MAIN vertical list, since that
    /// list has already gone to the page builder (§1081). A `ListBuilder` is
    /// never that list — the page builder owns its own — so the refusal has no
    /// counterpart here.
    pub fn last_box(&mut self) -> Option<BoxNode> {
        if !matches!(self.list.last(), Some(Node::Box(_))) {
            return None;
        }
        if self.last_is_a_discretionarys_replacement() {
            return None;
        }
        let Some(Node::Box(mut b)) = self.list.pop() else {
            return None;
        };
        b.shift_amount = 0;
        Some(b)
    }

    /// `\unpenalty`, `\unkern`, `\unskip` (§1105's `delete_last`): remove the
    /// last node of the list if it is of the named kind.
    ///
    /// Silently does nothing when the last node is something else, which is
    /// what makes `\unskip` safe to write at the end of a macro that may or
    /// may not have left a space behind. Returns what was removed.
    pub fn remove_last(&mut self, kind: Removable) -> Option<Node> {
        let wanted = match kind {
            Removable::Glue => 10,
            Removable::Kern => 11,
            Removable::Penalty => 12,
        };
        if self.list.last().map(Node::type_code) != Some(wanted) {
            return None;
        }
        if self.last_is_a_discretionarys_replacement() {
            return None;
        }
        self.list.pop()
    }

    /// §1081 and §1105 both walk the list from the head to find out whether
    /// the tail is one of the `replace_count` nodes a discretionary owns, and
    /// both give up when it is.
    fn last_is_a_discretionarys_replacement(&self) -> bool {
        let Some(tail) = self.list.len().checked_sub(1) else {
            return false;
        };
        for (i, node) in self.list.iter().enumerate() {
            if let Node::Disc(d) = node {
                if tail > i && tail <= i + d.replace_count {
                    return true;
                }
            }
        }
        false
    }

    /// Close the list as a box.
    pub fn package(self, kind: Kind, spec: Spec) -> Packed {
        package(kind, self.list, spec, self.box_max_depth, self.tolerances)
    }
}

/// Which kind of node `\unskip`, `\unkern` or `\unpenalty` will take off the
/// end of the list (§1105: "cur_chr is the type of node that will be deleted,
/// if present").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removable {
    /// `\unskip`.
    Glue,
    /// `\unkern`.
    Kern,
    /// `\unpenalty`.
    Penalty,
}

/// Which of a box's three dimensions is being read or written (§1247).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dimension {
    /// `\wd`.
    Width,
    /// `\ht`.
    Height,
    /// `\dp`.
    Depth,
}

/// `\wd⟨n⟩`, `\ht⟨n⟩`, `\dp⟨n⟩` as a value (§1246).
///
/// A void register reads as zero, which is what `\ifdim\wd0=0pt` relies on.
pub fn box_dimen(registers: &dyn Registers, n: u8, which: Dimension) -> Scaled {
    let Some(b) = registers.box_register(n) else {
        return 0;
    };
    match which {
        Dimension::Width => b.width,
        Dimension::Height => b.height,
        Dimension::Depth => b.depth,
    }
}

/// `\wd⟨n⟩=⟨dimen⟩` (§1247's `alter_box_dimen`).
///
/// This changes what the box CLAIMS to be, not what is in it. A box whose
/// width is set to zero still draws everything it holds, which is how
/// `\rlap` and `\llap` work and why `\wd` is a lie a document tells on
/// purpose. A void register is left void rather than being conjured into one.
pub fn set_box_dimen(registers: &mut dyn Registers, n: u8, which: Dimension, value: Scaled) {
    let Some(mut b) = registers.take_box(n) else {
        return;
    };
    match which {
        Dimension::Width => b.width = value,
        Dimension::Height => b.height = value,
        Dimension::Depth => b.depth = value,
    }
    registers.set_box(n, Some(b));
}

/// `\vsplit⟨n⟩ to ⟨dimen⟩` (§977) against a register, leaving the remainder
/// behind in it.
pub fn vsplit_register(
    registers: &mut dyn Registers,
    n: u8,
    height: Scaled,
    split_max_depth: Scaled,
    split_top_skip: Glue,
    tol: Tolerances,
) -> Option<crate::page::Split> {
    let boxed = registers.take_box(n)?;
    let split = crate::page::vsplit(boxed, height, split_max_depth, split_top_skip, tol)?;
    registers.set_box(n, split.remainder.clone());
    Some(split)
}

/// `\vbox` at its natural size, which is the shape nearly every use takes.
pub fn vbox(list: Vec<Node>, tol: Tolerances) -> BoxNode {
    vpack(list, crate::pack::NATURAL, tol).node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::GlueSource;
    use crate::pack::NATURAL;

    fn pt(n: i64) -> Scaled {
        n * UNITY
    }

    fn boxed(width: Scaled, height: Scaled, depth: Scaled, vertical: bool) -> BoxNode {
        BoxNode {
            width,
            height,
            depth,
            vertical,
            ..BoxNode::null()
        }
    }

    /// §1071: `\raise` is up, and up is negative, because `hlist_out` adds the
    /// shift to a downward coordinate. Getting this backwards puts every
    /// superscript below the line.
    #[test]
    fn raise_is_negative_and_lower_is_positive() {
        let b = boxed(pt(10), pt(5), 0, false);
        assert_eq!(raise(b.clone(), pt(3)).shift_amount, -pt(3));
        assert_eq!(lower(b.clone(), pt(3)).shift_amount, pt(3));
        assert_eq!(move_left(b.clone(), pt(3)).shift_amount, -pt(3));
        assert_eq!(move_right(b, pt(3)).shift_amount, pt(3));
    }

    /// §653: a raised box in an hbox raises the box's HEIGHT too, because
    /// `hpack` takes `height(p)-s` and `depth(p)+s`.
    #[test]
    fn a_raised_box_makes_the_line_taller() {
        let inner = raise(boxed(pt(10), pt(5), pt(1), false), pt(3));
        let packed = hpack(vec![Node::Box(inner)], NATURAL, Tolerances::plain(), None);
        assert_eq!(packed.node.height, pt(8));
        // The depth would be 1pt - 3pt = -2pt, but §650 starts `d:=0` and only
        // ever takes a maximum, so a box lifted clear of the baseline leaves
        // the line with no depth rather than a negative one.
        assert_eq!(packed.node.depth, 0);
        // Lowering the same box does deepen the line.
        let inner = lower(boxed(pt(10), pt(5), pt(1), false), pt(3));
        let packed = hpack(vec![Node::Box(inner)], NATURAL, Tolerances::plain(), None);
        assert_eq!(packed.node.depth, pt(4));
        assert_eq!(packed.node.height, pt(2));
    }

    /// §463: an `\hrule` is 0.4pt tall with a running width; a `\vrule` is
    /// 0.4pt wide with running height and depth. "Running" is `null_flag`, not
    /// zero -- a rule of zero width draws nothing.
    #[test]
    fn the_rules_have_tex_s_own_defaults() {
        let h = hrule();
        assert_eq!(h.height, DEFAULT_RULE);
        assert_eq!(h.depth, 0);
        assert!(RuleNode::is_running(h.width));
        // 0.4pt, to the scaled point.
        assert_eq!(crate::dimen::print_scaled(DEFAULT_RULE), "0.4");
        let v = vrule();
        assert_eq!(v.width, DEFAULT_RULE);
        assert!(RuleNode::is_running(v.height));
        assert!(RuleNode::is_running(v.depth));
    }

    /// §1056: after a rule in vertical mode, `\prevdepth` is `ignore_depth`,
    /// so the box below the rule gets no interline glue and sits against it.
    #[test]
    fn a_rule_stops_the_next_baselineskip() {
        let mut b = ListBuilder::new(Mode::Vertical);
        b.box_(boxed(pt(100), pt(8), pt(2), false));
        assert_eq!(b.prev_depth, pt(2));
        b.rule(hrule());
        assert_eq!(b.prev_depth, IGNORE_DEPTH);
        b.box_(boxed(pt(100), pt(8), pt(2), false));
        // box, rule, box -- no glue node anywhere.
        assert_eq!(b.list.len(), 3);
        assert!(!b.list.iter().any(|n| matches!(n, Node::Glue(_))));
    }

    /// §679 through the builder: two boxes with no rule between them DO get
    /// interline glue, so the test above is about the rule rather than about
    /// the builder never inserting any.
    #[test]
    fn two_boxes_in_a_row_do_get_interline_glue() {
        let mut b = ListBuilder::new(Mode::Vertical);
        b.box_(boxed(pt(100), pt(8), pt(2), false));
        b.box_(boxed(pt(100), pt(8), pt(2), false));
        assert_eq!(b.list.len(), 3);
        let Node::Glue(g) = &b.list[1] else {
            panic!("interline glue");
        };
        assert_eq!(g.source, GlueSource::BaselineSkip);
        assert_eq!(g.spec.natural, pt(2));
    }

    /// §1110: `\unhbox` empties the register and `\unhcopy` does not, and what
    /// lands on the list is the CONTENTS rather than the box.
    #[test]
    fn unboxing_spills_the_contents_and_copying_keeps_the_register() {
        let mut regs = Boxes::default();
        let inner = hpack(
            vec![
                Node::Glue(GlueNode::new(Glue::fixed(pt(3)))),
                Node::Box(boxed(pt(10), 0, 0, false)),
            ],
            NATURAL,
            Tolerances::plain(),
            None,
        )
        .node;
        regs.set_box(0, Some(inner));

        let mut b = ListBuilder::new(Mode::Horizontal);
        b.unbox(&mut regs, 0, true).expect("an hbox in hmode");
        assert_eq!(b.list.len(), 2);
        assert!(regs.box_register(0).is_some());

        b.unbox(&mut regs, 0, false).expect("an hbox in hmode");
        assert_eq!(b.list.len(), 4);
        assert!(regs.box_register(0).is_none());

        // And a void register unboxes to nothing, without complaint.
        b.unbox(&mut regs, 7, false).expect("a void register");
        assert_eq!(b.list.len(), 4);
    }

    /// §1110: "I refuse to unbox an \hbox in vertical mode or vice versa."
    #[test]
    fn unboxing_the_wrong_kind_of_box_is_refused() {
        let mut regs = Boxes::default();
        regs.set_box(0, Some(boxed(pt(10), 0, 0, false)));
        let mut v = ListBuilder::new(Mode::Vertical);
        assert_eq!(
            v.unbox(&mut regs, 0, false),
            Err(BoxError::IncompatibleList)
        );
        // Refused means untouched: the register still holds the box.
        assert!(regs.box_register(0).is_some());
        assert!(v.list.is_empty());
        assert_eq!(
            BoxError::IncompatibleList.to_string(),
            "Incompatible list can't be unboxed"
        );
    }

    /// §1247: setting `\wd` changes what the box claims, not what it holds.
    /// `\rlap` is exactly this -- a box of zero width that still draws.
    #[test]
    fn setting_a_box_dimension_does_not_move_what_is_in_it() {
        let mut regs = Boxes::default();
        let inner = hpack(
            vec![Node::Box(boxed(pt(10), pt(4), pt(1), false))],
            NATURAL,
            Tolerances::plain(),
            None,
        )
        .node;
        regs.set_box(0, Some(inner));
        assert_eq!(box_dimen(&regs, 0, Dimension::Width), pt(10));
        set_box_dimen(&mut regs, 0, Dimension::Width, 0);
        assert_eq!(box_dimen(&regs, 0, Dimension::Width), 0);
        // The contents are still there, at their own width.
        let b = regs.box_register(0).expect("still a box");
        assert_eq!(b.list.len(), 1);
        assert_eq!(b.height, pt(4));
        // A void register reads zero and stays void.
        assert_eq!(box_dimen(&regs, 9, Dimension::Height), 0);
        set_box_dimen(&mut regs, 9, Dimension::Height, pt(5));
        assert!(regs.box_register(9).is_none());
    }

    /// §1058: `\hss` can SHRINK where `\hfil` cannot, which is the difference
    /// between centring that can overflow gracefully and centring that cannot.
    #[test]
    fn hss_shrinks_where_hfil_only_stretches() {
        let line = |spec: Glue| {
            vec![
                Node::Glue(GlueNode::new(spec)),
                Node::Box(boxed(pt(50), 0, 0, false)),
                Node::Glue(GlueNode::new(spec)),
            ]
        };
        // Squeezed into 40pt: \hfil has no shrink, so this is overfull.
        let packed = hpack(
            line(fill::fil()),
            Spec::Exactly(pt(40)),
            Tolerances::plain(),
            None,
        );
        assert_eq!(packed.report, Some(crate::pack::Report::Overfull(pt(10))));
        // \hss has infinite shrink, so the same line sets without complaint.
        let packed = hpack(
            line(fill::ss()),
            Spec::Exactly(pt(40)),
            Tolerances::plain(),
            None,
        );
        assert_eq!(packed.report, None);
        let widths = crate::pack::glue_widths(&packed.node);
        // Each side pulls back 5pt: the box is centred and hangs out both ends.
        assert_eq!(widths, vec![-pt(5), -pt(5)]);
    }

    /// `\hfil` on each side centres, and the two halves are exactly equal --
    /// which they are only because §625's rounding accumulates.
    #[test]
    fn hfil_on_both_sides_centres_a_box() {
        let list = vec![
            Node::Glue(GlueNode::new(fill::fil())),
            Node::Box(boxed(pt(50), 0, 0, false)),
            Node::Glue(GlueNode::new(fill::fil())),
        ];
        let packed = hpack(list, Spec::Exactly(pt(101)), Tolerances::plain(), None);
        let widths = crate::pack::glue_widths(&packed.node);
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0] + widths[1], pt(51));
        // 51pt of slack over two fils: the first gets 25.5pt, and the second
        // takes the odd scaled point so the sum is exact.
        assert_eq!(widths[0], pt(51) / 2);
    }

    /// §1058: `\hfilneg` cancels one `\hfil` exactly, which is how a document
    /// undoes a fill it inherited.
    #[test]
    fn hfilneg_cancels_an_hfil() {
        let list = vec![
            Node::Glue(GlueNode::new(fill::fil())),
            Node::Glue(GlueNode::new(fill::fil_neg())),
            Node::Box(boxed(pt(10), 0, 0, false)),
        ];
        let packed = hpack(list, Spec::Exactly(pt(30)), Tolerances::plain(), None);
        // The two fils cancel, so there is nothing left to stretch and the
        // order falls back to normal with no stretch at all.
        assert_eq!(packed.node.glue_sign, crate::node::GlueSign::Normal);
        assert_eq!(packed.node.width, pt(30));
    }

    /// §1085-§1086 through `package`: the three constructions differ only in
    /// where the reference point ends up.
    #[test]
    fn hbox_vbox_and_vtop_put_the_reference_point_in_three_places() {
        let list = || {
            vec![
                Node::Box(boxed(pt(30), pt(7), pt(2), false)),
                Node::Box(boxed(pt(40), pt(5), pt(3), false)),
            ]
        };
        let h = package(
            Kind::H,
            list(),
            NATURAL,
            crate::node::MAX_DIMEN,
            Tolerances::plain(),
        );
        // Side by side: widths add, heights and depths take the max.
        assert_eq!(h.node.width, pt(70));
        assert_eq!(h.node.height, pt(7));
        assert_eq!(h.node.depth, pt(3));

        let v = package(
            Kind::V,
            list(),
            NATURAL,
            crate::node::MAX_DIMEN,
            Tolerances::plain(),
        );
        // Stacked: 7 + 2 + 5 of height, and the last depth stays depth.
        assert_eq!(v.node.width, pt(40));
        assert_eq!(v.node.height, pt(14));
        assert_eq!(v.node.depth, pt(3));

        let t = package(
            Kind::Top,
            list(),
            NATURAL,
            crate::node::MAX_DIMEN,
            Tolerances::plain(),
        );
        // The same stack, hung from its first baseline instead.
        assert_eq!(t.node.height, pt(7));
        assert_eq!(t.node.depth, pt(10));
        assert_eq!(
            t.node.height + t.node.depth,
            v.node.height + v.node.depth,
            "a vtop is the same material, measured from a different point"
        );
    }

    /// §977 against a register: the remainder replaces what was there, so a
    /// second `\vsplit` continues where the first stopped.
    #[test]
    fn vsplit_leaves_the_remainder_in_the_register() {
        let mut regs = Boxes::default();
        let mut list = Vec::new();
        for i in 0..6 {
            if i > 0 {
                list.push(Node::Glue(GlueNode::new(Glue {
                    natural: pt(2),
                    stretch: pt(1),
                    shrink: pt(1),
                    ..Glue::default()
                })));
            }
            list.push(Node::Box(boxed(pt(100), pt(10), 0, false)));
        }
        regs.set_box(1, Some(vbox(list, Tolerances::plain())));
        let split = vsplit_register(
            &mut regs,
            1,
            pt(34),
            pt(4),
            Glue::fixed(pt(10)),
            Tolerances::plain(),
        )
        .expect("a vbox splits");
        assert_eq!(split.extracted.height, pt(34));
        let rest = regs.box_register(1).expect("the remainder stayed behind");
        assert!(rest.vertical);
        // Three lines were taken; three are left, under a fresh \splittopskip.
        assert!(rest.height > pt(20));
    }

    /// §1081: `\lastbox` takes the box off the list and clears its shift,
    /// because the raise was where the box SAT and not what it was.
    #[test]
    fn lastbox_takes_the_box_and_forgets_where_it_sat() {
        let mut b = ListBuilder::new(Mode::Horizontal);
        b.box_(boxed(pt(10), pt(3), 0, false));
        b.box_(raise(boxed(pt(20), pt(3), 0, false), pt(5)));
        let taken = b.last_box().expect("the list ends with a box");
        assert_eq!(taken.width, pt(20));
        assert_eq!(taken.shift_amount, 0, "the raise did not come with it");
        assert_eq!(b.list.len(), 1);
        // What is left ends with the first box, so a second \lastbox gets it.
        assert_eq!(b.last_box().map(|b| b.width), Some(pt(10)));
        // And a third finds nothing at all rather than erroring.
        assert!(b.last_box().is_none());
    }

    /// §1081: a `\lastbox` where the last node is not a box is void, and the
    /// list is left exactly as it was.
    #[test]
    fn lastbox_is_void_when_the_list_does_not_end_with_one() {
        let mut b = ListBuilder::new(Mode::Horizontal);
        b.box_(boxed(pt(10), pt(3), 0, false));
        b.glue(Glue::fixed(pt(4)));
        assert!(b.last_box().is_none());
        assert_eq!(b.list.len(), 2, "the glue is still there");
        b.rule(vrule());
        assert!(b.last_box().is_none(), "a rule is not a box");
    }

    /// §1081, §1105: a node a discretionary owns as one of its `replace_count`
    /// successors is not the list's to remove — taking it would leave the disc
    /// pointing past the end of the list.
    #[test]
    fn a_discretionarys_replacement_is_not_the_lists_to_take() {
        let mut b = ListBuilder::new(Mode::Horizontal);
        b.list.push(Node::Disc(crate::node::DiscNode {
            pre_break: Vec::new(),
            post_break: Vec::new(),
            replace_count: 1,
        }));
        b.box_(boxed(pt(10), pt(3), 0, false));
        assert!(b.last_box().is_none());
        assert_eq!(b.list.len(), 2);
    }

    /// §1105: `\unskip` removes the last glue and nothing else; run against a
    /// list that ends in a penalty it is a no-op, which is what makes it safe
    /// at the end of a macro.
    #[test]
    fn unskip_unkern_and_unpenalty_each_take_only_their_own_kind() {
        let mut b = ListBuilder::new(Mode::Horizontal);
        b.box_(boxed(pt(10), pt(3), 0, false));
        b.glue(Glue::fixed(pt(4)));
        b.kern(pt(2));
        b.penalty(150);

        assert!(
            b.remove_last(Removable::Glue).is_none(),
            "a penalty is not glue"
        );
        assert!(matches!(
            b.remove_last(Removable::Penalty),
            Some(Node::Penalty(150))
        ));
        assert!(matches!(
            b.remove_last(Removable::Kern),
            Some(Node::Kern { width, .. }) if width == pt(2)
        ));
        assert!(matches!(
            b.remove_last(Removable::Glue),
            Some(Node::Glue(_))
        ));
        // Only the box is left, and no \unskip can reach it.
        assert!(b.remove_last(Removable::Glue).is_none());
        assert_eq!(b.list.len(), 1);
    }
}
