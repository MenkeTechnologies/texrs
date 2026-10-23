//! Breaking a paragraph's node list into lines: `tex.web` §813-§876.
//!
//! `crate::linebreak` breaks a paragraph given as measured text, for the
//! string path a LaTeX document still takes. This is the algorithm itself, over
//! the horizontal list the stomach built: Knuth and Plass's shortest path
//! through the feasible breakpoints, with the active list, the passive list
//! and the delta nodes of §819-§824, `try_break` (§829-§860), the three
//! passes of §863 and the `\looseness` search of §873-§875. Its answer is the
//! chain of breakpoints `crate::postline::post_line_break` (§877-§890) turns
//! into lines.
//!
//! The active list is a `Vec` whose order is the linked list's order, and a
//! `tex.web` pointer into the paragraph is an index into the node list (`None`
//! is `null`, the end of the paragraph). Everything else is arithmetic on
//! scaled points exactly as written, which is the point: §852's
//! `shortfall>7230584` guard and §859's `d*d` decide which lines a paragraph
//! gets, and a port that rounds differently sets different lines.

use crate::glue::Glue;
use crate::node::{Node, Scaled, AWFUL_BAD, EJECT_PENALTY, INF_BAD, INF_PENALTY};
use crate::postline::LineShape;

/// §817's fitness classes.
const VERY_LOOSE_FIT: u8 = 0;
const LOOSE_FIT: u8 = 1;
const DECENT_FIT: u8 = 2;
const TIGHT_FIT: u8 = 3;

/// `max_halfword` (§110), the line number of `last_active` (§820).
const MAX_HALFWORD: i64 = 0x0FFF_FFFF;

/// The parameters §813-§876 read from `eqtb`.
#[derive(Clone, Debug)]
pub struct BreakParams {
    pub pretolerance: i64,
    pub tolerance: i64,
    pub line_penalty: i64,
    pub hyphen_penalty: i64,
    pub ex_hyphen_penalty: i64,
    pub adj_demerits: i64,
    pub double_hyphen_demerits: i64,
    pub final_hyphen_demerits: i64,
    pub looseness: i64,
    pub emergency_stretch: Scaled,
    pub left_skip: Glue,
    pub right_skip: Glue,
    /// `\hsize`, `\hangindent`/`\hangafter` or `\parshape`, as §848-§849
    /// reduce them.
    pub shape: LineShape,
    /// `prev_graf` of the enclosing list: where line numbering starts
    /// (§814).
    pub prev_graf: i64,
}

/// One entry of the active list (§819, §822).
#[derive(Clone, Debug)]
enum Entry {
    Active {
        /// `break_node`: the passive node of this break, `None` for the
        /// beginning of the paragraph.
        passive: Option<usize>,
        line_number: i64,
        fitness: u8,
        hyphenated: bool,
        total_demerits: i64,
    },
    /// A delta node: the six differences of §822.
    Delta([Scaled; 6]),
}

/// A passive node (§821).
#[derive(Clone, Debug)]
struct Passive {
    /// `cur_break`: where the line ends; `None` is the end of the paragraph.
    cur_break: Option<usize>,
    prev_break: Option<usize>,
}

/// What `line_break` decided.
#[derive(Clone, Debug)]
pub struct Breaks {
    /// The breakpoints, first line first (§878 has reversed the chain).
    /// `None` is the end of the paragraph.
    pub breaks: Vec<Option<usize>>,
    /// `best_line`: one more than the number of the paragraph's last line.
    pub best_line: i64,
    /// §826 found infinite shrinkage and made it finite; tex reports
    /// `Infinite glue shrinkage found in a paragraph` once.
    pub infinite_shrinkage: bool,
}

/// The state of one paragraph's search: §823's six-tuples, §833's
/// minimal demerits, and the two lists.
struct Search<'a> {
    list: &'a [Node],
    p: &'a BreakParams,
    active: Vec<Entry>,
    passive: Vec<Passive>,
    active_width: [Scaled; 6],
    cur_active_width: [Scaled; 6],
    background: [Scaled; 6],
    break_width: [Scaled; 6],
    minimal_demerits: [i64; 4],
    minimum_demerits: i64,
    best_place: [Option<usize>; 4],
    best_pl_line: [i64; 4],
    disc_width: Scaled,
    threshold: i64,
    final_pass: bool,
    easy_line: i64,
    last_special_line: i64,
    second_width: Scaled,
}

/// The width of a node that a line containing it is charged for (§841,
/// §842, §870, §871): a character, a ligature, a box, a rule or a kern.
fn node_width(n: &Node) -> Scaled {
    match n {
        Node::Char(c) | Node::Ligature(c, _) => c.width,
        Node::Box(b) => b.width,
        Node::Rule(r) => r.width,
        Node::Kern { width, .. } => *width,
        _ => 0,
    }
}

/// §825's `check_shrinkage`: infinite shrink becomes finite.
fn finite(g: &mut Glue) -> bool {
    match g.shrink_order != 0 && g.shrink != 0 {
        true => {
            g.shrink_order = 0;
            true
        }
        false => false,
    }
}

/// `line_break` (§815) up to `post_line_break`: find the optimal breakpoints
/// of the paragraph `list`, which §816 has already ended with `\penalty10000
/// \parfillskip`.
///
/// The list is the stomach's, and §826's `check_shrinkage` edits its glue in
/// place.
/// The hyphenation pass's word-breaking: called once with the paragraph
/// before the first pass that may hyphenate (§863's `second_pass`), it puts
/// the discretionaries §894-§918 would in. Every word that follows a glue
/// node the scan reaches gets its turn in tex too -- a pass ends early only
/// when a later one will scan the rest -- and what a word becomes depends on
/// the word and what follows it, not on the breaks, so doing them all at the
/// start of the pass sets the same list.
pub type Hyphenate<'a> = &'a mut dyn FnMut(&mut Vec<Node>);

pub fn line_break(list: &mut Vec<Node>, p: &BreakParams, mut hyphenate: Option<Hyphenate>) -> Breaks {
    let mut infinite_shrinkage = false;
    // §827 for the two skips; §868 does the paragraph's own glue as it goes,
    // but the answer is the same done once here.
    let (mut left, mut right) = (p.left_skip, p.right_skip);
    infinite_shrinkage |= finite(&mut left);
    infinite_shrinkage |= finite(&mut right);
    for n in list.iter_mut() {
        if let Node::Glue(g) = n {
            infinite_shrinkage |= finite(&mut g.spec);
        }
    }
    let mut background = [0; 6];
    background[0] = left.natural + right.natural;
    background[1 + left.stretch_order.clamp(0, 3) as usize] += left.stretch;
    background[1 + right.stretch_order.clamp(0, 3) as usize] += right.stretch;
    background[5] = left.shrink + right.shrink;

    // §848.
    let last_special_line = p.shape.last_special_line() as i64;
    let easy_line = match p.looseness == 0 {
        true => last_special_line,
        false => MAX_HALFWORD,
    };
    // §863.
    let (mut threshold, mut second_pass, mut final_pass) = match p.pretolerance >= 0 {
        true => (p.pretolerance, false, false),
        false => (p.tolerance, true, p.emergency_stretch <= 0),
    };
    let mut hyphenated = false;
    loop {
        if threshold > INF_BAD {
            threshold = INF_BAD;
        }
        if second_pass && !hyphenated {
            hyphenated = true;
            if let Some(h) = hyphenate.as_mut() {
                h(list);
            }
        }
        let mut s = Search {
            list,
            p,
            active: Vec::new(),
            passive: Vec::new(),
            active_width: [0; 6],
            cur_active_width: [0; 6],
            background,
            break_width: [0; 6],
            minimal_demerits: [AWFUL_BAD; 4],
            minimum_demerits: AWFUL_BAD,
            best_place: [None; 4],
            best_pl_line: [0; 4],
            disc_width: 0,
            threshold,
            final_pass,
            easy_line,
            last_special_line,
            second_width: p.shape.second_width(),
        };
        // §864: the beginning of the paragraph.
        s.active = vec![Entry::Active {
            passive: None,
            line_number: p.prev_graf + 1,
            fitness: DECENT_FIT,
            hyphenated: false,
            total_demerits: 0,
        }];
        s.active_width = s.background;
        if let Some((passive, best_line)) = s.scan() {
            // §878: the chain from the best break back to the start,
            // reversed.
            let mut breaks = Vec::new();
            let mut q = passive;
            while let Some(i) = q {
                breaks.push(s.passive[i].cur_break);
                q = s.passive[i].prev_break;
            }
            breaks.reverse();
            return Breaks {
                breaks,
                best_line,
                infinite_shrinkage,
            };
        }
        // §863: try again.
        if !second_pass {
            threshold = p.tolerance;
            second_pass = true;
            final_pass = p.emergency_stretch <= 0;
        } else {
            background[1] += p.emergency_stretch;
            final_pass = true;
        }
    }
}

impl Search<'_> {
    /// One pass of §863's loop: run through the paragraph calling
    /// `try_break` at every legal breakpoint (§866), then try the final
    /// break (§873). `Some` is the best active node's passive node and line
    /// number when the desired breakpoints were found.
    fn scan(&mut self) -> Option<(Option<usize>, i64)> {
        let list = self.list;
        let mut cur_p = 0usize;
        let mut auto_breaking = true;
        // §863: glue at the beginning is not a legal breakpoint.
        let mut prev_p = 0usize;
        while cur_p < list.len() && !self.active.is_empty() {
            // §867: a run of characters.
            if list[cur_p].is_char() {
                prev_p = cur_p;
                while cur_p < list.len() && list[cur_p].is_char() {
                    self.active_width[0] += node_width(&list[cur_p]);
                    cur_p += 1;
                }
                if cur_p >= list.len() {
                    break;
                }
            }
            // §866.
            match &list[cur_p] {
                Node::Box(_) | Node::Rule(_) => self.active_width[0] += node_width(&list[cur_p]),
                Node::Whatsit(_) | Node::Out(_) => {}
                Node::Glue(g) => {
                    // §868.
                    if auto_breaking {
                        let prev = &list[prev_p];
                        let legal = prev.is_char()
                            || prev.precedes_break()
                            || matches!(prev, Node::Kern { explicit: false, .. });
                        if legal {
                            self.try_break(0, false, Some(cur_p));
                        }
                    }
                    let q = g.spec;
                    self.active_width[0] += q.natural;
                    self.active_width[1 + q.stretch_order.clamp(0, 3) as usize] += q.stretch;
                    self.active_width[5] += q.shrink;
                }
                Node::Kern { width, explicit } => match explicit {
                    true => self.kern_break(cur_p, auto_breaking, *width),
                    false => self.active_width[0] += width,
                },
                Node::AccKern(width) => self.active_width[0] += width,
                Node::Ligature(..) => self.active_width[0] += node_width(&list[cur_p]),
                Node::Disc(d) => {
                    // §869.
                    self.disc_width = d.pre_break.iter().map(node_width).sum();
                    match d.pre_break.is_empty() {
                        true => self.try_break(self.p.ex_hyphen_penalty, true, Some(cur_p)),
                        false => {
                            self.active_width[0] += self.disc_width;
                            self.try_break(self.p.hyphen_penalty, true, Some(cur_p));
                            self.active_width[0] -= self.disc_width;
                        }
                    }
                    let mut s = cur_p + 1;
                    for _ in 0..d.replace_count {
                        if let Some(n) = list.get(s) {
                            self.active_width[0] += node_width(n);
                        }
                        s += 1;
                    }
                    prev_p = cur_p;
                    cur_p = s;
                    continue;
                }
                // §866: no break inside a formula, but after it.
                Node::Math(width) | Node::MathOff(width) => {
                    auto_breaking = matches!(list[cur_p], Node::MathOff(_));
                    let width = *width;
                    self.kern_break(cur_p, auto_breaking, width);
                }
                Node::Penalty(pi) => self.try_break(*pi, false, Some(cur_p)),
                Node::Mark(_) | Node::Ins(_) | Node::Adjust(_) => {}
                Node::Char(_) => {}
            }
            prev_p = cur_p;
            cur_p += 1;
        }
        if cur_p < list.len() {
            return None;
        }
        // §873: the final line break.
        self.try_break(EJECT_PENALTY, true, None);
        let (best_bet, best_line) = self.fewest_demerits()?;
        if self.p.looseness == 0 {
            return Some((best_bet, best_line));
        }
        let (bet, actual) = self.looseness(best_line);
        if actual == self.p.looseness || self.final_pass {
            return Some(bet);
        }
        None
    }

    /// §866's `kern_break`: an explicit kern or a math node is a legal
    /// breakpoint when glue follows it.
    fn kern_break(&mut self, cur_p: usize, auto_breaking: bool, width: Scaled) {
        let next_is_glue = matches!(self.list.get(cur_p + 1), Some(Node::Glue(_)));
        if next_is_glue && auto_breaking {
            self.try_break(0, false, Some(cur_p));
        }
        self.active_width[0] += width;
    }

    /// §874: the active node with the fewest demerits, by its passive node
    /// and line number.
    fn fewest_demerits(&self) -> Option<(Option<usize>, i64)> {
        let mut fewest = AWFUL_BAD;
        let mut best = None;
        for e in &self.active {
            if let Entry::Active {
                passive,
                line_number,
                total_demerits,
                ..
            } = e
            {
                if *total_demerits < fewest {
                    fewest = *total_demerits;
                    best = Some((*passive, *line_number));
                }
            }
        }
        best
    }

    /// §875: the best active node for the desired `\looseness`, and how
    /// loose it came out.
    fn looseness(&self, best_line: i64) -> ((Option<usize>, i64), i64) {
        let mut actual = 0i64;
        let mut best = (None, best_line);
        let mut fewest = AWFUL_BAD;
        let looseness = self.p.looseness;
        for e in &self.active {
            let Entry::Active {
                passive,
                line_number,
                total_demerits,
                ..
            } = e
            else {
                continue;
            };
            let line_diff = line_number - best_line;
            if (line_diff < actual && looseness <= line_diff)
                || (line_diff > actual && looseness >= line_diff)
            {
                best = (*passive, *line_number);
                actual = line_diff;
                fewest = *total_demerits;
            } else if line_diff == actual && *total_demerits < fewest {
                best = (*passive, *line_number);
                fewest = *total_demerits;
            }
        }
        (best, actual)
    }

    /// The line width of line class `l` (§850), updating `old_l`.
    fn line_width(&self, l: i64, old_l: &mut i64) -> Scaled {
        if l > self.easy_line {
            *old_l = MAX_HALFWORD - 1;
            return self.second_width;
        }
        *old_l = l;
        match l > self.last_special_line {
            true => self.second_width,
            false => self.p.shape.line(l.max(1) as usize).1,
        }
    }

    /// `try_break(pi, break_type)` (§829-§860) at `cur_p` (`None` is the end
    /// of the paragraph).
    fn try_break(&mut self, mut pi: i64, hyphenated: bool, cur_p: Option<usize>) {
        // §831.
        if pi.abs() >= INF_PENALTY {
            if pi > 0 {
                return;
            }
            pi = EJECT_PENALTY;
        }
        let mut no_break_yet = true;
        // `prev_r` and `prev_prev_r` as indices into `active`, `None` being
        // the list head `active`.
        let mut prev_r: Option<usize> = None;
        let mut prev_prev_r: Option<usize> = None;
        let mut old_l = 0i64;
        let mut line_width: Scaled = 0;
        self.cur_active_width = self.active_width;
        let mut r = 0usize;
        loop {
            // §832.
            if let Some(Entry::Delta(d)) = self.active.get(r) {
                for k in 0..6 {
                    self.cur_active_width[k] += d[k];
                }
                prev_prev_r = prev_r;
                prev_r = Some(r);
                r += 1;
                continue;
            }
            // §835: `l` is `line_number(r)`, and `last_active`'s is
            // `max_halfword`.
            let (l, r_fitness, r_hyphenated, r_total, r_passive) = match self.active.get(r) {
                Some(Entry::Active {
                    line_number,
                    fitness,
                    hyphenated,
                    total_demerits,
                    passive,
                }) => (*line_number, *fitness, *hyphenated, *total_demerits, *passive),
                _ => (MAX_HALFWORD, 0, true, 0, None),
            };
            let at_end = r >= self.active.len();
            if l > old_l {
                if self.minimum_demerits < AWFUL_BAD && (old_l != self.easy_line || at_end) {
                    // §836.
                    if no_break_yet {
                        no_break_yet = false;
                        self.compute_break_width(hyphenated, cur_p);
                    }
                    self.delta_before(&mut r, &mut prev_r, &mut prev_prev_r);
                    let adj = self.p.adj_demerits.abs();
                    if adj >= AWFUL_BAD - self.minimum_demerits {
                        self.minimum_demerits = AWFUL_BAD - 1;
                    } else {
                        self.minimum_demerits += adj;
                    }
                    for fit_class in VERY_LOOSE_FIT..=TIGHT_FIT {
                        let fc = fit_class as usize;
                        if self.minimal_demerits[fc] <= self.minimum_demerits {
                            // §845.
                            self.passive.push(Passive {
                                cur_break: cur_p,
                                prev_break: self.best_place[fc],
                            });
                            let node = Entry::Active {
                                passive: Some(self.passive.len() - 1),
                                line_number: self.best_pl_line[fc] + 1,
                                fitness: fit_class,
                                hyphenated,
                                total_demerits: self.minimal_demerits[fc],
                            };
                            self.active.insert(r, node);
                            prev_r = Some(r);
                            r += 1;
                        }
                        self.minimal_demerits[fc] = AWFUL_BAD;
                    }
                    self.minimum_demerits = AWFUL_BAD;
                    // §844.
                    if r < self.active.len() {
                        let mut d = [0; 6];
                        for k in 0..6 {
                            d[k] = self.cur_active_width[k] - self.break_width[k];
                        }
                        self.active.insert(r, Entry::Delta(d));
                        prev_prev_r = prev_r;
                        prev_r = Some(r);
                        r += 1;
                    }
                }
                if r >= self.active.len() {
                    return;
                }
                line_width = self.line_width(l, &mut old_l);
            }
            // §851.
            let mut artificial_demerits = false;
            let shortfall = line_width - self.cur_active_width[0];
            let (b, fit_class) = match shortfall > 0 {
                true => self.stretch_badness(shortfall),
                false => self.shrink_badness(shortfall),
            };
            let node_r_stays_active;
            if b > INF_BAD || pi == EJECT_PENALTY {
                // §854.
                let next_is_last = r + 1 >= self.active.len();
                if self.final_pass
                    && self.minimum_demerits == AWFUL_BAD
                    && next_is_last
                    && prev_r.is_none()
                {
                    artificial_demerits = true;
                } else if b > self.threshold {
                    self.deactivate(&mut r, &mut prev_r, &mut prev_prev_r);
                    continue;
                }
                node_r_stays_active = false;
            } else {
                prev_r = Some(r);
                if b > self.threshold {
                    r += 1;
                    continue;
                }
                node_r_stays_active = true;
            }
            // §855.
            let mut d = match artificial_demerits {
                true => 0,
                false => self.demerits(b, pi, hyphenated, r_hyphenated, cur_p, fit_class, r_fitness),
            };
            d += r_total;
            let fc = fit_class as usize;
            if d <= self.minimal_demerits[fc] {
                self.minimal_demerits[fc] = d;
                self.best_place[fc] = r_passive;
                self.best_pl_line[fc] = l;
                if d < self.minimum_demerits {
                    self.minimum_demerits = d;
                }
            }
            if node_r_stays_active {
                r += 1;
                continue;
            }
            self.deactivate(&mut r, &mut prev_r, &mut prev_prev_r);
        }
    }

    /// §852.
    fn stretch_badness(&self, shortfall: Scaled) -> (i64, u8) {
        let w = &self.cur_active_width;
        if w[2] != 0 || w[3] != 0 || w[4] != 0 {
            return (0, DECENT_FIT);
        }
        if shortfall > 7_230_584 && w[1] < 1_663_497 {
            return (INF_BAD, VERY_LOOSE_FIT);
        }
        let b = crate::pack::badness(shortfall, w[1]);
        let fit = if b > 12 {
            match b > 99 {
                true => VERY_LOOSE_FIT,
                false => LOOSE_FIT,
            }
        } else {
            DECENT_FIT
        };
        (b, fit)
    }

    /// §853.
    fn shrink_badness(&self, shortfall: Scaled) -> (i64, u8) {
        let b = match -shortfall > self.cur_active_width[5] {
            true => INF_BAD + 1,
            false => crate::pack::badness(-shortfall, self.cur_active_width[5]),
        };
        let fit = match b > 12 {
            true => TIGHT_FIT,
            false => DECENT_FIT,
        };
        (b, fit)
    }

    /// §859.
    #[allow(clippy::too_many_arguments)]
    fn demerits(
        &self,
        b: i64,
        pi: i64,
        hyphenated: bool,
        r_hyphenated: bool,
        cur_p: Option<usize>,
        fit_class: u8,
        r_fitness: u8,
    ) -> i64 {
        let mut d = self.p.line_penalty + b;
        d = match d.abs() >= 10000 {
            true => 100_000_000,
            false => d * d,
        };
        if pi != 0 {
            if pi > 0 {
                d += pi * pi;
            } else if pi > EJECT_PENALTY {
                d -= pi * pi;
            }
        }
        if hyphenated && r_hyphenated {
            d += match cur_p.is_some() {
                true => self.p.double_hyphen_demerits,
                false => self.p.final_hyphen_demerits,
            };
        }
        if (i64::from(fit_class) - i64::from(r_fitness)).abs() > 1 {
            d += self.p.adj_demerits;
        }
        d
    }

    /// §837-§842: `break_width`, the active widths after a break at `cur_p`
    /// with the discardable nodes after it gone.
    fn compute_break_width(&mut self, hyphenated: bool, cur_p: Option<usize>) {
        let list = self.list;
        self.break_width = self.background;
        let Some(start) = cur_p else {
            return;
        };
        let mut s = Some(start);
        if hyphenated {
            if let Node::Disc(d) = &list[start] {
                // §840.
                let mut v = start;
                for _ in 0..d.replace_count {
                    v += 1;
                    if let Some(n) = list.get(v) {
                        self.break_width[0] -= node_width(n);
                    }
                }
                for n in &d.post_break {
                    self.break_width[0] += node_width(n);
                }
                self.break_width[0] += self.disc_width;
                s = match d.post_break.is_empty() {
                    true => Some(v + 1),
                    false => None,
                };
            }
        }
        while let Some(i) = s {
            let Some(n) = list.get(i) else {
                break;
            };
            match n {
                Node::Glue(g) => {
                    let v = g.spec;
                    self.break_width[0] -= v.natural;
                    self.break_width[1 + v.stretch_order.clamp(0, 3) as usize] -= v.stretch;
                    self.break_width[5] -= v.shrink;
                }
                Node::Penalty(_) => {}
                Node::Math(w) | Node::MathOff(w) => self.break_width[0] -= w,
                Node::Kern { width, explicit } => match explicit {
                    true => self.break_width[0] -= width,
                    false => break,
                },
                _ => break,
            }
            s = Some(i + 1);
        }
    }

    /// §843: a delta node (or the active widths themselves) ready for the
    /// new active nodes at `cur_p`.
    fn delta_before(
        &mut self,
        r: &mut usize,
        prev_r: &mut Option<usize>,
        prev_prev_r: &mut Option<usize>,
    ) {
        match *prev_r {
            Some(pr) if matches!(self.active[pr], Entry::Delta(_)) => {
                if let Entry::Delta(d) = &mut self.active[pr] {
                    for k in 0..6 {
                        d[k] = d[k] - self.cur_active_width[k] + self.break_width[k];
                    }
                }
            }
            None => self.active_width = self.break_width,
            Some(_) => {
                let mut d = [0; 6];
                for k in 0..6 {
                    d[k] = self.break_width[k] - self.cur_active_width[k];
                }
                self.active.insert(*r, Entry::Delta(d));
                *prev_prev_r = *prev_r;
                *prev_r = Some(*r);
                *r += 1;
            }
        }
    }

    /// §860-§861: take node `r` out of the active list, keeping the deltas
    /// around it consistent. `r` is left at the entry that followed it.
    fn deactivate(
        &mut self,
        r: &mut usize,
        prev_r: &mut Option<usize>,
        prev_prev_r: &mut Option<usize>,
    ) {
        self.active.remove(*r);
        match *prev_r {
            // §861: the first active node went, so the widths absorb the
            // delta that now leads the list.
            None => {
                if let Some(Entry::Delta(d)) = self.active.first().cloned() {
                    for k in 0..6 {
                        self.active_width[k] += d[k];
                    }
                    self.cur_active_width = self.active_width;
                    self.active.remove(0);
                }
            }
            Some(pr) if matches!(self.active[pr], Entry::Delta(_)) => {
                let after = *r;
                if after >= self.active.len() {
                    // The delta before the end is no longer between two
                    // active nodes.
                    if let Entry::Delta(d) = self.active[pr].clone() {
                        for k in 0..6 {
                            self.cur_active_width[k] -= d[k];
                        }
                    }
                    self.active.remove(pr);
                    *prev_r = *prev_prev_r;
                    *r = pr;
                } else if let Entry::Delta(next) = self.active[after].clone() {
                    for k in 0..6 {
                        self.cur_active_width[k] += next[k];
                    }
                    if let Entry::Delta(d) = &mut self.active[pr] {
                        for k in 0..6 {
                            d[k] += next[k];
                        }
                    }
                    self.active.remove(after);
                }
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{CharNode, GlueNode, GlueSource};

    const PT: Scaled = 65536;

    fn word(w: Scaled) -> Node {
        Node::Char(CharNode {
            font: 1,
            character: 'x',
            width: w,
            height: 0,
            depth: 0,
        })
    }

    fn space() -> Node {
        Node::Glue(GlueNode::new(Glue {
            natural: 3 * PT,
            stretch: 2 * PT,
            shrink: PT,
            ..Glue::default()
        }))
    }

    /// §816's ending, as the stomach makes it.
    fn ended(mut list: Vec<Node>) -> Vec<Node> {
        list.push(Node::Penalty(INF_PENALTY));
        list.push(Node::Glue(GlueNode::param(
            Glue {
                stretch: PT,
                stretch_order: 1,
                ..Glue::default()
            },
            GlueSource::ParFillSkip,
        )));
        list
    }

    fn params(hsize: Scaled) -> BreakParams {
        BreakParams {
            pretolerance: 100,
            tolerance: 200,
            line_penalty: 10,
            hyphen_penalty: 50,
            ex_hyphen_penalty: 50,
            adj_demerits: 10000,
            double_hyphen_demerits: 10000,
            final_hyphen_demerits: 5000,
            looseness: 0,
            emergency_stretch: 0,
            left_skip: Glue::default(),
            right_skip: Glue::default(),
            shape: LineShape::fixed(hsize),
            prev_graf: 0,
        }
    }

    /// A paragraph that fits on one line is one line, ending at the end.
    #[test]
    fn a_short_paragraph_is_one_line() {
        let mut list = ended(vec![word(10 * PT), space(), word(10 * PT)]);
        let b = line_break(&mut list, &params(100 * PT), None);
        assert_eq!(b.breaks, vec![None]);
        assert_eq!(b.best_line, 2);
    }

    /// Four words of 20pt with 3pt spaces in a 46pt measure: two per line,
    /// each line exactly full with its space, so the break is at the second
    /// space and both lines are decent.
    #[test]
    fn the_break_falls_where_both_lines_are_best() {
        let mut list = ended(vec![
            word(20 * PT),
            space(),
            word(20 * PT),
            space(),
            word(20 * PT),
            space(),
            word(20 * PT),
        ]);
        let b = line_break(&mut list, &params(43 * PT), None);
        assert_eq!(b.breaks, vec![Some(3), None]);
    }

    /// §831: a penalty of 10000 forbids its break, and -10000 forces one.
    #[test]
    fn penalties_forbid_and_force() {
        let mut list = ended(vec![
            word(10 * PT),
            Node::Penalty(-10000),
            word(10 * PT),
        ]);
        let b = line_break(&mut list, &params(100 * PT), None);
        assert_eq!(b.breaks, vec![Some(1), None]);
    }
}
