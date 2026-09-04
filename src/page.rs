//! Breaking vertical lists into pages: `\vsplit`, the page builder, insertions,
//! marks and the output routine.
//!
//! `tex.web` §967-§1028. Everything a LaTeX page is made of rides on this. A
//! footnote is an `\insert`; a running head is a `\mark` read back as
//! `\topmark`; a float is material the output routine holds over; the page
//! itself is `\box255`, packaged to `\pagegoal` and handed to `\output`. An
//! engine that stacks lines until the paper runs out has none of them, and
//! cannot get them by adding a special case, because they all come out of the
//! same decision: WHERE the vertical list is cut.
//!
//! The decision is a cost, not a fit (§1005). A page is priced by how badly
//! its glue has to stretch to reach `\pagegoal` (§1007), plus the penalty at
//! the break, plus the penalties of any insertions held over; a penalty of
//! $-10000$ or worse forces the break outright, and a page that cannot be
//! shrunk to fit is `awful_bad` and ends the search. So `\penalty-10000`
//! (`\break`), `\penalty10000` (`\nobreak`), `\widowpenalty` and
//! `\clubpenalty` are not hints here — they are terms in the sum the builder
//! minimises.
//!
//! The one structural departure from `tex.web` is that a list is a `Vec` and a
//! "pointer" is an index into it. That changes nothing about the algorithm and
//! removes the only part of §1013-§1022 that is about memory rather than
//! typesetting.

use crate::glue::Glue;
use crate::node::{
    BoxNode, GlueNode, GlueSource, InsNode, Node, Scaled, AWFUL_BAD, DEPLORABLE, EJECT_PENALTY,
    INF_BAD, INF_PENALTY, MAX_DIMEN,
};
use crate::pack::{badness, vpack, vpackage, x_over_n, Spec, Tolerances, NATURAL};
use std::collections::{BTreeMap, VecDeque};

/// The six running totals `vert_break` and the page builder keep (§970, §982).
///
/// `tex.web` calls them `active_height[1..6]` and `page_so_far[1..6]`: the
/// natural height, the four orders of stretch, and the shrink. Named here
/// because `page_so_far[3]` is `\pagefilstretch` and a document can read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// `page_total` / `cur_height`.
    pub height: Scaled,
    /// Finite stretch, then `fil`, `fill`, `filll`.
    pub stretch: [Scaled; 4],
    /// `page_shrink`.
    pub shrink: Scaled,
}

impl Totals {
    /// §975: any infinite stretch at all makes a short page perfectly good.
    fn has_infinite_stretch(&self) -> bool {
        self.stretch[1] != 0 || self.stretch[2] != 0 || self.stretch[3] != 0
    }

    /// Add a glue node's stretch and shrink (§976, §1004).
    ///
    /// The shrink is added whatever its order, exactly as `tex.web` does: it
    /// adds first and only then complains that infinite shrinkage does not
    /// belong in a list being broken, "since the offensive shrinkability has
    /// been made finite".
    fn add(&mut self, spec: &Glue) {
        let order = spec.stretch_order.clamp(0, 3) as usize;
        self.stretch[order] += spec.stretch;
        self.shrink += spec.shrink;
    }

    /// Whether that glue was the infinite shrinkage §976 objects to.
    fn infinite_shrink(spec: &Glue) -> bool {
        spec.shrink_order != 0 && spec.shrink != 0
    }
}

/// §970-§975: how bad a break at the current point would be, given a goal
/// height `h` and the totals accumulated so far.
fn page_badness(totals: &Totals, h: Scaled) -> i64 {
    if totals.height < h {
        return match totals.has_infinite_stretch() {
            true => 0,
            false => badness(h - totals.height, totals.stretch[0]),
        };
    }
    if totals.height - h > totals.shrink {
        return AWFUL_BAD;
    }
    badness(totals.height - h, totals.shrink)
}

/// The penalty `\end` contributes to force the last page out (§1054):
/// `\hbox to \hsize{}\vfill\penalty-'10000000000`.
///
/// Far below `eject_penalty`, and deliberately so: it must beat any penalty a
/// document could have written to hold the page together.
pub const END_PENALTY: i64 = -0o10_000_000_000_i64;

/// §974: the penalty at a break, folded into the badness to give a cost.
fn cost(b: i64, pi: i64) -> i64 {
    if b >= AWFUL_BAD {
        return b;
    }
    if pi <= EJECT_PENALTY {
        return pi;
    }
    match b < INF_BAD {
        true => b + pi,
        false => DEPLORABLE,
    }
}

/// What `vert_break` found (§970, §971).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Break {
    /// Where to cut, as an index into the list. `None` is the artificial
    /// forced break after the last node.
    pub place: Option<usize>,
    /// `best_height_plus_depth` (§971): the natural size of the box that
    /// break produces, which the insertion splitter needs.
    pub height_plus_depth: Scaled,
    /// `least_cost` (§970).
    pub cost: i64,
    /// Whether §976's infinite-shrinkage condition was met on the way.
    pub infinite_shrink: bool,
}

/// Which branch of §972's switch a node takes.
enum Step {
    /// A legal breakpoint carrying this penalty.
    Legal(i64),
    /// `goto update_heights`.
    UpdateHeights,
    /// `goto not_found`.
    NotFound,
}

/// `vert_break(p,h,d)` (§970-§976): the best single place to cut a vertical
/// list to get a box of height `h` whose depth is at most `d`.
///
/// Unlike the paragraph breaker this finds ONE break rather than an optimal
/// sequence, and it stops as soon as the list is too full to accept more —
/// there is no point pricing a break past the one that already overflows.
pub fn vert_break(list: &[Node], h: Scaled, d: Scaled) -> Break {
    let mut totals = Totals::default();
    let mut prev_dp: Scaled = 0;
    let mut least_cost = AWFUL_BAD;
    let mut best_place: Option<usize> = None;
    let mut best_hpd: Scaled = 0;
    let mut infinite_shrink = false;
    // §970: "an initial glue node is not a legal breakpoint", which falls out
    // of `prev_p` starting equal to `p`.
    let mut prev_p: usize = 0;
    let mut p: usize = 0;

    loop {
        let step = match list.get(p) {
            // §972: the list has run out; the break there is forced.
            None => Step::Legal(EJECT_PENALTY),
            Some(Node::Box(_)) | Some(Node::Rule(_)) => {
                let (bh, bd) = height_and_depth(&list[p]);
                totals.height += prev_dp + bh;
                prev_dp = bd;
                Step::NotFound
            }
            Some(Node::Whatsit(_)) => Step::NotFound,
            Some(Node::Glue(_)) => match list[prev_p].precedes_break() {
                true => Step::Legal(0),
                false => Step::UpdateHeights,
            },
            // §972: a kern is a breakpoint only when glue follows it, and a
            // kern at the very end of the list is treated as followed by a
            // penalty, i.e. not a breakpoint.
            Some(Node::Kern { .. }) => match list.get(p + 1).map(Node::type_code) {
                Some(10) => Step::Legal(0),
                _ => Step::UpdateHeights,
            },
            Some(Node::Penalty(pi)) => Step::Legal(*pi),
            Some(Node::Mark(_)) | Some(Node::Ins(_)) => Step::NotFound,
            // `confusion("vertbreak")`: nothing else belongs in a vlist.
            Some(_) => Step::NotFound,
        };

        if let Step::Legal(pi) = step {
            // §973: check whether this is a new champion.
            if pi < INF_PENALTY {
                let b = page_badness(&totals, h);
                let c = cost(b, pi);
                if c <= least_cost {
                    best_place = match p < list.len() {
                        true => Some(p),
                        false => None,
                    };
                    least_cost = c;
                    best_hpd = totals.height + prev_dp;
                }
                if b == AWFUL_BAD || pi <= EJECT_PENALTY {
                    break;
                }
            }
        }

        // §972: fall through to update_heights only for glue and kerns.
        let glue_or_kern = matches!(list.get(p).map(Node::type_code), Some(10) | Some(11));
        let update = matches!(step, Step::UpdateHeights)
            || (glue_or_kern && !matches!(step, Step::NotFound));
        if update {
            // §976.
            match &list[p] {
                Node::Kern { width, .. } => {
                    totals.height += prev_dp + width;
                }
                Node::Glue(g) => {
                    totals.add(&g.spec);
                    infinite_shrink |= Totals::infinite_shrink(&g.spec);
                    totals.height += prev_dp + g.spec.natural;
                }
                _ => {}
            }
            prev_dp = 0;
        }

        // §972's `not_found`: a depth past the limit is charged to the height.
        if prev_dp > d {
            totals.height += prev_dp - d;
            prev_dp = d;
        }
        prev_p = p;
        p += 1;
    }

    Break {
        place: best_place,
        height_plus_depth: best_hpd,
        cost: least_cost,
        infinite_shrink,
    }
}

/// The height and depth a node contributes to a vertical list.
fn height_and_depth(node: &Node) -> (Scaled, Scaled) {
    match node {
        Node::Box(b) => (b.height, b.depth),
        Node::Rule(r) => (r.height, r.depth),
        _ => (0, 0),
    }
}

/// `prune_page_top(p)` (§968): drop the glue, kerns and penalties that precede
/// the first box of a list, and put `\splittopskip` in front of it instead.
///
/// This is why the first line of a continued page sits where it does rather
/// than where the break left it: the glue that was holding the two lines apart
/// on the old page is thrown away, and a fresh piece is made whose width puts
/// the new top baseline `\splittopskip` from the top — "whenever this is
/// possible without backspacing", which is what the clamp at zero means.
pub fn prune_page_top(list: Vec<Node>, split_top_skip: Glue) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(list.len() + 1);
    let mut pruning = true;
    for node in list {
        if !pruning {
            out.push(node);
            continue;
        }
        match &node {
            // §968: a box or rule ends the pruning, and the `\splittopskip`
            // goes in ahead of it.
            Node::Box(_) | Node::Rule(_) => {
                let (height, _) = height_and_depth(&node);
                let natural = (split_top_skip.natural - height).max(0);
                out.push(Node::Glue(GlueNode::param(
                    Glue {
                        natural,
                        ..split_top_skip
                    },
                    GlueSource::SplitTopSkip,
                )));
                out.push(node);
                pruning = false;
            }
            // Whatsits, marks and insertions survive the pruning: a mark that
            // was thrown away here would take a running head with it.
            Node::Whatsit(_) | Node::Mark(_) | Node::Ins(_) => out.push(node),
            // Glue, kerns and penalties are discarded.
            _ => {}
        }
    }
    out
}

/// What `\vsplit` produced (§977).
#[derive(Clone, Debug)]
pub struct Split {
    /// The box of height `h` that was extracted.
    pub extracted: BoxNode,
    /// What is left of the original box, or `None` if it was used up.
    pub remainder: Option<BoxNode>,
    /// `\splitfirstmark`.
    pub first_mark: Option<String>,
    /// `\splitbotmark`.
    pub bot_mark: Option<String>,
}

/// `vsplit(n,h)` (§977-§979): cut a vbox in two at the best place for a box of
/// height `h`.
///
/// The marks in the part that came off become `\splitfirstmark` and
/// `\splitbotmark`, which is how a long table split across pages knows which
/// rows it is showing.
pub fn vsplit(
    boxed: BoxNode,
    h: Scaled,
    split_max_depth: Scaled,
    split_top_skip: Glue,
    tol: Tolerances,
) -> Option<Split> {
    // §978: an hbox cannot be split, and a void box splits to nothing.
    if !boxed.vertical {
        return None;
    }
    let list = boxed.list;
    let brk = vert_break(&list, h, split_max_depth);
    let cut = brk.place.unwrap_or(list.len());

    // §979: the marks before the break are the split marks.
    let mut first_mark: Option<String> = None;
    let mut bot_mark: Option<String> = None;
    let mut head: Vec<Node> = Vec::with_capacity(cut);
    let mut tail: Vec<Node> = Vec::new();
    for (i, node) in list.into_iter().enumerate() {
        if i < cut {
            if let Node::Mark(text) = &node {
                if first_mark.is_none() {
                    first_mark = Some(text.clone());
                }
                bot_mark = Some(text.clone());
            }
            head.push(node);
        } else {
            tail.push(node);
        }
    }

    let rest = prune_page_top(tail, split_top_skip);
    let remainder = match rest.is_empty() {
        true => None,
        false => Some(vpack(rest, NATURAL, tol).node),
    };
    let extracted = vpackage(head, Spec::Exactly(h), split_max_depth, tol).node;
    Some(Split {
        extracted,
        remainder,
        first_mark,
        bot_mark,
    })
}

/// What is on the current page so far (§980).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Contents {
    /// Marks and content-less whatsits only.
    #[default]
    Empty,
    /// Insertions, but no box yet.
    InsertsOnly,
    /// A box or rule has arrived, so the page specifications are frozen.
    BoxThere,
}

/// One insertion class's registers (§1008-§1009).
///
/// `\count n` scales an insertion's contribution to the page (1000 means "as
/// tall as it is"), `\dimen n` caps how much of class `n` one page may hold,
/// and `\skip n` is the glue the page pays for having any of it at all —
/// LaTeX's `\skip\footins` is the space above the footnote rule.
///
/// The default is TeX's: `\count n = 0`, `\dimen n = 0`, `\skip n = 0pt`. A
/// class left at those values holds nothing, so a document that uses
/// insertions sets all three.
#[derive(Clone, Copy, Debug, Default)]
pub struct InsertClass {
    pub count: i64,
    pub dimen: Scaled,
    pub skip: Glue,
}

/// A page insertion record (§981): what class `n` has accumulated on the page
/// being built.
#[derive(Clone, Debug)]
struct PageIns {
    number: u8,
    /// `type(r)`: `false` is `inserting`, `true` is `split_up`.
    split_up: bool,
    /// The height-plus-depth of the box and everything inserted into it.
    height: Scaled,
    /// Where the insertion that overflowed would be split.
    broken_ptr: Option<usize>,
    /// Which insertion node that was, as an index into the current page.
    broken_ins: Option<usize>,
    /// The most recent insertion of this class that would go on this page.
    last_ins_ptr: Option<usize>,
    /// The one that should actually go, for the best break known.
    best_ins_ptr: Option<usize>,
}

/// One of the eight entries of `page_so_far` (§982), each of which is both a
/// `\pagegoal`-style value the document can read and one it can assign to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PageDimen {
    /// `\pagegoal` — `page_so_far[0]`.
    Goal,
    /// `\pagetotal` — `page_so_far[1]`.
    Total,
    /// `\pagestretch` and its three infinite orders — `page_so_far[2..5]`.
    Stretch(usize),
    /// `\pageshrink` — `page_so_far[6]`.
    Shrink,
    /// `\pagedepth` — `page_so_far[7]`.
    Depth,
}

/// The page parameters the builder freezes when the page gets its first box
/// (§987).
#[derive(Clone, Debug)]
pub struct PageParams {
    /// `\vsize`.
    pub vsize: Scaled,
    /// `\maxdepth`.
    pub max_depth: Scaled,
    /// `\topskip`: the glue above the first box on the page.
    pub top_skip: Glue,
    /// `\splittopskip`.
    pub split_top_skip: Glue,
    /// `\splitmaxdepth`.
    pub split_max_depth: Scaled,
    /// `\holdinginserts`: non-zero leaves insertions on the page for the
    /// output routine to deal with.
    pub holding_inserts: i64,
    /// `\maxdeadcycles`.
    pub max_dead_cycles: i64,
    pub tolerances: Tolerances,
    /// The registers of each insertion class a document uses.
    pub inserts: BTreeMap<u8, InsertClass>,
}

impl Default for PageParams {
    /// Plain TeX's page: `\vsize=8.9in`, `\maxdepth=4pt`,
    /// `\topskip=10pt`, `\splittopskip=10pt`, `\splitmaxdepth=\maxdimen`.
    fn default() -> PageParams {
        let pt = |n: i64| n * crate::dimen::UNITY;
        PageParams {
            vsize: crate::dimen::to_scaled(8, crate::dimen::round_decimals("9"), "in")
                .unwrap_or(pt(643)),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            split_top_skip: Glue::fixed(pt(10)),
            split_max_depth: MAX_DIMEN,
            holding_inserts: 0,
            max_dead_cycles: 25,
            tolerances: Tolerances::plain(),
            inserts: BTreeMap::new(),
        }
    }
}

/// What the builder decided to do with a completed page (§1023, §1025).
#[derive(Clone, Debug)]
pub enum Fired {
    /// There is no `\output`, so `\box255` is shipped as it stands — §1023's
    /// "default output routine", which is `\shipout\box255` and nothing else.
    ShipOut(BoxNode),
    /// `\output` is set. `\box255` holds the page and the routine has to run;
    /// the builder is suspended until [`PageBuilder::resume_output`] is called
    /// with the vertical list the routine built.
    Output {
        /// The page, already in `\box255`.
        page: BoxNode,
        /// `\outputpenalty`: the penalty at the break, which the routine may
        /// read to find out why it was called.
        output_penalty: i64,
        /// How many outputs in a row have shipped nothing (§1024).
        dead_cycles: i64,
    },
}

/// The page builder (§980-§1028).
///
/// Material is `contribute`d; the builder moves it to the current page,
/// pricing every legal breakpoint as it goes, and fires the output routine
/// when it finds one it cannot improve on.
pub struct PageBuilder {
    pub params: PageParams,
    /// The current page: everything the builder has accounted for.
    page: Vec<Node>,
    /// The contribution list: what has arrived but not been looked at.
    contrib: VecDeque<Node>,
    contents: Contents,
    totals: Totals,
    page_goal: Scaled,
    page_depth: Scaled,
    page_max_depth: Scaled,
    best_page_break: Option<usize>,
    least_page_cost: i64,
    best_size: Scaled,
    page_ins: Vec<PageIns>,
    insert_penalties: i64,
    output_active: bool,
    dead_cycles: i64,
    output_penalty: i64,
    /// Whether the document has an `\output` routine at all.
    pub has_output_routine: bool,
    /// `\topmark`, `\firstmark`, `\botmark` (§1012).
    pub top_mark: Option<String>,
    pub first_mark: Option<String>,
    pub bot_mark: Option<String>,
    /// The insertion boxes, and `\box255` while a page is in it.
    pub boxes: BTreeMap<u8, BoxNode>,
    /// `\lastskip`, `\lastpenalty`, `\lastkern` (§982, §996).
    pub last_glue: Option<Glue>,
    pub last_penalty: i64,
    pub last_kern: Scaled,
    /// Every box `hpack`/`vpack` would have complained about, in order.
    pub reports: Vec<String>,
}

impl PageBuilder {
    pub fn new(params: PageParams) -> PageBuilder {
        let mut b = PageBuilder {
            params,
            page: Vec::new(),
            contrib: VecDeque::new(),
            contents: Contents::Empty,
            totals: Totals::default(),
            page_goal: 0,
            page_depth: 0,
            page_max_depth: 0,
            best_page_break: None,
            least_page_cost: AWFUL_BAD,
            best_size: 0,
            page_ins: Vec::new(),
            insert_penalties: 0,
            output_active: false,
            dead_cycles: 0,
            output_penalty: INF_PENALTY,
            has_output_routine: false,
            top_mark: None,
            first_mark: None,
            bot_mark: None,
            boxes: BTreeMap::new(),
            last_glue: None,
            last_penalty: 0,
            last_kern: 0,
            reports: Vec::new(),
        };
        b.start_new_page();
        b
    }

    /// `\pagegoal` (§982). Zero until the page has its first box, because the
    /// specifications are not frozen before then.
    pub fn page_goal(&self) -> Scaled {
        self.page_goal
    }

    /// `\pagetotal` (§982).
    pub fn page_total(&self) -> Scaled {
        self.totals.height
    }

    /// `\pagestretch`, `\pagefilstretch`, `\pagefillstretch`,
    /// `\pagefilllstretch` (§982), by order.
    pub fn page_stretch(&self, order: usize) -> Scaled {
        self.totals.stretch[order.min(3)]
    }

    /// `\pageshrink` (§982).
    pub fn page_shrink(&self) -> Scaled {
        self.totals.shrink
    }

    /// `\pagedepth` (§982).
    pub fn page_depth(&self) -> Scaled {
        self.page_depth
    }

    /// `alter_page_so_far` (§1245): `\pagegoal`, `\pagetotal` and the rest are
    /// WRITABLE, not just readable.
    ///
    /// This is not a curiosity. An output routine that decides to keep some of
    /// its page — a two-column routine that has just set the first column, or
    /// LaTeX's float mechanism deciding a float will not fit — says so by
    /// assigning to `\pagegoal`, and the builder must go on from the value it
    /// was given rather than from the one it had frozen. `\pagegoal=\maxdimen`
    /// is how a routine says "do not break again yet".
    pub fn set_page_so_far(&mut self, which: PageDimen, value: Scaled) {
        match which {
            PageDimen::Goal => self.page_goal = value,
            PageDimen::Total => self.totals.height = value,
            PageDimen::Stretch(order) => self.totals.stretch[order.min(3)] = value,
            PageDimen::Shrink => self.totals.shrink = value,
            PageDimen::Depth => self.page_depth = value,
        }
    }

    /// The same eight values, read by the same name.
    pub fn page_so_far(&self, which: PageDimen) -> Scaled {
        match which {
            PageDimen::Goal => self.page_goal,
            PageDimen::Total => self.totals.height,
            PageDimen::Stretch(order) => self.totals.stretch[order.min(3)],
            PageDimen::Shrink => self.totals.shrink,
            PageDimen::Depth => self.page_depth,
        }
    }

    /// `\insertpenalties` (§982): the penalties of everything held over.
    pub fn insert_penalties(&self) -> i64 {
        self.insert_penalties
    }

    /// `\outputpenalty` (§1013).
    pub fn output_penalty(&self) -> i64 {
        self.output_penalty
    }

    /// `\deadcycles` (§1024).
    pub fn dead_cycles(&self) -> i64 {
        self.dead_cycles
    }

    /// What is on the current page so far, as §980 classifies it.
    pub fn contents(&self) -> Contents {
        self.contents
    }

    /// The current page, for a caller that wants to look at it — `\showlists`
    /// prints exactly this.
    pub fn current_page(&self) -> &[Node] {
        &self.page
    }

    /// Whether the contribution list still holds anything.
    pub fn contributions_pending(&self) -> bool {
        !self.contrib.is_empty()
    }

    /// Append material to the contribution list and run the builder over it
    /// (§994's `build_page`).
    pub fn contribute(&mut self, nodes: Vec<Node>) -> Vec<Fired> {
        self.contrib.extend(nodes);
        self.build_page()
    }

    /// `\end`: the last page has to come out even though nothing forced it.
    ///
    /// §1054's `\end` fires `\hbox to \hsize{}\vfill\penalty-'10000000000`,
    /// which is a forced break after glue that can absorb any shortfall. That
    /// is what this contributes, so the final page is broken by the same code
    /// that broke every other one rather than by a special case.
    pub fn finish(&mut self) -> Vec<Fired> {
        let mut fired = self.contribute(vec![
            Node::Box(BoxNode::null()),
            Node::Glue(GlueNode::new(Glue {
                stretch: crate::dimen::UNITY,
                stretch_order: 2,
                ..Glue::default()
            })),
            Node::Penalty(END_PENALTY),
        ]);
        fired.extend(self.build_page());
        fired
    }

    /// §991: everything the page keeps between one output and the next.
    fn start_new_page(&mut self) {
        self.contents = Contents::Empty;
        self.page.clear();
        self.last_glue = None;
        self.last_penalty = 0;
        self.last_kern = 0;
        self.page_depth = 0;
        self.page_max_depth = 0;
    }

    /// `freeze_page_specs(s)` (§987): the page takes the `\vsize` and
    /// `\maxdepth` in force at the moment it receives its first box, and keeps
    /// them however the document changes them afterwards.
    fn freeze_page_specs(&mut self, s: Contents) {
        self.contents = s;
        self.page_goal = self.params.vsize;
        self.page_max_depth = self.params.max_depth;
        self.page_depth = 0;
        self.totals = Totals::default();
        self.least_page_cost = AWFUL_BAD;
    }

    /// `build_page` (§994-§1005).
    fn build_page(&mut self) -> Vec<Fired> {
        let mut fired = Vec::new();
        if self.output_active {
            return fired;
        }
        while !self.contrib.is_empty() {
            let Some(node) = self.contrib.pop_front() else {
                break;
            };
            self.record_last(&node);
            match self.move_to_page(node) {
                Move::Linked => {}
                Move::Discarded => {}
                // §998: a kern at the end of the contribution list is not
                // contributed until its successor is known.
                Move::Wait(node) => {
                    self.contrib.push_front(node);
                    return fired;
                }
                Move::Fire(node) => {
                    let out = self.fire_up(Some(node));
                    fired.push(out);
                    if self.output_active {
                        return fired;
                    }
                }
            }
        }
        fired
    }

    /// §996: `\lastskip`, `\lastpenalty` and `\lastkern` name the last thing
    /// contributed.
    fn record_last(&mut self, node: &Node) {
        self.last_penalty = 0;
        self.last_kern = 0;
        self.last_glue = None;
        match node {
            Node::Glue(g) => self.last_glue = Some(g.spec),
            Node::Penalty(p) => self.last_penalty = *p,
            Node::Kern { width, .. } => self.last_kern = *width,
            _ => {}
        }
    }

    /// What happened to a node the builder looked at.
    fn move_to_page(&mut self, node: Node) -> Move {
        // §997's switch.
        let pi = match &node {
            Node::Box(_) | Node::Rule(_) => {
                if self.contents < Contents::BoxThere {
                    // §1001: the page gets its \topskip, and the box is looked
                    // at again behind it.
                    if self.contents == Contents::Empty {
                        self.freeze_page_specs(Contents::BoxThere);
                    } else {
                        self.contents = Contents::BoxThere;
                    }
                    let (height, _) = height_and_depth(&node);
                    let natural = (self.params.top_skip.natural - height).max(0);
                    let skip = Node::Glue(GlueNode::param(
                        Glue {
                            natural,
                            ..self.params.top_skip
                        },
                        GlueSource::TopSkip,
                    ));
                    self.contrib.push_front(node);
                    self.contrib.push_front(skip);
                    return Move::Discarded;
                }
                // §1002.
                let (height, depth) = height_and_depth(&node);
                self.totals.height += self.page_depth + height;
                self.page_depth = depth;
                return self.contribute_node(node);
            }
            Node::Whatsit(_) | Node::Mark(_) => return self.contribute_node(node),
            Node::Ins(ins) => {
                let ins = ins.clone();
                self.append_insertion(&ins);
                return self.contribute_node(node);
            }
            Node::Glue(g) => {
                if self.contents < Contents::BoxThere {
                    return Move::Discarded;
                }
                match self.page.last().map(Node::precedes_break) {
                    // §985: the page head is a glue node, so glue at the very
                    // top of a page is not a legal break.
                    Some(true) => Some(0),
                    _ => {
                        self.update_heights(&Node::Glue(g.clone()));
                        return self.contribute_node(node);
                    }
                }
            }
            Node::Kern { .. } => {
                if self.contents < Contents::BoxThere {
                    return Move::Discarded;
                }
                match self.contrib.front().map(Node::type_code) {
                    None => return Move::Wait(node),
                    Some(10) => Some(0),
                    _ => {
                        self.update_heights(&node);
                        return self.contribute_node(node);
                    }
                }
            }
            Node::Penalty(p) => match self.contents < Contents::BoxThere {
                true => return Move::Discarded,
                false => Some(*p),
            },
            // `confusion("page")`: nothing else reaches the main vertical list.
            _ => return self.contribute_node(node),
        };

        // §1005: check whether this is a new champion breakpoint.
        if let Some(pi) = pi {
            if pi < INF_PENALTY {
                let b = self.break_badness();
                let mut c = cost(b, pi);
                if self.insert_penalties >= 10000 {
                    c = AWFUL_BAD;
                }
                if c <= self.least_page_cost {
                    self.best_page_break = Some(self.page.len());
                    self.best_size = self.page_goal;
                    self.least_page_cost = c;
                    for r in &mut self.page_ins {
                        r.best_ins_ptr = r.last_ins_ptr;
                    }
                }
                if c == AWFUL_BAD || pi <= EJECT_PENALTY {
                    return Move::Fire(node);
                }
            }
        }
        // Glue and kerns that were legal breakpoints still update the totals.
        if matches!(node.type_code(), 10 | 11) {
            self.update_heights(&node);
        }
        self.contribute_node(node)
    }

    /// §1007: how bad the page would be if it were broken here.
    fn break_badness(&self) -> i64 {
        page_badness(&self.totals, self.page_goal)
    }

    /// §1004: a glue or kern node's effect on the page measurements.
    fn update_heights(&mut self, node: &Node) {
        match node {
            Node::Kern { width, .. } => self.totals.height += self.page_depth + width,
            Node::Glue(g) => {
                self.totals.add(&g.spec);
                if Totals::infinite_shrink(&g.spec) {
                    self.reports
                        .push("Infinite glue shrinkage found on current page".into());
                }
                self.totals.height += self.page_depth + g.spec.natural;
            }
            _ => return,
        }
        self.page_depth = 0;
    }

    /// §1003 and §999: clamp the depth, then link the node onto the page.
    fn contribute_node(&mut self, node: Node) -> Move {
        if self.page_depth > self.page_max_depth {
            self.totals.height += self.page_depth - self.page_max_depth;
            self.page_depth = self.page_max_depth;
        }
        self.page.push(node);
        Move::Linked
    }

    /// §1008-§1010: account for an insertion on the current page.
    fn append_insertion(&mut self, p: &InsNode) {
        if self.contents == Contents::Empty {
            self.freeze_page_specs(Contents::InsertsOnly);
        }
        let n = p.number;
        let class = self.params.inserts.get(&n).copied().unwrap_or_default();
        let here = self.page.len();

        if !self.page_ins.iter().any(|r| r.number == n) {
            // §1009: the first insertion of this class on this page freezes
            // what `\box n` already holds and pays for `\skip n` up front.
            let held = self
                .boxes
                .get(&n)
                .map(BoxNode::height_plus_depth)
                .unwrap_or(0);
            let h = match class.count == 1000 {
                true => held,
                false => x_over_n(held, 1000) * class.count,
            };
            self.page_goal -= h + class.skip.natural;
            self.totals.add(&class.skip);
            if Totals::infinite_shrink(&class.skip) {
                self.reports
                    .push(format!("Infinite glue shrinkage inserted from \\skip{n}"));
            }
            self.page_ins.push(PageIns {
                number: n,
                split_up: false,
                height: held,
                broken_ptr: None,
                broken_ins: None,
                last_ins_ptr: None,
                best_ins_ptr: None,
            });
            self.page_ins.sort_by_key(|r| r.number);
        }

        let index = self
            .page_ins
            .iter()
            .position(|r| r.number == n)
            .expect("the record was just made");
        if self.page_ins[index].split_up {
            // §1008: this class has already overflowed, so everything further
            // is held over and costs its floating penalty.
            self.insert_penalties += p.float_cost;
            return;
        }
        self.page_ins[index].last_ins_ptr = Some(here);
        let delta = self.page_goal - self.totals.height - self.page_depth + self.totals.shrink;
        let h = match class.count == 1000 {
            true => p.height,
            false => x_over_n(p.height, 1000) * class.count,
        };
        let r_height = self.page_ins[index].height;
        if (h <= 0 || h <= delta) && p.height + r_height <= class.dimen {
            self.page_goal -= h;
            self.page_ins[index].height += p.height;
            return;
        }
        // §1010: it does not fit, so find where it would be split.
        let mut w = match class.count <= 0 {
            true => MAX_DIMEN,
            false => {
                let w = self.page_goal - self.totals.height - self.page_depth;
                match class.count == 1000 {
                    true => w,
                    false => x_over_n(w, class.count) * 1000,
                }
            }
        };
        if w > class.dimen - r_height {
            w = class.dimen - r_height;
        }
        let brk = vert_break(&p.list, w, p.depth);
        self.page_ins[index].height += brk.height_plus_depth;
        let mut charged = brk.height_plus_depth;
        if class.count != 1000 {
            charged = x_over_n(charged, 1000) * class.count;
        }
        self.page_goal -= charged;
        self.page_ins[index].split_up = true;
        self.page_ins[index].broken_ptr = brk.place;
        self.page_ins[index].broken_ins = Some(here);
        self.insert_penalties += match brk.place.and_then(|i| p.list.get(i)) {
            Some(Node::Penalty(pen)) => *pen,
            Some(_) => 0,
            None => EJECT_PENALTY,
        };
    }

    /// `fire_up(c)` (§1012-§1023): package the best page into `\box255`, put
    /// the insertions in their boxes, and hand it to the output routine.
    fn fire_up(&mut self, mut contributing: Option<Node>) -> Fired {
        let cut = self.best_page_break.unwrap_or(self.page.len());
        // §1013: `\outputpenalty` is the penalty at the break, and that
        // penalty node is set to `inf_penalty` so the same break cannot be
        // taken again — the node goes back on the contribution list at §1017,
        // and an armed forced penalty there would fire an empty page at once.
        //
        // The break may be at the node still being CONTRIBUTED rather than at
        // one already on the page (§1015: "c not yet linked in"), which is the
        // usual case for `\penalty-10000`, so both are looked at.
        let penalty_at_break = {
            let node = match cut < self.page.len() {
                true => self.page.get_mut(cut),
                false => contributing.as_mut(),
            };
            match node {
                Some(Node::Penalty(p)) => {
                    let value = *p;
                    *p = INF_PENALTY;
                    Some(value)
                }
                _ => None,
            }
        };
        self.output_penalty = penalty_at_break.unwrap_or(INF_PENALTY);
        // §1012: `\topmark` becomes what `\botmark` was.
        if self.bot_mark.is_some() {
            self.top_mark = self.bot_mark.clone();
            self.first_mark = None;
        }

        self.insert_penalties = 0;
        let mut page = std::mem::take(&mut self.page);
        let leftover: Vec<Node> = page.split_off(cut.min(page.len()));
        let mut held_over: Vec<Node> = Vec::new();
        let mut kept: Vec<Node> = Vec::with_capacity(page.len());

        for (i, node) in page.into_iter().enumerate() {
            match node {
                Node::Ins(ins) if self.params.holding_inserts <= 0 => {
                    // §1020-§1022.
                    if self.place_insertion(i, ins.clone()) {
                        self.insert_penalties += 1;
                        held_over.push(Node::Ins(ins));
                    }
                }
                Node::Mark(text) => {
                    // §1016.
                    if self.first_mark.is_none() {
                        self.first_mark = Some(text.clone());
                    }
                    self.bot_mark = Some(text.clone());
                    kept.push(Node::Mark(text));
                }
                other => kept.push(other),
            }
        }

        // §1017: what follows the break goes back to the front of the
        // contribution list, ahead of anything that arrived since.
        let mut back: VecDeque<Node> = leftover.into();
        if let Some(node) = contributing {
            back.push_back(node);
        }
        back.append(&mut self.contrib);
        self.contrib = back;

        // §1017: errors are suppressed here, because the stretch and shrink of
        // the insertion `\skip`s are not in the box even though the goal was
        // reduced by them.
        let quiet = Tolerances {
            vbadness: INF_BAD,
            vfuzz: MAX_DIMEN,
            ..self.params.tolerances
        };
        let page = vpackage(
            kept,
            Spec::Exactly(self.best_size),
            self.page_max_depth,
            quiet,
        )
        .node;
        self.boxes.insert(255, page.clone());

        self.start_new_page();
        self.page = held_over;
        self.page_ins.clear();
        self.best_page_break = None;
        self.least_page_cost = AWFUL_BAD;

        // §1012: `\firstmark` falls back to `\topmark` when the page had no
        // marks of its own.
        if self.first_mark.is_none() {
            self.first_mark = self.top_mark.clone();
        }
        if self.top_mark.is_none() {
            self.top_mark = self.first_mark.clone();
        }

        if self.has_output_routine && self.dead_cycles < self.params.max_dead_cycles {
            // §1025.
            self.output_active = true;
            self.dead_cycles += 1;
            return Fired::Output {
                page,
                output_penalty: self.output_penalty,
                dead_cycles: self.dead_cycles,
            };
        }
        // §1023: the default output routine.
        let held = std::mem::take(&mut self.page);
        let mut back: VecDeque<Node> = held.into();
        back.append(&mut self.contrib);
        self.contrib = back;
        self.boxes.remove(&255);
        Fired::ShipOut(page)
    }

    /// §1020-§1022: put one insertion's material into its box, or say it must
    /// be held over.
    ///
    /// Returns whether the insertion node is held over to the next page.
    fn place_insertion(&mut self, at: usize, p: InsNode) -> bool {
        let n = p.number;
        let Some(index) = self.page_ins.iter().position(|r| r.number == n) else {
            return true;
        };
        if self.page_ins[index].best_ins_ptr.is_none() {
            return true;
        }
        let is_last = self.page_ins[index].best_ins_ptr == Some(at);
        let split_here = self.page_ins[index].split_up
            && self.page_ins[index].broken_ins == Some(at)
            && self.page_ins[index].broken_ptr.is_some();

        let mut material = p.list;
        let mut wait = false;
        if is_last && split_here {
            // §1022: the part after the break stays behind for the next page.
            let cut = self.page_ins[index].broken_ptr.unwrap_or(material.len());
            let rest = material.split_off(cut.min(material.len()));
            let rest = prune_page_top(rest, p.split_top_skip);
            if !rest.is_empty() {
                wait = true;
            }
        }

        let target = self.boxes.entry(n).or_insert_with(BoxNode::null);
        // §1021: an insertion box is a vbox, and the material queues onto it.
        target.vertical = true;
        target.list.append(&mut material);

        if is_last {
            // §1022: package the box, now that nothing more will go into it.
            let taken = self.boxes.remove(&n).unwrap_or_else(BoxNode::null);
            let packed = vpack(taken.list, NATURAL, self.params.tolerances);
            self.boxes.insert(n, packed.node);
            self.page_ins[index].best_ins_ptr = None;
        }
        wait
    }

    /// §1026: the output routine has finished, and the vertical list it built
    /// goes back for another look.
    ///
    /// Everything the routine did NOT ship — LaTeX's held-over floats are the
    /// case that matters — goes ahead of the held-over insertions and ahead of
    /// whatever arrived while it was running, which is the order that keeps a
    /// float from jumping past the text it was declared in.
    pub fn resume_output(&mut self, built: Vec<Node>) -> Vec<Fired> {
        self.output_active = false;
        self.insert_penalties = 0;
        if self.boxes.remove(&255).is_some() {
            self.reports
                .push("Output routine didn't use all of \\box255".into());
        }
        self.page.extend(built);
        let held = std::mem::take(&mut self.page);
        let mut back: VecDeque<Node> = held.into();
        back.append(&mut self.contrib);
        self.contrib = back;
        self.build_page()
    }

    /// `\shipout` from inside an output routine: the page went out, so the
    /// dead-cycle counter goes back to zero (§1025's `dead_cycles:=0`).
    pub fn shipped(&mut self) {
        self.dead_cycles = 0;
    }
}

/// What became of a node the builder looked at.
enum Move {
    /// It is on the page now.
    Linked,
    /// It was thrown away, or put back for a second look.
    Discarded,
    /// A kern with nothing after it yet: put it back and wait.
    Wait(Node),
    /// It forced a page break; the node has not been linked in.
    Fire(Node),
}

/// The penalties TeX puts BETWEEN the lines of a paragraph (§890).
#[derive(Clone, Copy, Debug, Default)]
pub struct ParPenalties {
    /// `\interlinepenalty`: charged between every pair of lines.
    pub inter_line: i64,
    /// `\clubpenalty`: charged after the FIRST line, so a page break there
    /// leaves a club line behind.
    pub club: i64,
    /// `\widowpenalty`: charged before the LAST line, so a break there carries
    /// a widow line forward.
    pub widow: i64,
    /// `\brokenpenalty`: charged after a line that ended in a hyphen.
    pub broken: i64,
}

impl ParPenalties {
    /// LaTeX's own defaults: `\clubpenalty=150`, `\widowpenalty=150`,
    /// `\brokenpenalty=100`, `\interlinepenalty=0`.
    pub fn latex() -> ParPenalties {
        ParPenalties {
            inter_line: 0,
            club: 150,
            widow: 150,
            broken: 100,
        }
    }
}

/// §890: the penalty node that follows line `line` of a paragraph of `total`
/// lines, if a nonzero one is appropriate.
///
/// The last line is never penalised, because there is nothing after it to
/// break from. Note that both club and widow are charged in a two-line
/// paragraph — §890 says so explicitly, and it falls out of the arithmetic
/// rather than being a special case.
pub fn interline_penalty(
    line: usize,
    total: usize,
    disc_break: bool,
    p: ParPenalties,
) -> Option<i64> {
    if line + 1 >= total {
        return None;
    }
    let mut pen = p.inter_line;
    if line == 0 {
        pen += p.club;
    }
    if line + 2 == total {
        pen += p.widow;
    }
    if disc_break {
        pen += p.broken;
    }
    match pen {
        0 => None,
        pen => Some(pen),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimen::UNITY;
    use crate::pack::Baselines;

    fn pt(n: i64) -> Scaled {
        n * UNITY
    }

    fn line(height: Scaled, depth: Scaled) -> Node {
        Node::Box(BoxNode {
            width: pt(100),
            height,
            depth,
            ..BoxNode::null()
        })
    }

    fn skip(natural: Scaled, stretch: Scaled, shrink: Scaled) -> Node {
        Node::Glue(GlueNode::new(Glue {
            natural,
            stretch,
            shrink,
            ..Glue::default()
        }))
    }

    /// A vertical list of `n` boxes 10pt tall with 2pt of glue between them.
    fn column(n: usize) -> Vec<Node> {
        let mut list = Vec::new();
        for i in 0..n {
            if i > 0 {
                list.push(skip(pt(2), pt(1), pt(1)));
            }
            list.push(line(pt(10), 0));
        }
        list
    }

    /// §970-§975: the break that leaves the page closest to its goal wins, and
    /// the cost is `badness` rather than "does it fit".
    #[test]
    fn vert_break_picks_the_cheapest_place_not_the_last_that_fits() {
        // Five 10pt lines with 2pt glue: heights at each glue are 10, 22, 34,
        // 46, and the total is 58.
        let list = column(5);
        // Boxes at 0,2,4,6,8 and glue at 1,3,5,7. The heights reached at each
        // glue are 10, 22, 34 and 46, so a goal of 34pt is a perfect fit at
        // the glue after the third line.
        let brk = vert_break(&list, pt(34), pt(4));
        assert_eq!(brk.place, Some(5));
        assert_eq!(brk.cost, 0);
        assert_eq!(brk.height_plus_depth, pt(34));
        // A goal of 30pt fits nowhere: the glue at 5 is 4pt over and can
        // shrink only 2pt, which is `awful_bad` and ends the search (§973).
        // Both earlier breaks are `deplorable`, and §973 takes the LAST of
        // equal costs -- `if b<=least_cost` -- so the later one wins and the
        // page is as full as it can be made.
        let brk = vert_break(&list, pt(30), pt(4));
        assert_eq!(brk.place, Some(3));
    }

    /// §972: glue is a legal breakpoint only after something that stays, so
    /// the glue at the very top of a list is not one.
    #[test]
    fn a_list_that_starts_with_glue_cannot_be_broken_at_it() {
        let list = vec![skip(pt(20), 0, 0), line(pt(10), 0)];
        // Asking for a 0pt page: breaking at the leading glue would give a
        // perfect fit, and it is not offered.
        let brk = vert_break(&list, 0, pt(4));
        assert_eq!(brk.place, None);
    }

    /// §973: a penalty of `-10000` or worse forces the break wherever it is.
    #[test]
    fn a_forced_penalty_ends_the_search_where_it_stands() {
        let mut list = column(5);
        list.insert(2, Node::Penalty(EJECT_PENALTY));
        let brk = vert_break(&list, pt(100), pt(4));
        assert_eq!(brk.place, Some(2));
        assert_eq!(brk.cost, EJECT_PENALTY);
    }

    /// §973: `\nobreak` is `\penalty10000`, and `pi<inf_penalty` is the test
    /// that refuses it -- so a page will overflow rather than break there.
    #[test]
    fn nobreak_is_not_a_breakpoint_at_any_cost() {
        let mut list = column(4);
        // Forbid the break after the second line.
        list.insert(3, Node::Penalty(INF_PENALTY));
        let brk = vert_break(&list, pt(22), pt(4));
        // The glue at index 3 is now preceded by a penalty, which does not
        // precede a break, so neither the penalty nor that glue is offered:
        // the break lands at the next legal place instead.
        assert_ne!(brk.place, Some(3));
        assert_ne!(brk.place, Some(4));
    }

    /// §975: a page carrying infinite stretch is never short, which is what
    /// `\vfill` at the end of a chapter relies on -- the page ends there at no
    /// cost rather than being priced as 470pt of empty paper.
    ///
    /// The comparison is against the same list with rigid glue in place of the
    /// `\vfil`, where the identical break is `deplorable`.
    #[test]
    fn infinite_stretch_makes_a_short_page_perfect() {
        let column = |filler: Glue| {
            vec![
                line(pt(10), 0),
                Node::Glue(GlueNode::new(filler)),
                line(pt(10), 0),
                Node::Glue(GlueNode::new(Glue::default())),
                line(pt(500), 0),
            ]
        };
        let vfil = Glue {
            stretch: UNITY,
            stretch_order: 1,
            ..Glue::default()
        };
        // With the \vfil, the break after the second line costs nothing.
        let brk = vert_break(&column(vfil), pt(100), pt(4));
        assert_eq!(brk.place, Some(3));
        assert_eq!(brk.cost, 0);
        // Without it, the same break is 80pt short of the goal with no stretch
        // to give: `badness` is infinite and §974 prices it `deplorable`.
        let brk = vert_break(&column(Glue::default()), pt(100), pt(4));
        assert_eq!(brk.place, Some(3));
        assert_eq!(brk.cost, DEPLORABLE);
    }

    /// §968: the material after a split loses the glue that was holding it to
    /// what came before, and gains `\splittopskip` instead.
    #[test]
    fn pruning_a_page_top_replaces_the_glue_with_splittopskip() {
        let list = vec![
            skip(pt(2), pt(1), 0),
            Node::Penalty(50),
            line(pt(7), pt(1)),
            skip(pt(2), 0, 0),
        ];
        let pruned = prune_page_top(list, Glue::fixed(pt(10)));
        assert_eq!(pruned.len(), 3);
        let Node::Glue(g) = &pruned[0] else {
            panic!("splittopskip is a glue node");
        };
        // 10pt of \splittopskip less the 7pt height of the box below it.
        assert_eq!(g.spec.natural, pt(3));
        assert_eq!(g.source, GlueSource::SplitTopSkip);
        // And the top baseline is then \splittopskip from the top: 3 + 7.
        assert_eq!(g.spec.natural + pt(7), pt(10));
    }

    /// §968: a box taller than `\splittopskip` gets no backspacing -- the glue
    /// goes to zero rather than negative.
    #[test]
    fn splittopskip_never_backspaces() {
        let pruned = prune_page_top(vec![line(pt(30), 0)], Glue::fixed(pt(10)));
        let Node::Glue(g) = &pruned[0] else {
            panic!("splittopskip is a glue node");
        };
        assert_eq!(g.spec.natural, 0);
    }

    /// §977-§979: the extracted box is exactly the height asked for, the rest
    /// stays behind, and the marks in the part that came off are the split
    /// marks.
    #[test]
    fn vsplit_cuts_a_box_in_two_and_reports_its_marks() {
        let mut list = column(5);
        list.insert(0, Node::Mark("first".into()));
        list.insert(4, Node::Mark("second".into()));
        let boxed = vpack(list, NATURAL, Tolerances::plain()).node;
        let split = vsplit(
            boxed,
            pt(34),
            pt(4),
            Glue::fixed(pt(10)),
            Tolerances::plain(),
        )
        .expect("a vbox splits");
        assert_eq!(split.extracted.height, pt(34));
        assert_eq!(split.first_mark.as_deref(), Some("first"));
        assert_eq!(split.bot_mark.as_deref(), Some("second"));
        let rest = split.remainder.expect("something was left");
        // Two lines and the glue between them, under a fresh \splittopskip.
        assert!(rest.height > 0);
    }

    /// §978: an hbox cannot be split at all.
    #[test]
    fn vsplit_refuses_an_hbox() {
        let boxed = crate::pack::hpack(Vec::new(), NATURAL, Tolerances::plain(), None).node;
        assert!(vsplit(
            boxed,
            pt(10),
            pt(4),
            Glue::fixed(pt(10)),
            Tolerances::plain()
        )
        .is_none());
    }

    /// §1001: the first box on a page gets `\topskip`, and the glue is
    /// `\topskip` LESS that box's height, so the first baseline sits
    /// `\topskip` from the top of the page whatever the line measures.
    #[test]
    fn topskip_puts_the_first_baseline_where_it_belongs() {
        let params = PageParams {
            vsize: pt(200),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        };
        let mut builder = PageBuilder::new(params);
        builder.contribute(vec![line(pt(7), pt(2))]);
        let page = builder.current_page();
        assert_eq!(page.len(), 2);
        let Node::Glue(g) = &page[0] else {
            panic!("\\topskip is a glue node");
        };
        assert_eq!(g.source, GlueSource::TopSkip);
        assert_eq!(g.spec.natural, pt(3));
        assert_eq!(builder.page_total(), pt(10));
        assert_eq!(builder.page_depth(), pt(2));
        assert_eq!(builder.page_goal(), pt(200));
    }

    /// §997: glue and penalties before the first box are discarded, which is
    /// why a `\vskip` at the top of a page does nothing.
    #[test]
    fn glue_before_the_first_box_is_thrown_away() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(200),
            ..PageParams::default()
        });
        builder.contribute(vec![
            skip(pt(50), 0, 0),
            Node::Penalty(100),
            line(pt(10), 0),
        ]);
        assert_eq!(builder.contents(), Contents::BoxThere);
        // \topskip glue and the box: the 50pt skip and the penalty are gone.
        assert_eq!(builder.current_page().len(), 2);
        assert_eq!(builder.page_total(), pt(10));
    }

    /// §1005: `\penalty-10000` forces the page out where it stands.
    #[test]
    fn a_forced_break_ships_the_page_at_once() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(500),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        });
        let mut nodes = column(3);
        nodes.push(Node::Penalty(EJECT_PENALTY));
        nodes.extend(column(2));
        let fired = builder.contribute(nodes);
        assert_eq!(fired.len(), 1);
        let Fired::ShipOut(page) = &fired[0] else {
            panic!("with no \\output the page ships");
        };
        // \topskip 10pt to the first baseline, then 10+2 twice: 34pt, packaged
        // to the \pagegoal that was frozen when the page took its first box.
        assert_eq!(page.height, pt(500));
        assert_eq!(builder.output_penalty(), EJECT_PENALTY);
        // What followed the break went back to be looked at again: two more
        // lines, 10pt each with 2pt between them, under a fresh \topskip.
        assert_eq!(builder.page_total(), pt(10 + 2 + 10));
        // §1013: the penalty that fired is disarmed, so it cannot fire the
        // empty page that follows it.
        assert_eq!(builder.current_page().len(), 4);
    }

    /// §1005 and §1007: a page that cannot be shrunk to the goal is
    /// `awful_bad`, and the builder takes the best break it had rather than
    /// going further.
    #[test]
    fn an_overfull_page_breaks_at_the_best_place_already_seen() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(34),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        });
        // Lines at 10, 22, 34, 46: the goal is 34, so the break after the
        // third line is perfect and the fourth overflows.
        let fired = builder.contribute(column(6));
        assert_eq!(fired.len(), 1);
        let Fired::ShipOut(page) = &fired[0] else {
            panic!("with no \\output the page ships");
        };
        assert_eq!(page.height, pt(34));
        // Three lines and the two glues between them.
        assert_eq!(page.list.len(), 6);
    }

    /// §1245: `\pagegoal` is an assignable parameter, and assigning to it
    /// moves the break. The goal frozen from `\vsize` would have given a 34pt
    /// page of three lines; raising it to 46pt gives four.
    #[test]
    fn assigning_to_pagegoal_moves_the_break() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(34),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        });
        // The first box freezes the specifications at \vsize (§987).
        assert!(builder.contribute(vec![line(pt(10), 0)]).is_empty());
        assert_eq!(builder.page_goal(), pt(34));

        builder.set_page_so_far(PageDimen::Goal, pt(46));
        assert_eq!(builder.page_so_far(PageDimen::Goal), pt(46));

        let mut rest = Vec::new();
        for _ in 0..5 {
            rest.push(skip(pt(2), pt(1), pt(1)));
            rest.push(line(pt(10), 0));
        }
        let fired = builder.contribute(rest);
        assert_eq!(fired.len(), 1);
        let Fired::ShipOut(page) = &fired[0] else {
            panic!("with no \\output the page ships");
        };
        assert_eq!(page.height, pt(46));
        // Four lines and the three glues between them.
        assert_eq!(page.list.len(), 8);
    }

    /// §1025-§1026: with an `\output` routine the builder stops and hands over
    /// `\box255`, and picks up again with whatever the routine left behind.
    #[test]
    fn an_output_routine_gets_box255_and_hands_back_a_list() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(34),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        });
        builder.has_output_routine = true;
        let fired = builder.contribute(column(6));
        assert_eq!(fired.len(), 1);
        let Fired::Output {
            page, dead_cycles, ..
        } = &fired[0]
        else {
            panic!("an \\output routine was set");
        };
        assert_eq!(page.height, pt(34));
        assert_eq!(*dead_cycles, 1);
        // \box255 is the routine's to empty.
        assert!(builder.boxes.contains_key(&255));
        // The routine ships it and returns nothing; building resumes with the
        // material that followed the break.
        builder.boxes.remove(&255);
        builder.shipped();
        let fired = builder.resume_output(Vec::new());
        assert_eq!(builder.dead_cycles(), 0);
        // Three lines are left and they exactly fill the page, but nothing has
        // forced them out: a page is shipped when a BREAK is found, not when
        // the material runs out.
        assert!(fired.is_empty());
        assert_eq!(builder.page_total(), pt(34));
        // §1054: `\end` is what forces the last one, and it does it with a
        // `\vfill` and a penalty rather than with a special case.
        let fired = builder.finish();
        assert_eq!(fired.len(), 1);
        let Fired::Output { page, .. } = &fired[0] else {
            panic!("the routine is still set");
        };
        assert_eq!(page.height, pt(34));
    }

    /// §1012, §1016: `\topmark` is what `\botmark` was on the page before,
    /// `\firstmark` is the first mark on this page, `\botmark` the last.
    #[test]
    fn the_three_marks_say_what_was_on_the_page() {
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(34),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            ..PageParams::default()
        });
        // Three 10pt lines is exactly the 34pt goal, so the forced break at
        // the end of the run makes one page of them.
        let page_of = |marks: [&str; 2]| {
            vec![
                line(pt(10), 0),
                Node::Mark(marks[0].into()),
                skip(pt(2), pt(1), pt(1)),
                line(pt(10), 0),
                Node::Mark(marks[1].into()),
                skip(pt(2), pt(1), pt(1)),
                line(pt(10), 0),
                Node::Penalty(EJECT_PENALTY),
            ]
        };
        let fired = builder.contribute(page_of(["A", "B"]));
        assert_eq!(fired.len(), 1);
        // The first page carried A and B.
        assert_eq!(builder.first_mark.as_deref(), Some("A"));
        assert_eq!(builder.bot_mark.as_deref(), Some("B"));
        // Nothing preceded it, so `\topmark` is empty in tex; here it falls
        // back to the first mark rather than being a different kind of
        // nothing.
        assert_eq!(builder.top_mark.as_deref(), Some("A"));

        // §1012: on the NEXT page, `\topmark` is what `\botmark` was --
        // which is exactly how a running head names the section a page
        // started in the middle of.
        let fired = builder.contribute(page_of(["C", "D"]));
        assert_eq!(fired.len(), 1);
        assert_eq!(builder.top_mark.as_deref(), Some("B"));
        assert_eq!(builder.first_mark.as_deref(), Some("C"));
        assert_eq!(builder.bot_mark.as_deref(), Some("D"));
    }

    /// §1008-§1009: an insertion takes its space out of `\pagegoal` before the
    /// text is priced against it, which is the whole mechanism behind a
    /// footnote pushing text onto the next page.
    #[test]
    fn an_insertion_reduces_the_page_goal_by_what_it_takes() {
        let mut inserts = BTreeMap::new();
        inserts.insert(
            1,
            InsertClass {
                count: 1000,
                dimen: pt(200),
                skip: Glue::fixed(pt(5)),
            },
        );
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(200),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            inserts,
            ..PageParams::default()
        });
        builder.contribute(vec![
            line(pt(10), 0),
            Node::Ins(InsNode {
                number: 1,
                height: pt(20),
                depth: pt(4),
                float_cost: 100,
                split_top_skip: Glue::fixed(pt(10)),
                list: vec![line(pt(20), 0)],
            }),
        ]);
        // 200pt of \vsize, less the 5pt of \skip1 the page pays for having a
        // footnote at all, less the 20pt the footnote is tall.
        assert_eq!(builder.page_goal(), pt(200 - 5 - 20));
    }

    /// §1020-§1022: when the page goes out, the insertion's material is in
    /// `\box n` rather than on the page, packaged as a vbox.
    #[test]
    fn a_fired_page_leaves_its_insertions_in_their_boxes() {
        let mut inserts = BTreeMap::new();
        inserts.insert(
            1,
            InsertClass {
                count: 1000,
                dimen: pt(200),
                skip: Glue::fixed(pt(5)),
            },
        );
        let mut builder = PageBuilder::new(PageParams {
            vsize: pt(100),
            max_depth: pt(4),
            top_skip: Glue::fixed(pt(10)),
            inserts,
            ..PageParams::default()
        });
        let fired = builder.contribute(vec![
            line(pt(10), 0),
            Node::Ins(InsNode {
                number: 1,
                height: pt(20),
                depth: 0,
                float_cost: 100,
                split_top_skip: Glue::fixed(pt(10)),
                list: vec![line(pt(20), 0)],
            }),
            skip(pt(2), pt(1), pt(1)),
            line(pt(10), 0),
            Node::Penalty(EJECT_PENALTY),
        ]);
        assert_eq!(fired.len(), 1);
        let Fired::ShipOut(page) = &fired[0] else {
            panic!("with no \\output the page ships");
        };
        // The insertion node itself is off the page.
        assert!(!page.list.iter().any(|n| matches!(n, Node::Ins(_))));
        let footnotes = builder.boxes.get(&1).expect("\\box1 holds the footnote");
        assert!(footnotes.vertical);
        assert_eq!(footnotes.height, pt(20));
    }

    /// §890: both club and widow are charged in a two-line paragraph, and the
    /// last line is never penalised.
    #[test]
    fn widow_and_club_penalties_land_where_tex_puts_them() {
        let p = ParPenalties::latex();
        // A two-line paragraph: one penalty, carrying both.
        assert_eq!(interline_penalty(0, 2, false, p), Some(300));
        assert_eq!(interline_penalty(1, 2, false, p), None);
        // A four-line paragraph: club after the first, widow before the last.
        assert_eq!(interline_penalty(0, 4, false, p), Some(150));
        assert_eq!(interline_penalty(1, 4, false, p), None);
        assert_eq!(interline_penalty(2, 4, false, p), Some(150));
        assert_eq!(interline_penalty(3, 4, false, p), None);
        // A hyphenated line adds \brokenpenalty on top.
        assert_eq!(interline_penalty(1, 4, true, p), Some(100));
        // A one-line paragraph is never penalised at all.
        assert_eq!(interline_penalty(0, 1, false, p), None);
    }

    /// The pieces together: four lines appended with `\baselineskip`, and the
    /// widow and club penalties §890 puts between them. With
    /// `\widowpenalty=10000` — which is what a document that refuses widows
    /// outright sets — the break before the last line is FORBIDDEN rather than
    /// merely expensive, so the page has to end a line earlier.
    ///
    /// This is the whole chain in one place: `append_to_vlist` makes the
    /// interline glue, `interline_penalty` puts §890's penalties in it, and
    /// `vert_break` refuses the place they forbid.
    #[test]
    fn a_widow_penalty_moves_the_break_a_line_earlier() {
        // A stretchable `\baselineskip`, so the earlier break is merely bad
        // rather than impossible: 12pt plus 6pt, on 10pt lines with no depth,
        // leaves 2pt plus 6pt of interline glue.
        let baselines = Baselines {
            baseline_skip: Glue {
                natural: pt(12),
                stretch: pt(6),
                ..Glue::default()
            },
            ..Baselines::plain()
        };
        let refuse_widows = ParPenalties {
            widow: 10000,
            club: 10000,
            ..ParPenalties::latex()
        };
        let make = |penalties: Option<ParPenalties>| {
            let mut list = Vec::new();
            let mut prev_depth = crate::node::IGNORE_DEPTH;
            for i in 0..4 {
                crate::pack::append_to_vlist(
                    &mut list,
                    BoxNode {
                        width: pt(100),
                        height: pt(10),
                        ..BoxNode::null()
                    },
                    &mut prev_depth,
                    baselines,
                );
                if let Some(p) = penalties {
                    if let Some(pen) = interline_penalty(i, 4, false, p) {
                        list.push(Node::Penalty(pen));
                    }
                }
            }
            list
        };
        // Heights reached at the three interline glues are 10, 22 and 34, so
        // a goal of 34pt is a perfect fit at the last of them.
        let brk = vert_break(&make(None), pt(34), pt(4));
        assert_eq!(brk.place, Some(5));
        assert_eq!(brk.cost, 0);

        // With the penalties in, that place carries `\widowpenalty=10000`,
        // and §973's `pi<inf_penalty` refuses it outright. The glue AFTER a
        // penalty is not a breakpoint either (§148), so the page ends at the
        // previous line -- 800 badness rather than 0, and taken anyway.
        let penalised = vert_break(&make(Some(refuse_widows)), pt(34), pt(4));
        assert_eq!(penalised.place, Some(4));
        assert_eq!(penalised.cost, badness(pt(12), pt(6)));
        assert_eq!(penalised.cost, 800);
    }
}
