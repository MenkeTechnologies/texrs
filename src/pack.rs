//! Packaging: `hpack`, `vpack`, badness, and the glue setting that makes a box
//! come out at the width it was asked for.
//!
//! `tex.web` §644-§679. This is the piece the engine did not have: a line
//! whose glue has been SET is a line whose spaces each got a share of the
//! slack, and the share is not a division — TeX distributes by ORDER of
//! infinity (§658), so `\hfil` in a line takes all of it and the interword
//! spaces take none, and it rounds by accumulating the exact real total and
//! taking the difference (§625), so the widths sum to the box width instead of
//! drifting by a scaled point per space.
//!
//! Three things here are worth naming because getting them wrong is invisible:
//!
//! - **Badness is not $100(t/s)^3$.** §108 says every implementation of TeX
//!   should use *precisely* its integer method, and it caps at 8189 before
//!   jumping to `inf_bad`. A float version agrees to within a unit almost
//!   everywhere and then disagrees at the fitness-class boundaries, which is
//!   where badness is actually read.
//! - **Only the highest order present is stretched** (§657, §659). A list with
//!   `3pt` and `1fil` of stretch has `1fil` of stretch, and the `3pt` gets
//!   nothing at all.
//! - **`vpack` charges depth to the height** (§668-§670): each box on a
//!   vertical list contributes `prev_depth + height`, and the depth of the
//!   whole is the depth of the last box, clipped to `\boxmaxdepth`.

use crate::glue::{Glue, Order};
use crate::node::{
    BoxNode, GlueNode, GlueSign, GlueSource, Node, RuleNode, Scaled, IGNORE_DEPTH, INF_BAD,
    MAX_DIMEN,
};

/// §108: how bad it is to make a total of `t` out of glue that has `s` to
/// give.
///
/// Ported, not approximated. "It produces an integer value that is a
/// reasonably close approximation to $100(t/s)^3$, and all implementations of
/// TeX should use precisely this method." $297^3 = 99.94\times2^{18}$, and
/// $1290^3<2^{31}<1291^3$ is why the cap sits at 1290.
pub fn badness(t: Scaled, s: Scaled) -> i64 {
    if t == 0 {
        return 0;
    }
    if s <= 0 {
        return INF_BAD;
    }
    let r = if t <= 7_230_584 {
        (t * 297) / s
    } else if s >= 1_663_497 {
        t / (s / 297)
    } else {
        t
    };
    if r > 1290 {
        return INF_BAD;
    }
    // r^3/2^18, rounded to the nearest integer.
    (r * r * r + 0o400_000) / 0o1_000_000
}

/// §106 `x_over_n`: a scaled dimension divided by an integer, truncating
/// towards zero on both signs.
pub fn x_over_n(x: Scaled, n: i64) -> Scaled {
    if n == 0 {
        return 0;
    }
    let (x, n) = match n < 0 {
        true => (-x, -n),
        false => (x, n),
    };
    match x >= 0 {
        true => x / n,
        false => -((-x) / n),
    }
}

/// What dimension a box is being packaged to (§644).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spec {
    /// `\hbox to 300pt`: the box comes out exactly this wide.
    Exactly(Scaled),
    /// `\hbox spread 10pt`: this much MORE than the natural size.
    Additional(Scaled),
}

/// `hpack(p,natural)` — a box at its natural size (§644: "`natural` is
/// shorthand for `0,additional`").
pub const NATURAL: Spec = Spec::Additional(0);

/// The parameters `hpack` and `vpack` read when deciding whether to complain
/// (§660, §663, §666, §674, §677).
///
/// They are separate from the packaging itself because the glue setting does
/// not depend on them at all: a box is set the same way whether or not TeX
/// says anything about it.
///
/// The default is INITEX's: every integer parameter starts at zero (§240), so
/// a bare `tex` complains about any box that is not perfect.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tolerances {
    pub hbadness: i64,
    pub vbadness: i64,
    pub hfuzz: Scaled,
    pub vfuzz: Scaled,
}

impl Tolerances {
    /// What `plain.tex` sets: `\hbadness=1000`, `\vbadness=1000`,
    /// `\hfuzz=0.1pt`, `\vfuzz=0.1pt`.
    pub fn plain() -> Tolerances {
        Tolerances {
            hbadness: 1000,
            vbadness: 1000,
            hfuzz: 6554,
            vfuzz: 6554,
        }
    }
}

/// What TeX would have said about a box it has just packaged (§660-§667,
/// §674-§678).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Report {
    /// Stretched past `\hbadness`/`\vbadness`, badness above 100.
    Underfull(i64),
    /// The same, badness at or below 100.
    Loose(i64),
    /// Shrunk past the tolerance, but the shrink was available.
    Tight(i64),
    /// More material than the shrink can absorb: by this much.
    Overfull(Scaled),
}

impl Report {
    /// The line `tex` prints on the terminal for this, without the location.
    pub fn message(&self, vertical: bool) -> String {
        let (b, dim) = match vertical {
            true => ("\\vbox", "pt too high"),
            false => ("\\hbox", "pt too wide"),
        };
        match self {
            Report::Underfull(n) => format!("Underfull {b} (badness {n})"),
            Report::Loose(n) => format!("Loose {b} (badness {n})"),
            Report::Tight(n) => format!("Tight {b} (badness {n})"),
            Report::Overfull(by) => {
                format!("Overfull {b} ({}{dim})", crate::dimen::print_scaled(*by))
            }
        }
    }
}

/// A packaged box, with the two things `tex.web` leaves in globals.
#[derive(Clone, Debug)]
pub struct Packed {
    pub node: BoxNode,
    /// `last_badness` (§646), which is what `\badness` reads.
    pub badness: i64,
    /// What would have been reported, if anything.
    pub report: Option<Report>,
}

/// The four orders' worth of stretch or shrink found in a list (§646).
#[derive(Clone, Copy, Debug, Default)]
struct Totals([Scaled; 4]);

impl Totals {
    fn add(&mut self, amount: Scaled, order: Order) {
        let order = order.clamp(0, 3) as usize;
        self.0[order] += amount;
    }

    /// §657: "The highest order of infinity that has a nonzero coefficient is
    /// then used as if no other orders were present."
    fn order(&self) -> Order {
        (0..4).rev().find(|&o| self.0[o] != 0).unwrap_or(0) as Order
    }

    fn at(&self, order: Order) -> Scaled {
        self.0[order.clamp(0, 3) as usize]
    }
}

/// `hpack(p,w,m)` (§649-§667): wrap a horizontal list in a box.
///
/// `adjust` is `tex.web`'s `adjust_tail` (§647): when it is `Some`, the
/// insertions, marks and `\vadjust` material inside the list are MOVED out of
/// the box onto it, which is how a footnote written inside a paragraph reaches
/// the page rather than being sealed inside a line.
pub fn hpack(
    list: Vec<Node>,
    spec: Spec,
    tol: Tolerances,
    adjust: Option<&mut Vec<Node>>,
) -> Packed {
    let mut r = BoxNode {
        vertical: false,
        ..BoxNode::null()
    };
    // §650: h, d, x are the height, depth and natural width so far.
    let mut h: Scaled = 0;
    let mut d: Scaled = 0;
    let mut x: Scaled = 0;
    let mut stretch = Totals::default();
    let mut shrink = Totals::default();

    let mut kept: Vec<Node> = Vec::with_capacity(list.len());
    let mut adjust = adjust;
    for node in list {
        match &node {
            // §654: a character contributes its own three dimensions.
            Node::Char(c) | Node::Ligature(c) => {
                x += c.width;
                h = h.max(c.height);
                d = d.max(c.depth);
            }
            // §653: a box or rule, with the shift applied to height and depth.
            Node::Box(_) | Node::Rule(_) => {
                let (w, bh, bd) = box_dimensions(&node);
                x += w;
                let s = node.shift_amount();
                h = h.max(bh - s);
                d = d.max(bd + s);
            }
            // §655: moved to the adjustment list, if there is one.
            Node::Ins(_) | Node::Mark(_) | Node::Adjust(_) => {
                if let Some(tail) = adjust.as_deref_mut() {
                    transfer_to_adjustment(node, tail);
                    continue;
                }
            }
            // §656: glue adds its natural width and its two stretchabilities,
            // and leaders lend the box their own height and depth.
            Node::Glue(g) => {
                x += g.spec.natural;
                stretch.add(g.spec.stretch, g.spec.stretch_order);
                shrink.add(g.spec.shrink, g.spec.shrink_order);
                if let (true, Some(leader)) = (g.kind.is_leaders(), &g.leader) {
                    let (_, lh, ld) = box_dimensions(leader);
                    h = h.max(lh);
                    d = d.max(ld);
                }
            }
            // §651: a kern and a math node contribute width and nothing else.
            Node::Kern { width, .. } | Node::Math(width) => x += *width,
            // §651's `othercases do_nothing`.
            Node::Disc(_) | Node::Whatsit(_) | Node::Penalty(_) => {}
        }
        kept.push(node);
    }

    r.height = h;
    r.depth = d;
    let (badness, report) = set_glue(&mut r, x, spec, stretch, shrink, &kept, tol);
    r.list = kept;
    Packed {
        node: r,
        badness,
        report,
    }
}

/// `vpackage(p,h,m,l)` (§668-§678): wrap a vertical list in a box, with `l` as
/// the largest depth it may keep.
///
/// `vpack(p,h,m)` is this with `l = max_dimen` (§668).
pub fn vpackage(list: Vec<Node>, spec: Spec, max_depth: Scaled, tol: Tolerances) -> Packed {
    let mut r = BoxNode {
        vertical: true,
        ..BoxNode::null()
    };
    // §668: w, d, x are the width, depth and natural height so far.
    let mut w: Scaled = 0;
    let mut d: Scaled = 0;
    let mut x: Scaled = 0;
    let mut stretch = Totals::default();
    let mut shrink = Totals::default();

    for node in &list {
        match node {
            // §670: the running total picks up the PREVIOUS box's depth, and
            // this box's depth becomes the new pending one.
            Node::Box(_) | Node::Rule(_) => {
                let (bw, bh, bd) = box_dimensions(node);
                x += d + bh;
                d = bd;
                w = w.max(bw + node.shift_amount());
            }
            // §671.
            Node::Glue(g) => {
                x += d;
                d = 0;
                x += g.spec.natural;
                stretch.add(g.spec.stretch, g.spec.stretch_order);
                shrink.add(g.spec.shrink, g.spec.shrink_order);
                if let (true, Some(leader)) = (g.kind.is_leaders(), &g.leader) {
                    let (lw, _, _) = box_dimensions(leader);
                    w = w.max(lw);
                }
            }
            // §669: a kern closes off the pending depth just as glue does.
            Node::Kern { width, .. } => {
                x += d + width;
                d = 0;
            }
            _ => {}
        }
    }

    r.width = w;
    // §668: a depth past the limit is turned into height, which is what moves
    // the reference point down rather than letting the box hang.
    if d > max_depth {
        x += d - max_depth;
        r.depth = max_depth;
    } else {
        r.depth = d;
    }
    let (badness, report) = set_glue(&mut r, x, spec, stretch, shrink, &list, tol);
    r.list = list;
    Packed {
        node: r,
        badness,
        report,
    }
}

/// `vpack(p,h,m)` (§668): `vpackage` with no limit on the depth.
pub fn vpack(list: Vec<Node>, spec: Spec, tol: Tolerances) -> Packed {
    vpackage(list, spec, MAX_DIMEN, tol)
}

/// §1086: a `\vtop` is a `\vbox` whose height is the height of its FIRST item,
/// if that item is a box or a rule, and zero otherwise; the rest of the height
/// becomes depth.
pub fn vtop(mut packed: Packed) -> Packed {
    let h = match packed.node.list.first() {
        Some(first) if first.type_code() <= 2 => box_dimensions(first).1,
        _ => 0,
    };
    packed.node.depth = packed.node.depth - h + packed.node.height;
    packed.node.height = h;
    packed
}

/// The width, height and depth a box or rule contributes.
///
/// A running rule dimension is `null_flag`, and §653 relies on it being "a
/// highly negative number" that loses every `max` it takes part in. Passing it
/// through unchanged is the port; special-casing it would change what an
/// `\hrule` inside an `\hbox` does.
fn box_dimensions(node: &Node) -> (Scaled, Scaled, Scaled) {
    match node {
        Node::Box(b) => (b.width, b.height, b.depth),
        Node::Rule(r) => (r.width, r.height, r.depth),
        Node::Char(c) | Node::Ligature(c) => (c.width, c.height, c.depth),
        _ => (0, 0, 0),
    }
}

/// §655: an `\vadjust` contributes its CONTENTS to the adjustment list, while
/// an insertion or a mark moves across whole.
fn transfer_to_adjustment(node: Node, tail: &mut Vec<Node>) {
    match node {
        Node::Adjust(list) => tail.extend(list),
        other => tail.push(other),
    }
}

/// §657-§667 and §672-§678: turn the natural size into the asked-for size, and
/// record how the glue has to be set to get there.
///
/// One routine for both packers because `tex.web` writes the two as the same
/// sequence with `width`/`height` and `hbadness`/`vbadness` exchanged, and a
/// second copy is a second place for the order rule to be got wrong.
fn set_glue(
    r: &mut BoxNode,
    natural: Scaled,
    spec: Spec,
    stretch: Totals,
    shrink: Totals,
    list: &[Node],
    tol: Tolerances,
) -> (i64, Option<Report>) {
    let vertical = r.vertical;
    let target = match spec {
        Spec::Exactly(w) => w,
        Spec::Additional(w) => natural + w,
    };
    match vertical {
        true => r.height = target,
        false => r.width = target,
    }
    // §658: now x is the excess to be made up.
    let x = target - natural;
    let (badness_limit, fuzz) = match vertical {
        true => (tol.vbadness, tol.vfuzz),
        false => (tol.hbadness, tol.hfuzz),
    };

    if x == 0 {
        r.glue_sign = GlueSign::Normal;
        r.glue_order = 0;
        r.glue_set = 0.0;
        return (0, None);
    }

    if x > 0 {
        // §659: stretch.
        let o = stretch.order();
        r.glue_order = o;
        r.glue_sign = GlueSign::Stretching;
        if stretch.at(o) != 0 {
            r.glue_set = x as f64 / stretch.at(o) as f64;
        } else {
            r.glue_sign = GlueSign::Normal;
            r.glue_set = 0.0;
        }
        if o != 0 || list.is_empty() {
            return (0, None);
        }
        // §660, §674: an underfull or loose box.
        let b = badness(x, stretch.at(0));
        if b > badness_limit {
            let report = match b > 100 {
                true => Report::Underfull(b),
                false => Report::Loose(b),
            };
            return (b, Some(report));
        }
        return (b, None);
    }

    // §664, §678: shrink.
    let o = shrink.order();
    r.glue_order = o;
    r.glue_sign = GlueSign::Shrinking;
    if shrink.at(o) != 0 {
        r.glue_set = (-x) as f64 / shrink.at(o) as f64;
    } else {
        r.glue_sign = GlueSign::Normal;
        r.glue_set = 0.0;
    }
    if shrink.at(o) < -x && o == 0 && !list.is_empty() {
        // §666, §677: overfull. The glue is set to its maximum shrinkage and
        // the material sticks out.
        r.glue_set = 1.0;
        let over = -x - shrink.at(0);
        if over > fuzz || badness_limit < 100 {
            return (1_000_000, Some(Report::Overfull(over)));
        }
        return (1_000_000, None);
    }
    if o != 0 || list.is_empty() {
        return (0, None);
    }
    // §667, §678: a tight box.
    let b = badness(-x, shrink.at(0));
    if b > badness_limit {
        return (b, Some(Report::Tight(b)));
    }
    (b, None)
}

/// A billion, in `tex.web`'s `vet_glue` (§625): the clamp that keeps a glue
/// ratio that has gone wild from producing a nonsense dimension.
const BILLION: f64 = 1_000_000_000.0;

/// Walks a set box's list handing back the width each glue node is SET to
/// (§625 for an hlist, §634 for a vlist).
///
/// The rounding is the whole point, and it is not "round each glue's share".
/// TeX keeps `cur_glue`, the exact real total of the stretch seen so far, and
/// `cur_g`, that total times the glue ratio rounded ONCE; the width handed
/// back is the difference between successive `cur_g`s. So a box of forty
/// spaces sharing 10.5pt of stretch comes out exactly 10.5pt wider, where
/// forty independently rounded shares would be off by up to twenty scaled
/// points and leave the right margin ragged.
pub struct Setter {
    glue_set: f64,
    sign: GlueSign,
    order: Order,
    cur_glue: f64,
    cur_g: Scaled,
}

impl Setter {
    /// The setter for one box's own list. A nested box is set by its OWN
    /// setter: `glue_set` belongs to the box that holds the glue.
    pub fn new(b: &BoxNode) -> Setter {
        Setter {
            glue_set: b.glue_set,
            sign: b.glue_sign,
            order: b.glue_order,
            cur_glue: 0.0,
            cur_g: 0,
        }
    }

    /// The width this glue node is set to, in the order the nodes appear.
    ///
    /// Must be called for every glue node of the list in order: the rounding
    /// carries from one to the next.
    pub fn glue(&mut self, spec: &Glue) -> Scaled {
        let mut width = spec.natural - self.cur_g;
        match self.sign {
            GlueSign::Stretching if spec.stretch_order == self.order => {
                self.cur_glue += spec.stretch as f64;
                self.cur_g = self.rounded();
            }
            GlueSign::Shrinking if spec.shrink_order == self.order => {
                self.cur_glue -= spec.shrink as f64;
                self.cur_g = self.rounded();
            }
            _ => {}
        }
        width += self.cur_g;
        width
    }

    /// `vet_glue(float(glue_set(this_box))*cur_glue)` then `round` (§625).
    fn rounded(&self) -> Scaled {
        let temp = (self.glue_set * self.cur_glue).clamp(-BILLION, BILLION);
        temp.round() as Scaled
    }
}

/// The width every glue node of a set hlist comes out at, in order.
///
/// A convenience over [`Setter`] for the callers that want the whole answer at
/// once — a renderer walking the list already has the nodes in hand.
pub fn glue_widths(b: &BoxNode) -> Vec<Scaled> {
    let mut setter = Setter::new(b);
    b.list
        .iter()
        .filter_map(|n| match n {
            Node::Glue(g) => Some(setter.glue(&g.spec)),
            _ => None,
        })
        .collect()
}

/// Where the copies of a leader box go inside the space its glue was set to
/// (§626-§627 for an hlist, §635-§637 for a vlist).
///
/// Leaders are not a repeated character: they are a box replicated at
/// positions the three kinds compute differently, and the difference is the
/// whole point of having three.
///
/// - `\leaders` (aligned) puts the copies on a GRID fixed by the enclosing
///   box, not by the glue: the first copy sits at the smallest multiple of the
///   box's width at or after the start of the space. That is why the dots of a
///   table of contents line up down the page instead of each row starting its
///   dots wherever its text happened to end.
/// - `\cleaders` centres the copies in the space, putting half the leftover at
///   each end.
/// - `\xleaders` spreads the leftover BETWEEN the copies as well, `lr/(q+1)`
///   per gap, with half of the rounding error at each end.
///
/// `cur_h` is where the glue starts and `left_edge` is the reference point of
/// the box that holds it, both on the axis the leaders run along; for a
/// vertical list read `cur_v` and the top edge, since §637 is §627 with the
/// names exchanged. The returned positions are the LEADING edge of each copy.
///
/// The `+10` is `tex.web`'s own, and its comment is "compensate for
/// floating-point rounding": the space came from `glue_set`, a float, so a
/// copy that lands within a hundred-thousandth of a point of the far edge is
/// meant to be drawn. Dropping it drops the last dot of a leader run about
/// half the time.
///
/// A leader box wider than the space, or a space of nothing, yields no copies
/// at all — §626 draws blank glue in that case. A leader that is a RULE is not
/// replicated: §626 sends it to `fin_rule`, where it is drawn once at the full
/// width of the space, so a caller that meets `Node::Rule` there draws one
/// rule rather than asking this.
pub fn leader_positions(
    kind: crate::node::LeaderKind,
    leader_width: Scaled,
    space: Scaled,
    cur_h: Scaled,
    left_edge: Scaled,
) -> Vec<Scaled> {
    use crate::node::LeaderKind;
    if leader_width <= 0 || space <= 0 {
        return Vec::new();
    }
    let rule_wd = space + 10;
    let edge = cur_h + rule_wd;
    let mut lx: Scaled = 0;
    // §627: where the first copy goes.
    let mut h = match kind {
        LeaderKind::Aligned => {
            // `left_edge + leader_wd*((cur_h-left_edge) div leader_wd)`, then
            // one more if that landed before where we are. Pascal's `div`
            // truncates towards zero and so does Rust's `/`, which is what
            // makes the correction below necessary rather than optional.
            let aligned = left_edge + leader_width * ((cur_h - left_edge) / leader_width);
            match aligned < cur_h {
                true => aligned + leader_width,
                false => aligned,
            }
        }
        LeaderKind::Centred => {
            let lr = rule_wd % leader_width;
            cur_h + lr / 2
        }
        LeaderKind::Expanded => {
            let lq = rule_wd / leader_width;
            let lr = rule_wd % leader_width;
            lx = lr / (lq + 1);
            cur_h + (lr - (lq - 1) * lx) / 2
        }
        // Not leaders at all: ordinary glue draws nothing.
        LeaderKind::Normal => return Vec::new(),
    };
    let mut out = Vec::new();
    while h + leader_width <= edge {
        out.push(h);
        h += leader_width + lx;
    }
    out
}

/// The interline glue parameters `append_to_vlist` reads (§679).
#[derive(Clone, Copy, Debug, Default)]
pub struct Baselines {
    /// `\baselineskip`: the distance between successive baselines.
    pub baseline_skip: Glue,
    /// `\lineskip`: what is used instead when that would be too tight.
    pub line_skip: Glue,
    /// `\lineskiplimit`: too tight means less than this.
    pub line_skip_limit: Scaled,
}

impl Baselines {
    /// Plain TeX's own (`plain.tex`): `\baselineskip=12pt`, `\lineskip=1pt`,
    /// `\lineskiplimit=0pt`.
    pub fn plain() -> Baselines {
        Baselines {
            baseline_skip: Glue::fixed(12 * crate::dimen::UNITY),
            line_skip: Glue::fixed(crate::dimen::UNITY),
            line_skip_limit: 0,
        }
    }
}

/// `append_to_vlist(b)` (§679): put a box on a vertical list with the right
/// glue above it.
///
/// This is where `\baselineskip` becomes a distance rather than a setting. The
/// glue inserted is not `\baselineskip`: it is `\baselineskip` MINUS the depth
/// of what is already there and the height of what is arriving, so that the
/// baselines end up that far apart whatever the two boxes measure. When that
/// subtraction leaves less than `\lineskiplimit` — two tall lines that would
/// otherwise touch — `\lineskip` is used whole instead, which is why a line of
/// tall parentheses pushes the next line down rather than colliding with it.
///
/// `prev_depth` is `tex.web`'s, and it is carried by the CALLER because the
/// value survives across every box appended to the same list; the first box
/// gets no glue at all, which is what `ignore_depth` marks.
pub fn append_to_vlist(list: &mut Vec<Node>, b: BoxNode, prev_depth: &mut Scaled, p: Baselines) {
    if *prev_depth > IGNORE_DEPTH {
        let d = p.baseline_skip.natural - *prev_depth - b.height;
        let glue = match d < p.line_skip_limit {
            true => GlueNode::param(p.line_skip, GlueSource::LineSkip),
            false => GlueNode::param(
                Glue {
                    natural: d,
                    ..p.baseline_skip
                },
                GlueSource::BaselineSkip,
            ),
        };
        list.push(Node::Glue(glue));
    }
    *prev_depth = b.depth;
    list.push(Node::Box(b));
}

/// A rule with its running dimensions resolved against the box that holds it
/// (§626 for an hlist, §635 for a vlist).
///
/// A rule set in an hlist runs to the height and depth of that box; one set in
/// a vlist runs to its width. Nothing else about the rule changes.
pub fn resolve_rule(rule: RuleNode, container: &BoxNode) -> RuleNode {
    let mut out = rule;
    match container.vertical {
        // §635.
        true => {
            if RuleNode::is_running(out.width) {
                out.width = container.width;
            }
        }
        // §626.
        false => {
            if RuleNode::is_running(out.height) {
                out.height = container.height;
            }
            if RuleNode::is_running(out.depth) {
                out.depth = container.depth;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimen::UNITY;
    use crate::node::{CharNode, LeaderKind};

    fn pt(n: i64) -> Scaled {
        n * UNITY
    }

    fn ch(width: Scaled, height: Scaled, depth: Scaled) -> Node {
        Node::Char(CharNode {
            font: 0,
            character: 'x',
            width,
            height,
            depth,
        })
    }

    fn glue(natural: Scaled, stretch: Scaled, shrink: Scaled) -> Node {
        Node::Glue(GlueNode::new(Glue {
            natural,
            stretch,
            shrink,
            ..Glue::default()
        }))
    }

    fn fil(order: Order) -> Node {
        Node::Glue(GlueNode::new(Glue {
            stretch: UNITY,
            stretch_order: order,
            ..Glue::default()
        }))
    }

    /// §108's own numbers. `badness(t,s)` is $100(t/s)^3$ only approximately,
    /// and the approximation is the specification: `badness(1,1)` is 100 but
    /// `badness(2,1)` is 800 rather than 800.0 and `badness(5,1)` saturates.
    #[test]
    fn badness_is_the_integer_method_tex_specifies() {
        assert_eq!(badness(0, 0), 0);
        assert_eq!(badness(1, 0), INF_BAD);
        assert_eq!(badness(UNITY, UNITY), 100);
        assert_eq!(badness(2 * UNITY, UNITY), 800);
        // Not 2700: r=891, and (891^3+2^17) div 2^18 is 2698. The two-point
        // gap IS the specification -- a float $100(t/s)^3$ would say 2700 and
        // would then disagree with tex at a fitness boundary.
        assert_eq!(badness(3 * UNITY, UNITY), 2698);
        // 4.343^3*100 is 8189, the largest badness that is not infinite: §108
        // says "any badness of 2^13 or more is treated as infinitely bad".
        assert_eq!(badness(1290 * UNITY, 297 * UNITY), 8189);
        assert_eq!(badness(1291 * UNITY, 297 * UNITY), INF_BAD);
        // Monotone in both arguments, which §108 proves and which every
        // fitness decision downstream assumes.
        assert!(badness(UNITY + 1, UNITY) >= badness(UNITY, UNITY));
        assert!(badness(UNITY, UNITY) >= badness(UNITY, UNITY + 1));
    }

    /// §106: division truncates towards zero on both signs, which is not what
    /// Rust's `/` does for a negative numerator by accident -- it is what it
    /// does, and this pins it.
    #[test]
    fn x_over_n_truncates_towards_zero() {
        assert_eq!(x_over_n(7, 2), 3);
        assert_eq!(x_over_n(-7, 2), -3);
        assert_eq!(x_over_n(7, -2), -3);
        assert_eq!(x_over_n(7, 0), 0);
    }

    /// §649-§653: the natural size of an hbox is the sum of the widths and the
    /// max of the heights and depths.
    #[test]
    fn an_hbox_takes_its_natural_size_from_what_is_in_it() {
        let list = vec![
            ch(pt(10), pt(7), pt(2)),
            glue(pt(4), pt(2), pt(1)),
            ch(pt(6), pt(9), pt(1)),
        ];
        let packed = hpack(list, NATURAL, Tolerances::plain(), None);
        assert_eq!(packed.node.width, pt(20));
        assert_eq!(packed.node.height, pt(9));
        assert_eq!(packed.node.depth, pt(2));
        assert_eq!(packed.node.glue_sign, GlueSign::Normal);
        assert_eq!(packed.badness, 0);
    }

    /// §658-§659: `\hbox to` more than the natural width stretches, and the
    /// ratio is the excess over the stretch available.
    #[test]
    fn stretching_sets_the_glue_ratio_and_the_badness() {
        let list = vec![ch(pt(10), 0, 0), glue(pt(4), pt(2), pt(1)), ch(pt(6), 0, 0)];
        let packed = hpack(list, Spec::Exactly(pt(21)), Tolerances::plain(), None);
        assert_eq!(packed.node.width, pt(21));
        assert_eq!(packed.node.glue_sign, GlueSign::Stretching);
        assert_eq!(packed.node.glue_order, 0);
        // 1pt of excess over 2pt of stretch.
        assert!((packed.node.glue_set - 0.5).abs() < 1e-12);
        // badness(1pt, 2pt) = 100*(1/2)^3 = 12.
        assert_eq!(packed.badness, badness(pt(1), pt(2)));
        assert_eq!(packed.badness, 12);
        assert_eq!(packed.report, None);
    }

    /// §660: past `\hbadness` it is reported, and above badness 100 the word
    /// is "Underfull" rather than "Loose".
    #[test]
    fn an_underfull_box_is_reported_the_way_tex_reports_it() {
        let list = vec![ch(pt(10), 0, 0), glue(pt(4), pt(2), pt(1))];
        let packed = hpack(list, Spec::Exactly(pt(20)), Tolerances::plain(), None);
        // badness(6pt, 2pt): 100*3^3, which tex.web approximates as 2698.
        assert_eq!(packed.badness, 2698);
        assert_eq!(packed.report, Some(Report::Underfull(2698)));
        assert_eq!(
            packed.report.expect("reported").message(false),
            "Underfull \\hbox (badness 2698)"
        );
        // Under a tolerance that accepts it, the badness is the same number
        // and nothing is said: `\hbadness` decides the message, not the box.
        let list = vec![ch(pt(10), 0, 0), glue(pt(4), pt(2), pt(1))];
        let quiet = Tolerances {
            hbadness: 10000,
            ..Tolerances::plain()
        };
        let packed = hpack(list, Spec::Exactly(pt(20)), quiet, None);
        assert_eq!(packed.badness, 2698);
        assert_eq!(packed.report, None);
    }

    /// §666: more material than the shrink can take back is overfull, the glue
    /// is set to its maximum, and the report says by how much.
    #[test]
    fn an_overfull_box_says_how_far_past_the_measure_it_runs() {
        let list = vec![ch(pt(10), 0, 0), glue(pt(4), pt(2), pt(1)), ch(pt(6), 0, 0)];
        let packed = hpack(list, Spec::Exactly(pt(15)), Tolerances::plain(), None);
        assert_eq!(packed.badness, 1_000_000);
        // 5pt over, of which 1pt can be shrunk: 4pt too wide.
        assert_eq!(packed.report, Some(Report::Overfull(pt(4))));
        assert_eq!(
            packed.report.expect("reported").message(false),
            "Overfull \\hbox (4.0pt too wide)"
        );
        assert!((packed.node.glue_set - 1.0).abs() < 1e-12);
    }

    /// §657: "the highest order of infinity that has a nonzero coefficient is
    /// then used as if no other orders were present". This is Knuth's own
    /// worked example from §657, and it is the rule an `\hfil` in a line of
    /// ordinary spaces depends on.
    #[test]
    fn only_the_highest_order_of_infinity_stretches() {
        let spec = |stretch: Scaled, order: Order| {
            Node::Glue(GlueNode::new(Glue {
                stretch,
                stretch_order: order,
                ..Glue::default()
            }))
        };
        let list = vec![
            spec(pt(3), 0),
            spec(pt(8), 2),
            spec(pt(5), 1),
            spec(pt(6), 0),
            spec(-pt(3), 1),
            spec(-pt(8), 2),
        ];
        let packed = hpack(list, Spec::Exactly(pt(6)), Tolerances::plain(), None);
        // The fill components cancel, so the total is 2fil and the order is
        // fil, not fill and not normal.
        assert_eq!(packed.node.glue_order, 1);
        assert_eq!(packed.node.glue_sign, GlueSign::Stretching);
        assert!((packed.node.glue_set - 3.0).abs() < 1e-12);
        // And the 6pt is distributed 0, 0, 15pt, 0, -9pt, 0 -- §657's own
        // numbers, which is what the setter has to reproduce.
        let widths = glue_widths(&packed.node);
        assert_eq!(widths, vec![0, 0, pt(15), 0, -pt(9), 0]);
        // An infinite stretch is never bad: §659 reports only at order normal.
        assert_eq!(packed.badness, 0);
        assert_eq!(packed.report, None);
    }

    /// §625: the set widths sum to the box width EXACTLY, because the rounding
    /// accumulates rather than being applied per node. Forty spaces sharing a
    /// third of a point each is the case that shows it: rounding each share
    /// independently loses a scaled point on most of them.
    #[test]
    fn accumulated_rounding_makes_the_widths_sum_to_the_box() {
        let mut list: Vec<Node> = Vec::new();
        for _ in 0..40 {
            list.push(ch(pt(1), 0, 0));
            list.push(glue(pt(1), pt(1), pt(1)));
        }
        let target = pt(80) + 12345;
        let packed = hpack(list, Spec::Exactly(target), Tolerances::plain(), None);
        let widths = glue_widths(&packed.node);
        assert_eq!(widths.len(), 40);
        let set: Scaled = widths.iter().sum();
        assert_eq!(set + pt(40), target);
    }

    /// §668-§671: a vertical list charges each box's height plus the PREVIOUS
    /// box's depth, and the depth of the whole is the last box's.
    #[test]
    fn a_vbox_stacks_heights_and_keeps_the_last_depth() {
        let boxed = |h: Scaled, d: Scaled, w: Scaled| {
            Node::Box(BoxNode {
                width: w,
                height: h,
                depth: d,
                ..BoxNode::null()
            })
        };
        let list = vec![
            boxed(pt(7), pt(2), pt(30)),
            Node::Glue(GlueNode::new(Glue::fixed(pt(3)))),
            boxed(pt(5), pt(4), pt(50)),
        ];
        let packed = vpack(list, NATURAL, Tolerances::plain());
        // 7 + 2 + 3 + 5 = 17, and the trailing 4pt is depth, not height.
        assert_eq!(packed.node.height, pt(17));
        assert_eq!(packed.node.depth, pt(4));
        assert_eq!(packed.node.width, pt(50));
    }

    /// §668: a depth past `\boxmaxdepth` is turned into height. This is how a
    /// page box keeps its reference point where the driver expects it.
    #[test]
    fn depth_past_the_limit_becomes_height() {
        let list = vec![Node::Box(BoxNode {
            height: pt(10),
            depth: pt(6),
            ..BoxNode::null()
        })];
        let packed = vpackage(list, NATURAL, pt(2), Tolerances::plain());
        assert_eq!(packed.node.depth, pt(2));
        assert_eq!(packed.node.height, pt(14));
    }

    /// §1086: `\vtop`'s height is the first box's height and everything else
    /// is depth, which is what makes a `\vtop` align on its top line.
    #[test]
    fn a_vtop_takes_its_height_from_its_first_box() {
        let list = vec![
            Node::Box(BoxNode {
                height: pt(7),
                depth: pt(2),
                ..BoxNode::null()
            }),
            Node::Box(BoxNode {
                height: pt(5),
                depth: pt(3),
                ..BoxNode::null()
            }),
        ];
        let packed = vtop(vpack(list, NATURAL, Tolerances::plain()));
        assert_eq!(packed.node.height, pt(7));
        // The vbox was 7+2+5 = 14 high and 3 deep; the vtop is 7 high and the
        // remaining 7+3 = 10 deep.
        assert_eq!(packed.node.depth, pt(10));
    }

    /// §679: the glue above a box is `\baselineskip` less what is already
    /// there, so the BASELINES end up 12pt apart whatever the boxes measure.
    #[test]
    fn interline_glue_puts_the_baselines_where_baselineskip_says() {
        let mut list = Vec::new();
        let mut prev_depth = IGNORE_DEPTH;
        let p = Baselines::plain();
        let first = BoxNode {
            height: pt(7),
            depth: pt(2),
            ..BoxNode::null()
        };
        append_to_vlist(&mut list, first, &mut prev_depth, p);
        // The first box gets nothing above it.
        assert_eq!(list.len(), 1);
        assert_eq!(prev_depth, pt(2));
        let second = BoxNode {
            height: pt(4),
            depth: pt(1),
            ..BoxNode::null()
        };
        append_to_vlist(&mut list, second, &mut prev_depth, p);
        assert_eq!(list.len(), 3);
        let Node::Glue(g) = &list[1] else {
            panic!("interline glue is a glue node");
        };
        // 12pt - 2pt of depth - 4pt of height = 6pt.
        assert_eq!(g.spec.natural, pt(6));
        assert_eq!(g.source, GlueSource::BaselineSkip);
        // And the baselines really are 12pt apart: 2 + 6 + 4.
        assert_eq!(pt(2) + g.spec.natural + pt(4), pt(12));
    }

    /// §679: two tall boxes would collide, so `\lineskip` is used whole
    /// instead of a `\baselineskip` that has gone below `\lineskiplimit`.
    #[test]
    fn lineskip_takes_over_when_the_lines_would_touch() {
        let mut list = Vec::new();
        let mut prev_depth = pt(6);
        let p = Baselines::plain();
        let tall = BoxNode {
            height: pt(9),
            depth: pt(1),
            ..BoxNode::null()
        };
        append_to_vlist(&mut list, tall, &mut prev_depth, p);
        let Node::Glue(g) = &list[0] else {
            panic!("interline glue is a glue node");
        };
        // 12 - 6 - 9 = -3, which is below \lineskiplimit = 0.
        assert_eq!(g.spec.natural, pt(1));
        assert_eq!(g.source, GlueSource::LineSkip);
    }

    /// §655: a footnote written inside a paragraph has to leave the line box,
    /// or it can never reach the page. `hpack` moves it, and moves `\vadjust`
    /// material as its CONTENTS rather than as the adjust node.
    #[test]
    fn hpack_moves_insertions_and_adjustments_out_of_the_line() {
        let list = vec![
            ch(pt(10), 0, 0),
            Node::Ins(crate::node::InsNode {
                number: 1,
                height: pt(20),
                depth: 0,
                float_cost: 100,
                split_top_skip: Glue::default(),
                list: Vec::new(),
            }),
            Node::Mark("chapter".into()),
            Node::Adjust(vec![Node::Penalty(-50)]),
            ch(pt(6), 0, 0),
        ];
        let mut adjust = Vec::new();
        let packed = hpack(list, NATURAL, Tolerances::plain(), Some(&mut adjust));
        assert_eq!(packed.node.width, pt(16));
        assert_eq!(packed.node.list.len(), 2);
        assert_eq!(adjust.len(), 3);
        assert!(matches!(adjust[0], Node::Ins(_)));
        assert!(matches!(adjust[1], Node::Mark(_)));
        assert!(matches!(adjust[2], Node::Penalty(-50)));
        // Without an adjustment list they stay put -- §655 moves them only
        // when `adjust_tail` is non-null.
        let list = vec![ch(pt(10), 0, 0), Node::Mark("chapter".into())];
        let packed = hpack(list, NATURAL, Tolerances::plain(), None);
        assert_eq!(packed.node.list.len(), 2);
    }

    /// §626: an `\hrule` with no height set inside an hbox runs to that box's
    /// height and depth.
    #[test]
    fn a_running_rule_takes_the_dimensions_of_its_box() {
        let container = BoxNode {
            height: pt(8),
            depth: pt(3),
            width: pt(100),
            ..BoxNode::null()
        };
        let resolved = resolve_rule(RuleNode::running(), &container);
        assert_eq!(resolved.height, pt(8));
        assert_eq!(resolved.depth, pt(3));
        // Width is left running: an hlist does not decide it.
        assert!(RuleNode::is_running(resolved.width));
        let vertical = BoxNode {
            width: pt(100),
            vertical: true,
            ..BoxNode::null()
        };
        let resolved = resolve_rule(RuleNode::running(), &vertical);
        assert_eq!(resolved.width, pt(100));
        assert!(RuleNode::is_running(resolved.height));
    }

    /// §656: leaders lend their box's height and depth to the line, so a row
    /// of dot leaders in a table of contents does not squash the line it is in.
    #[test]
    fn leaders_lend_their_height_to_the_line() {
        let dots = BoxNode {
            width: pt(5),
            height: pt(6),
            depth: pt(1),
            ..BoxNode::null()
        };
        let list = vec![Node::Glue(GlueNode {
            spec: Glue {
                stretch: UNITY,
                stretch_order: 1,
                ..Glue::default()
            },
            kind: crate::node::LeaderKind::Aligned,
            leader: Some(Box::new(Node::Box(dots))),
            ..GlueNode::default()
        })];
        let packed = hpack(list, Spec::Exactly(pt(50)), Tolerances::plain(), None);
        assert_eq!(packed.node.height, pt(6));
        assert_eq!(packed.node.depth, pt(1));
    }

    /// §659: with no stretch at all in the list, the sign goes back to normal
    /// rather than dividing by zero, and the box is still the width asked for.
    #[test]
    fn a_box_with_nothing_to_stretch_is_still_the_width_asked_for() {
        let list = vec![ch(pt(10), 0, 0)];
        let packed = hpack(list, Spec::Exactly(pt(30)), Tolerances::plain(), None);
        assert_eq!(packed.node.width, pt(30));
        assert_eq!(packed.node.glue_sign, GlueSign::Normal);
        assert_eq!(packed.node.glue_set, 0.0);
    }

    /// §659: an empty box is never reported, however far from its natural size
    /// it is set -- `list_ptr(r)<>null` guards every report.
    #[test]
    fn an_empty_box_is_never_underfull() {
        let packed = hpack(Vec::new(), Spec::Exactly(pt(100)), Tolerances::default(), None);
        assert_eq!(packed.report, None);
        assert_eq!(packed.badness, 0);
        assert_eq!(packed.node.width, pt(100));
    }

    /// `\hfil` against `\hfill`: the higher order wins outright, which is what
    /// centring inside a `\hbox to \hsize` relies on.
    #[test]
    fn a_higher_infinity_takes_all_of_the_slack() {
        let list = vec![fil(1), ch(pt(10), 0, 0), fil(2)];
        let packed = hpack(list, Spec::Exactly(pt(50)), Tolerances::plain(), None);
        assert_eq!(packed.node.glue_order, 2);
        let widths = glue_widths(&packed.node);
        assert_eq!(widths, vec![0, pt(40)]);
    }

    /// §627: `\leaders` puts its copies on a grid fixed by the ENCLOSING BOX,
    /// so the same box drawn at two different starting points puts its dots in
    /// the same places. That is what lines the dot leaders of a table of
    /// contents up down the page instead of letting each row start its dots
    /// wherever its title happened to end.
    #[test]
    fn aligned_leaders_sit_on_the_boxs_grid_and_not_on_the_glues() {
        let dot = pt(10);
        // 40pt of space starting 25pt in: the first dot skips forward to 30pt,
        // the next multiple of its own width, and three fit before 65pt.
        let from_25 = leader_positions(LeaderKind::Aligned, dot, pt(40), pt(25), 0);
        assert_eq!(from_25, vec![pt(30), pt(40), pt(50)]);
        // A row whose text ends 2pt further along gets the SAME dots.
        let from_27 = leader_positions(LeaderKind::Aligned, dot, pt(40), pt(27), 0);
        assert_eq!(from_27, vec![pt(30), pt(40), pt(50)]);
        // And the grid is measured from the box, so shifting the box shifts
        // every dot with it.
        let shifted = leader_positions(LeaderKind::Aligned, dot, pt(40), pt(25), pt(3));
        assert_eq!(shifted, vec![pt(33), pt(43), pt(53)]);
    }

    /// §627: `\cleaders` centres the copies, leaving the same slack at each
    /// end; `\xleaders` puts the slack BETWEEN them as well, so its copies are
    /// further apart and there are the same number of them.
    #[test]
    fn cleaders_centre_the_copies_and_xleaders_spread_them() {
        let dot = pt(10);
        let space = pt(25);
        // `rule_wd` is 25pt plus §626's 10sp of slack, so two 10pt copies fit
        // with 5pt and 10sp left over. Centred puts half of that at each end.
        let centred = leader_positions(LeaderKind::Centred, dot, space, 0, 0);
        assert_eq!(centred, vec![163_845, 819_205]);
        let edge = space + 10;
        assert_eq!(
            centred[0],
            edge - (centred[1] + dot),
            "the same slack before the first copy and after the last"
        );
        assert_eq!(centred[1] - centred[0], dot, "no gap between the copies");

        // Expanded spreads the leftover between them too: lx = lr div (lq+1)
        // goes into every gap, and half the rounding error at each end.
        let expanded = leader_positions(LeaderKind::Expanded, dot, space, 0, 0);
        assert_eq!(expanded, vec![109_230, 873_820]);
        assert_eq!(expanded.len(), centred.len(), "the same number of copies");
        assert_eq!(
            expanded[0],
            edge - (expanded[1] + dot),
            "still symmetric about the space"
        );
        assert!(
            expanded[1] - expanded[0] > centred[1] - centred[0],
            "the copies are further apart than \\cleaders puts them"
        );
    }

    /// §626's `rule_wd:=rule_wd+10` — "compensate for floating-point
    /// rounding". The space a glue is set to came out of `glue_set`, a float,
    /// so a copy that lands a hundred-thousandth of a point short of the far
    /// edge is one TeX draws. Without the slack this run loses its last dot.
    #[test]
    fn the_ten_scaled_points_of_slack_keep_the_last_copy() {
        let dot = pt(10);
        // Three dots' worth of space, five scaled points short.
        let space = 3 * dot - 5;
        let with_slack = leader_positions(LeaderKind::Aligned, dot, space, 0, 0);
        assert_eq!(with_slack.len(), 3);
        // The same arithmetic without the compensation stops at two, which is
        // the dropped dot the +10 exists to prevent.
        let strict = (0..)
            .map(|i| i * dot)
            .take_while(|h| h + dot <= space)
            .count();
        assert_eq!(strict, 2);
    }

    /// §626: a leader box wider than the space it is given draws nothing at
    /// all, and neither does a space of nothing. The glue is blank instead.
    #[test]
    fn leaders_too_wide_for_their_space_draw_nothing() {
        assert!(leader_positions(LeaderKind::Aligned, pt(20), pt(15), 0, 0).is_empty());
        assert!(leader_positions(LeaderKind::Centred, pt(20), pt(15), 0, 0).is_empty());
        assert!(leader_positions(LeaderKind::Expanded, pt(20), pt(15), 0, 0).is_empty());
        assert!(leader_positions(LeaderKind::Aligned, pt(10), 0, 0, 0).is_empty());
        // Ordinary glue is not leaders and has nothing to replicate.
        assert!(leader_positions(LeaderKind::Normal, pt(10), pt(100), 0, 0).is_empty());
    }
}
