//! Turning chosen breakpoints into lines: `post_line_break`.
//!
//! `tex.web` §877-§890, with the line shapes of §848-§849. This is the step
//! between [`crate::linebreak`], which decides WHERE a paragraph breaks, and
//! [`crate::page`], which decides where the resulting vertical list breaks
//! into pages. Nothing in it is a heuristic: given the breakpoints, every
//! line's contents, width, indentation and trailing penalty are determined.
//!
//! Four things happen here that an engine which merely slices a paragraph at
//! the break positions does not do, and each of them is visible on the page:
//!
//! - **The glue at a break becomes `\rightskip`** (§881). It is not deleted
//!   and it is not kept: the node stays where it is with a new specification,
//!   which is why `\raggedright` (`\rightskip=0pt plus1fil`) makes the last
//!   glue of every line absorb the slack instead of spreading it between the
//!   words. A line ending anywhere else gets a `\rightskip` node appended
//!   (§887), so EVERY line ends with one.
//! - **A discretionary break is made compulsory** (§882-§886). The `replace`
//!   nodes are destroyed, the pre-break list is transplanted to the end of
//!   this line and the post-break list to the front of the next. That is what
//!   puts the hyphen at the right margin and, in `\discretionary{}{}{}`
//!   ligature breaks, restores the letters the ligature swallowed.
//! - **The next line's leading glue is pruned** (§879), except after a
//!   discretionary that left something behind. Without it the space that used
//!   to separate two words becomes an indent on the second line.
//! - **`\vadjust` and `\insert` material leaves the line** (§888, via
//!   `hpack`'s `adjust_tail`) and lands on the vertical list AFTER the line
//!   box. A footnote written in the middle of a paragraph reaches the page
//!   builder this way, and only this way.
//!
//! Lines are `Vec`s and a `tex.web` pointer is an index into one. The
//! link-reversal of §878 is therefore not needed — the breakpoints arrive in
//! forward order already — and it is the only part of §877-§890 that is about
//! memory rather than typesetting.

use crate::glue::Glue;
use crate::node::{DiscNode, GlueNode, GlueSource, Node, Scaled};
use crate::pack::{append_to_vlist, hpack, Baselines, Report, Spec, Tolerances};
use crate::page::{interline_penalty, ParPenalties};

/// The glue at the two ends of every line (§881, §887, §889).
///
/// `\leftskip` is omitted from the line entirely when it is zero glue (§889:
/// "if left_skip<>zero_glue"), so the common case costs no node. `\rightskip`
/// is always present, zero or not.
#[derive(Clone, Copy, Debug, Default)]
pub struct Skips {
    /// `\leftskip`.
    pub left_skip: Glue,
    /// `\rightskip`.
    pub right_skip: Glue,
}

/// How wide each line of the paragraph is, and how far it is indented
/// (§847-§849).
///
/// TeX holds this as four scalars plus `\parshape`, because in the common case
/// there are only two distinct line shapes: the ones covered by
/// `\hangindent`/`\hangafter` and all the rest. The four are computed once, at
/// §849, and read per line at §890.
#[derive(Clone, Debug)]
pub struct LineShape {
    /// `last_special_line`: lines past this one all use the second shape.
    last_special_line: usize,
    /// `first_width` / `first_indent`, for lines up to `last_special_line`.
    first: (Scaled, Scaled),
    /// `second_width` / `second_indent`, for the lines after.
    second: (Scaled, Scaled),
    /// `\parshape`, as (indent, width) pairs. Empty when there is none.
    par_shape: Vec<(Scaled, Scaled)>,
}

impl LineShape {
    /// §848: no `\parshape` and no `\hangindent` — every line is `\hsize`
    /// wide and flush left.
    pub fn fixed(hsize: Scaled) -> LineShape {
        LineShape {
            last_special_line: 0,
            first: (0, hsize),
            second: (0, hsize),
            par_shape: Vec::new(),
        }
    }

    /// §849: `\hangindent` and `\hangafter`.
    ///
    /// A NEGATIVE `\hangafter` indents the first `|\hangafter|` lines; a
    /// non-negative one indents everything after line `\hangafter`. A negative
    /// `\hangindent` narrows the line from the RIGHT, which is why the indent
    /// it yields is clamped at zero rather than made negative — the width
    /// shrinks either way, the margin only moves when the indent is positive.
    pub fn hanging(hsize: Scaled, hang_indent: Scaled, hang_after: i64) -> LineShape {
        let narrow = hsize - hang_indent.abs();
        let indent = hang_indent.max(0);
        match hang_after < 0 {
            true => LineShape {
                last_special_line: hang_after.unsigned_abs() as usize,
                first: (indent, narrow),
                second: (0, hsize),
                par_shape: Vec::new(),
            },
            false => LineShape {
                last_special_line: hang_after as usize,
                first: (0, hsize),
                second: (indent, narrow),
                par_shape: Vec::new(),
            },
        }
    }

    /// §848: `\parshape`, as `n` (indent, width) pairs. The last pair governs
    /// every line past the `n`th, which is what makes `\parshape 1 0pt 4in` a
    /// way of setting the whole paragraph narrow.
    ///
    /// An empty shape is no shape at all, and falls back to `\hsize`.
    pub fn par_shape(hsize: Scaled, shape: Vec<(Scaled, Scaled)>) -> LineShape {
        let Some(&last) = shape.last() else {
            return LineShape::fixed(hsize);
        };
        LineShape {
            last_special_line: shape.len() - 1,
            first: (0, hsize),
            second: last,
            par_shape: shape,
        }
    }

    /// §890: the indentation and width of line number `cur_line`, counting
    /// from 1.
    pub fn line(&self, cur_line: usize) -> (Scaled, Scaled) {
        if cur_line > self.last_special_line {
            return self.second;
        }
        match self.par_shape.is_empty() {
            true => self.first,
            // `mem[par_shape_ptr+2*cur_line]`: the `cur_line`th pair, which is
            // one-based in `tex.web` and so one back here.
            false => self.par_shape[cur_line - 1],
        }
    }
}

/// The parameters `post_line_break` reads, gathered so the call does not take
/// eight arguments.
#[derive(Clone, Debug)]
pub struct ParParams {
    pub shape: LineShape,
    pub skips: Skips,
    pub penalties: ParPenalties,
    pub baselines: Baselines,
    pub tolerances: Tolerances,
}

impl ParParams {
    /// Plain TeX's paragraph: `\leftskip=\rightskip=0pt`, `\hsize` wide, plain
    /// interline glue, and LaTeX's club and widow penalties.
    pub fn plain(hsize: Scaled) -> ParParams {
        ParParams {
            shape: LineShape::fixed(hsize),
            skips: Skips::default(),
            penalties: ParPenalties::latex(),
            baselines: Baselines::plain(),
            tolerances: Tolerances::plain(),
        }
    }
}

/// What the paragraph came to.
#[derive(Clone, Debug)]
pub struct Paragraph {
    /// The vertical list: line boxes, the interline glue between them, the
    /// penalties, and whatever `\vadjust` moved out.
    pub list: Vec<Node>,
    /// One entry per line: what `hpack` would have complained about, and
    /// `None` where it was content.
    pub reports: Vec<Option<Report>>,
    /// `\badness` after each line (§646).
    pub badness: Vec<i64>,
}

/// `post_line_break` (§877-§890): cut the paragraph at the chosen breakpoints,
/// justify each line to its own width, and append the lines to a vertical
/// list.
///
/// `breaks` holds the breakpoints in forward order, as indices into `list`.
/// The final breakpoint is the end of the paragraph and is written as any
/// index at or past the end — `tex.web`'s `cur_break` of `null` (§880's `else`
/// branch), which appends `\rightskip` after the last node there is.
///
/// `prev_depth` is the caller's, exactly as in §679: a paragraph appended to a
/// page that already has something on it gets interline glue above its first
/// line, and one that starts a page does not.
pub fn post_line_break(
    list: Vec<Node>,
    breaks: &[usize],
    params: &ParParams,
    prev_depth: &mut Scaled,
) -> Paragraph {
    let mut nodes: Vec<Option<Node>> = list.into_iter().map(Some).collect();
    let total = breaks.len();
    let mut out = Paragraph {
        list: Vec::new(),
        reports: Vec::with_capacity(total),
        badness: Vec::with_capacity(total),
    };
    // §888: what the break left over becomes the front of the next line.
    let mut pending: Vec<Node> = Vec::new();
    let mut pos = 0usize;

    for (i, &brk) in breaks.iter().enumerate() {
        let cur_line = i + 1;
        let brk = brk.min(nodes.len());
        let mut line: Vec<Node> = std::mem::take(&mut pending);
        for slot in nodes.iter_mut().take(brk).skip(pos) {
            if let Some(node) = slot.take() {
                line.push(node);
            }
        }

        // §880: modify the end of the line to reflect the nature of the break.
        let right_skip = Node::Glue(GlueNode::param(
            params.skips.right_skip,
            GlueSource::RightSkip,
        ));
        let mut disc_break = false;
        let mut post_disc_break = false;
        let mut next;
        match nodes.get_mut(brk).and_then(Option::take) {
            // §881: the glue at the break IS the `\rightskip`, respecified.
            Some(Node::Glue(_)) => {
                line.push(right_skip);
                next = brk + 1;
            }
            // §882-§886: make the discretionary compulsory.
            Some(Node::Disc(d)) => {
                // §884: destroy the `replace_count` nodes that follow.
                let end = (brk + 1 + d.replace_count).min(nodes.len());
                for slot in nodes.iter_mut().take(end).skip(brk + 1) {
                    slot.take();
                }
                next = end;
                // The node itself stays, emptied: `replace_count(q):=0` and
                // both lists transplanted away.
                line.push(Node::Disc(DiscNode::default()));
                // §886: the pre-break list ends this line.
                line.extend(d.pre_break);
                // §885: the post-break list begins the next one.
                post_disc_break = !d.post_break.is_empty();
                pending = d.post_break;
                disc_break = true;
                line.push(right_skip);
            }
            // §880: a math or kern node at a break has its width zeroed —
            // the space it was holding is at the margin now.
            Some(Node::Math(_)) => {
                line.push(Node::Math(0));
                line.push(right_skip);
                next = brk + 1;
            }
            Some(Node::Kern { explicit, .. }) => {
                line.push(Node::Kern { width: 0, explicit });
                line.push(right_skip);
                next = brk + 1;
            }
            // A penalty breakpoint keeps its node, at the end of the line.
            Some(other) => {
                line.push(other);
                line.push(right_skip);
                next = brk + 1;
            }
            // §880's `else`: `cur_break` is null, so the line ends with the
            // last node of the paragraph.
            None => {
                line.push(right_skip);
                next = nodes.len();
            }
        }

        // §889: `\leftskip` goes at the left, unless it is zero glue.
        if params.skips.left_skip != Glue::default() {
            line.insert(
                0,
                Node::Glue(GlueNode::param(
                    params.skips.left_skip,
                    GlueSource::LeftSkip,
                )),
            );
        }

        // §890: pack the line to its own width and shift it by its own indent.
        let (indent, width) = params.shape.line(cur_line);
        let mut adjust: Vec<Node> = Vec::new();
        let packed = hpack(
            line,
            Spec::Exactly(width),
            params.tolerances,
            Some(&mut adjust),
        );
        let mut just_box = packed.node;
        just_box.shift_amount = indent;
        out.reports.push(packed.report);
        out.badness.push(packed.badness);

        // §888: the box, then everything the packager took out of it.
        append_to_vlist(&mut out.list, just_box, prev_depth, params.baselines);
        out.list.append(&mut adjust);

        // §890: the penalty between this line and the next.
        if let Some(pen) = interline_penalty(i, total, disc_break, params.penalties) {
            out.list.push(Node::Penalty(pen));
        }

        // §879: prune the discardable nodes off the front of the next line,
        // unless the discretionary already put something there.
        if i + 1 < total && !post_disc_break {
            let stop = breaks[i + 1].min(nodes.len());
            while next < stop {
                let discardable = match nodes[next].as_ref() {
                    None => true,
                    Some(node) if node.is_char() => false,
                    Some(node) if node.non_discardable() => false,
                    // A kern is discarded only when it is EXPLICIT: an
                    // implicit one came from the font and belongs to the
                    // letters around it.
                    Some(Node::Kern { explicit, .. }) => *explicit,
                    Some(_) => true,
                };
                if !discardable {
                    break;
                }
                nodes[next].take();
                next += 1;
            }
        }
        pos = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glue::Glue;
    use crate::node::BoxNode;
    use crate::pack::glue_widths;

    fn pt(n: i64) -> Scaled {
        n * crate::dimen::UNITY
    }

    /// A word, as a box of a given width — enough for the packer, which asks
    /// nothing else of it.
    fn word(width: Scaled) -> Node {
        Node::Box(BoxNode {
            width,
            height: pt(7),
            depth: pt(2),
            ..BoxNode::null()
        })
    }

    /// Interword glue: plain TeX's cmr10 space, near enough.
    fn space() -> Node {
        Node::Glue(GlueNode::new(Glue {
            natural: pt(4),
            stretch: pt(2),
            shrink: pt(1),
            ..Glue::default()
        }))
    }

    /// `word space word space word space word`, breakable at each space.
    fn paragraph() -> Vec<Node> {
        let mut list = Vec::new();
        for i in 0..4 {
            if i > 0 {
                list.push(space());
            }
            list.push(word(pt(20)));
        }
        list
    }

    fn boxes(list: &[Node]) -> Vec<&BoxNode> {
        list.iter()
            .filter_map(|n| match n {
                Node::Box(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    /// §890: each line is packed to the width it was given, whatever its
    /// natural width was. Two words on a 60pt line is 44pt of material, and
    /// the line still comes out 60pt.
    #[test]
    fn every_line_is_packed_to_its_own_measure() {
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            paragraph(),
            &[3, usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let lines = boxes(&par.list);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].width, pt(60));
        assert_eq!(lines[1].width, pt(60));
        // The last line's depth is what `\prevdepth` is left at.
        assert_eq!(prev_depth, pt(2));
    }

    /// §881: the glue at the break is not deleted, it BECOMES `\rightskip`.
    /// With `\rightskip=0pt plus1fil` — `\raggedright` — the line's slack all
    /// goes to that last glue, and the interword space stays at its natural
    /// 4pt instead of stretching to fill the measure.
    #[test]
    fn the_glue_at_a_break_becomes_rightskip() {
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let ragged = ParParams {
            skips: Skips {
                left_skip: Glue::default(),
                right_skip: Glue {
                    natural: 0,
                    stretch: crate::dimen::UNITY,
                    stretch_order: 1,
                    ..Glue::default()
                },
            },
            ..ParParams::plain(pt(60))
        };
        let par = post_line_break(paragraph(), &[3, usize::MAX], &ragged, &mut prev_depth);
        let lines = boxes(&par.list);
        let widths = glue_widths(lines[0]);
        // Two glues on the line: the interword space and the `\rightskip`.
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0], pt(4), "the space did not stretch");
        assert_eq!(widths[1], pt(16), "the rightskip took all the slack");
        assert_eq!(lines[0].glue_order, 1);
    }

    /// §887: a line that does NOT end at glue still gets a `\rightskip` node,
    /// appended after the breakpoint. Breaking at a penalty leaves the penalty
    /// node inside the line box, which is where `tex` puts it.
    #[test]
    fn a_line_broken_at_a_penalty_keeps_the_penalty_and_gains_a_rightskip() {
        let mut list = vec![word(pt(20)), Node::Penalty(-500), word(pt(20))];
        list.insert(2, space());
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            list,
            &[1, usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let lines = boxes(&par.list);
        let first: Vec<u8> = lines[0].list.iter().map(Node::type_code).collect();
        // box, penalty, rightskip glue.
        assert_eq!(first, vec![0, 12, 10]);
        assert!(matches!(lines[0].list[1], Node::Penalty(-500)));
    }

    /// §879: the space that separated two words is thrown away when the line
    /// breaks there, so the second line starts flush rather than indented by a
    /// stray 4pt of glue.
    #[test]
    fn the_glue_at_the_break_does_not_start_the_next_line() {
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            paragraph(),
            &[3, usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let lines = boxes(&par.list);
        // The second line is `word space word rightskip`, not
        // `space word space word rightskip`.
        let second: Vec<u8> = lines[1].list.iter().map(Node::type_code).collect();
        assert_eq!(second, vec![0, 10, 0, 10]);
    }

    /// §882-§886: a discretionary break puts its pre-break list at the end of
    /// this line and its post-break list at the front of the next, and the
    /// `replace_count` nodes between them are destroyed.
    #[test]
    fn a_discretionary_break_transplants_both_of_its_lists() {
        let hyphen = word(pt(3));
        let replaced = word(pt(9));
        let list = vec![
            word(pt(20)),
            Node::Disc(DiscNode {
                pre_break: vec![hyphen],
                post_break: vec![word(pt(5))],
                replace_count: 1,
            }),
            replaced,
            word(pt(20)),
        ];
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            list,
            &[1, usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let lines = boxes(&par.list);
        // First line: the 20pt word, the emptied disc, the 3pt hyphen, the
        // rightskip. The 9pt replacement is gone.
        let first: Vec<Scaled> = boxes(&lines[0].list).iter().map(|b| b.width).collect();
        assert_eq!(first, vec![pt(20), pt(3)]);
        // Second line: the 5pt post-break, then the last word. It was NOT
        // pruned, because a post-break list suppresses the pruning (§877).
        let second: Vec<Scaled> = boxes(&lines[1].list).iter().map(|b| b.width).collect();
        assert_eq!(second, vec![pt(5), pt(20)]);
        // §890: a line ending at a discretionary is charged `\brokenpenalty`.
        let penalties: Vec<i64> = par
            .list
            .iter()
            .filter_map(|n| match n {
                Node::Penalty(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(
            penalties,
            vec![150 + 150 + 100],
            "a two-line paragraph is charged both club and widow, plus brokenpenalty"
        );
    }

    /// §888: `\vadjust` material comes OUT of the line box and lands on the
    /// vertical list right after it. A footnote reaches the page no other way.
    #[test]
    fn vadjust_material_leaves_the_line_and_follows_it() {
        let marker = Node::Box(BoxNode {
            width: pt(99),
            ..BoxNode::null()
        });
        let list = vec![
            word(pt(20)),
            Node::Adjust(vec![marker]),
            space(),
            word(pt(20)),
        ];
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            list,
            &[usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let lines = boxes(&par.list);
        // One line box, and the adjustment box beside it on the vlist.
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].width, pt(60));
        assert_eq!(lines[1].width, pt(99));
        // Inside the line there is no trace of it.
        assert!(boxes(&lines[0].list).iter().all(|b| b.width == pt(20)));
    }

    /// §679: the lines of a paragraph are set `\baselineskip` apart, measured
    /// baseline to baseline — so the glue between two 7pt-tall, 2pt-deep lines
    /// is 12 - 2 - 7 = 3pt, not 12pt.
    #[test]
    fn the_lines_come_out_a_baselineskip_apart() {
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let par = post_line_break(
            paragraph(),
            &[3, usize::MAX],
            &ParParams::plain(pt(60)),
            &mut prev_depth,
        );
        let interline: Vec<Scaled> = par
            .list
            .iter()
            .filter_map(|n| match n {
                Node::Glue(g) if g.source == GlueSource::BaselineSkip => Some(g.spec.natural),
                _ => None,
            })
            .collect();
        assert_eq!(interline, vec![pt(3)]);
    }

    /// §849: `\hangindent` with a negative `\hangafter` indents the FIRST
    /// lines; with a non-negative one it indents the rest.
    #[test]
    fn hanging_indentation_moves_the_lines_hangafter_names() {
        let shape = LineShape::hanging(pt(100), pt(20), -2);
        assert_eq!(shape.line(1), (pt(20), pt(80)));
        assert_eq!(shape.line(2), (pt(20), pt(80)));
        assert_eq!(shape.line(3), (0, pt(100)));

        let after = LineShape::hanging(pt(100), pt(20), 1);
        assert_eq!(after.line(1), (0, pt(100)));
        assert_eq!(after.line(2), (pt(20), pt(80)));

        // A negative `\hangindent` narrows from the right: the width drops,
        // the margin does not move.
        let right = LineShape::hanging(pt(100), pt(-20), 1);
        assert_eq!(right.line(2), (0, pt(80)));
    }

    /// §848: `\parshape` names each line, and its LAST pair governs every line
    /// past the ones it names.
    #[test]
    fn parshape_names_each_line_and_the_last_pair_governs_the_rest() {
        let shape = LineShape::par_shape(pt(100), vec![(pt(10), pt(90)), (pt(5), pt(95))]);
        assert_eq!(shape.line(1), (pt(10), pt(90)));
        assert_eq!(shape.line(2), (pt(5), pt(95)));
        assert_eq!(shape.line(3), (pt(5), pt(95)));
    }

    /// §890: the indent is a SHIFT on the line box, not a kern inside it, so
    /// the box still measures its full width and the material starts further
    /// in.
    #[test]
    fn the_indent_is_a_shift_and_not_part_of_the_line() {
        let mut prev_depth = crate::node::IGNORE_DEPTH;
        let params = ParParams {
            shape: LineShape::hanging(pt(60), pt(12), -1),
            ..ParParams::plain(pt(60))
        };
        let par = post_line_break(paragraph(), &[3, usize::MAX], &params, &mut prev_depth);
        let lines = boxes(&par.list);
        assert_eq!(lines[0].shift_amount, pt(12));
        assert_eq!(lines[0].width, pt(48));
        assert_eq!(lines[1].shift_amount, 0);
        assert_eq!(lines[1].width, pt(60));
    }
}
