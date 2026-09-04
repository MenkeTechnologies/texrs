//! The node-list shipper: a box tree drawn into a DVI file.
//!
//! `tex.web` §619-§640's `hlist_out`, `vlist_out` and `ship_out`. What they
//! have to get right is not the opcodes -- `src/dvi.rs` already wrote those --
//! but the ARITHMETIC between them, and it is invisible in a rendered page:
//!
//! - Glue is written at the width `hpack` SET it to, not at the width it was
//!   declared with. That is the whole reason a driver never has to be told to
//!   justify a line, and it is why the DVI path could not use a breaker that
//!   prices glue until this existed.
//! - A box inside a box is drawn relative to its parent and the position is
//!   RESTORED afterwards, so a nested box cannot drag what follows it.
//! - `cur_h`/`cur_v` and `dvi_h`/`dvi_v` differ until something is drawn, so a
//!   movement that is decided and then never used is never written.
//!
//! Every test here reads the op stream back out of the file, so the numbers
//! asserted are the numbers a driver will act on.

use texrs::dvi::{Dvi, Op, Writer};
use texrs::glue::Glue;
use texrs::node::{BoxNode, CharNode, GlueNode, LeaderKind, Node, RuleNode, Scaled};
use texrs::pack::{hpack, vpack, Spec, Tolerances, NATURAL};

/// One TeX point, in scaled points (`tex.web` §101: 2^16 sp to the point).
const PT: Scaled = 65536;

fn ch(character: char, width: Scaled, height: Scaled, depth: Scaled) -> Node {
    Node::Char(CharNode {
        font: 0,
        character,
        width,
        height,
        depth,
    })
}

fn glue(natural: Scaled, stretch: Scaled, shrink: Scaled) -> Node {
    Node::Glue(GlueNode::new(Glue {
        natural,
        stretch,
        stretch_order: 0,
        shrink,
        shrink_order: 0,
    }))
}

/// Ship one page and give back the ops between `bop` and `eop`, which is the
/// page itself with the preamble and postamble taken off.
fn shipped(page: &BoxNode) -> Vec<Op> {
    let mut w = Writer::new("texrs");
    w.define_font(0, "cmr10", 10 * PT as i32, 0, 10 * PT as i32);
    texrs::shipout::ship_out(&mut w, page, [1, 0, 0, 0, 0, 0, 0, 0, 0, 0], 0, 0);
    let dvi = Dvi::parse(&w.finish()).expect("the shipper writes a readable DVI");
    let mut ops = Vec::new();
    let mut on_page = false;
    for op in dvi.ops {
        match op {
            Op::BeginPage { .. } => on_page = true,
            Op::EndPage => on_page = false,
            op if on_page => ops.push(op),
            _ => {}
        }
    }
    ops
}

#[test]
fn glue_is_written_at_the_width_it_was_set_to_and_not_at_its_natural_width() {
    // `\hbox to 25pt{a\hskip 10pt plus 5pt b}` with two 5pt characters: the
    // natural width is 20pt and the box is 25pt, so the single glue takes all
    // 5pt of its stretch and is drawn 15pt wide.
    //
    // This is §625's `rule_wd:=width(g)-cur_g … rule_wd:=rule_wd+cur_g`, and
    // it is what makes a justified line possible in DVI at all: the driver is
    // told to move 15pt, and never learns that 10pt was asked for.
    let line = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            glue(10 * PT, 5 * PT, 0),
            ch('b', 5 * PT, PT, 0),
        ],
        Spec::Exactly(25 * PT),
        Tolerances::plain(),
        None,
    );
    assert_eq!(line.node.width, 25 * PT);
    assert_eq!(
        shipped(&line.node),
        vec![
            // The page's reference point is its baseline, one point below the
            // top because that is the box's height.
            Op::Down(PT as i32),
            Op::Font(0),
            Op::SetChar('a' as u32),
            Op::Right(15 * PT as i32),
            Op::SetChar('b' as u32),
        ]
    );
}

#[test]
fn glue_that_is_shrunk_is_written_narrower_than_it_asked_for() {
    // The other direction, and the one that stopped the DVI path from using a
    // real line breaker: a breaker that decides a line should be SHRUNK had
    // nowhere to put that answer. `\hbox to 18pt` over a natural 20pt shrinks
    // the glue by 2pt of the 3pt it offers, so it is drawn 8pt wide.
    let line = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            glue(10 * PT, 0, 3 * PT),
            ch('b', 5 * PT, PT, 0),
        ],
        Spec::Exactly(18 * PT),
        Tolerances::plain(),
        None,
    );
    let ops = shipped(&line.node);
    assert!(
        ops.contains(&Op::Right(8 * PT as i32)),
        "the glue is drawn at 8pt, not at 10pt: {ops:?}"
    );
}

#[test]
fn a_glue_of_the_wrong_order_does_not_move_when_the_box_is_stretched() {
    // §625 only accumulates glue whose ORDER matches the box's. A `\hfil` in a
    // box being stretched at order 0 would be the only thing that moves, and a
    // finite glue beside it must not move at all -- which is the whole of
    // `\hfil`'s behaviour, and would be invisible in a test that only checked
    // that the line came out the right width.
    let line = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            glue(2 * PT, 4 * PT, 0),
            ch('b', 5 * PT, PT, 0),
            Node::Glue(GlueNode::new(Glue {
                natural: 0,
                stretch: PT,
                stretch_order: 1,
                shrink: 0,
                shrink_order: 0,
            })),
            ch('c', 5 * PT, PT, 0),
        ],
        Spec::Exactly(20 * PT),
        Tolerances::plain(),
        None,
    );
    // The finite glue keeps its declared 2pt though it offers 4pt of stretch;
    // the `fil` takes the whole 3pt of slack.
    assert_eq!(
        shipped(&line.node),
        vec![
            Op::Down(PT as i32),
            Op::Font(0),
            Op::SetChar('a' as u32),
            Op::Right(2 * PT as i32),
            Op::SetChar('b' as u32),
            Op::Right(3 * PT as i32),
            Op::SetChar('c' as u32),
        ]
    );
}

#[test]
fn a_vertical_list_puts_each_box_a_baseline_further_down_and_at_one_left_edge() {
    // §629's invariant: a vlist keeps `cur_h = left_edge`, so nothing in it
    // drifts sideways however wide the boxes are, and only `cur_v` moves.
    let a = hpack(vec![ch('a', 5 * PT, PT, 0)], NATURAL, Tolerances::plain(), None);
    let b = hpack(
        vec![ch('b', 40 * PT, PT, 0)],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    let page = vpack(
        vec![
            Node::Kern {
                width: 11 * PT,
                explicit: false,
            },
            Node::Box(a.node),
            Node::Kern {
                width: 11 * PT,
                explicit: false,
            },
            Node::Box(b.node),
        ],
        NATURAL,
        Tolerances::plain(),
    );
    let ops = shipped(&page.node);
    assert_eq!(
        ops,
        vec![
            // Each line's own box pushes, draws, and pops, so the second one
            // starts from the left edge again rather than from where the first
            // ended -- which for a 40pt box is the whole point.
            Op::Down(12 * PT as i32),
            Op::Push,
            Op::Font(0),
            Op::SetChar('a' as u32),
            Op::Pop,
            Op::Down(12 * PT as i32),
            Op::Push,
            Op::SetChar('b' as u32),
            Op::Pop,
        ],
        "twelve points apart, both at the left edge"
    );
}

#[test]
fn a_box_inside_a_box_does_not_drag_what_follows_it() {
    // §623: `cur_h:=edge+width(p)` after the recursion, from the edge the
    // sub-box STARTED at -- not from wherever it happened to leave the
    // cursor. A sub-box whose contents are narrower than its declared width
    // is what tells the two apart.
    let inner = hpack(
        vec![ch('x', 2 * PT, PT, 0)],
        Spec::Exactly(20 * PT),
        Tolerances::plain(),
        None,
    );
    let outer = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            Node::Box(inner.node),
            ch('b', 5 * PT, PT, 0),
        ],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    assert_eq!(outer.node.width, 30 * PT, "5 + 20 + 5");
    let ops = shipped(&outer.node);
    assert_eq!(
        ops,
        vec![
            Op::Down(PT as i32),
            Op::Font(0),
            Op::SetChar('a' as u32),
            Op::Push,
            Op::SetChar('x' as u32),
            Op::Pop,
            // `pop` puts the reader back at 5pt, where the sub-box began, so
            // the move to 25pt -- past the sub-box's DECLARED width, not past
            // the 2pt it drew -- is 20pt.
            Op::Right(20 * PT as i32),
            Op::SetChar('b' as u32),
        ],
        "the `b` sits after the sub-box's declared width"
    );
}

#[test]
fn a_rule_in_an_hlist_takes_the_enclosing_boxs_height_when_its_own_is_running() {
    // §624: `\vrule` is written with a running height and depth, and it comes
    // out as tall as whatever box it lands in. A shipper that wrote the
    // running value itself would emit a rule 2^30 sp tall.
    let line = hpack(
        vec![
            ch('a', 5 * PT, 7 * PT, 2 * PT),
            Node::Rule(RuleNode {
                width: PT / 2,
                height: texrs::node::NULL_FLAG,
                depth: texrs::node::NULL_FLAG,
            }),
        ],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    let ops = shipped(&line.node);
    assert!(
        ops.contains(&Op::Rule {
            // 7pt of height and 2pt of depth: the rule is drawn from the
            // bottom of the box's depth upward through its whole height.
            height: 9 * PT as i32,
            width: (PT / 2) as i32,
            set: true,
        }),
        "the rule took the box's own height and depth: {ops:?}"
    );
    // And it was drawn from the FOOT of the line: the reader sat on the
    // baseline and was moved down by the box's depth before the rule went in,
    // because a DVI rule grows upward from where it is placed (§585).
    let at = ops
        .iter()
        .position(|op| matches!(op, Op::Rule { .. }))
        .expect("the rule reached the file");
    assert_eq!(
        ops[at - 1],
        Op::Down(2 * PT as i32),
        "moved to the foot of the line first: {ops:?}"
    );
}

#[test]
fn a_rule_in_a_vertical_list_spans_the_boxs_width_and_does_not_move_the_reader() {
    // §633: a rule in a vlist is a `put_rule` -- the list's own arithmetic has
    // already accounted for its height, so the reader must not be advanced by
    // it a second time. A running WIDTH takes the enclosing box's, which is
    // what `\hrule` in a `\vbox` does.
    let wide = hpack(
        vec![ch('a', 30 * PT, PT, 0)],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    let page = vpack(
        vec![
            Node::Box(wide.node),
            Node::Rule(RuleNode {
                width: texrs::node::NULL_FLAG,
                height: PT / 2,
                depth: 0,
            }),
        ],
        NATURAL,
        Tolerances::plain(),
    );
    let ops = shipped(&page.node);
    assert!(
        ops.contains(&Op::Rule {
            height: (PT / 2) as i32,
            width: 30 * PT as i32,
            set: false,
        }),
        "a put_rule as wide as the box: {ops:?}"
    );
}

#[test]
fn leaders_replicate_a_box_across_the_glue_they_are_set_in() {
    // §626-§628. A table of contents' dots are leaders, and the count is not a
    // property of the glue node: it is how many copies fit in the width the
    // glue was SET to. Five 4pt copies fit in 20pt.
    let dot = hpack(vec![ch('.', 4 * PT, PT, 0)], NATURAL, Tolerances::plain(), None);
    let line = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            Node::Glue(GlueNode {
                spec: Glue {
                    natural: 0,
                    stretch: PT,
                    stretch_order: 1,
                    shrink: 0,
                    shrink_order: 0,
                },
                kind: LeaderKind::Centred,
                source: Default::default(),
                leader: Some(Box::new(Node::Box(dot.node))),
            }),
            ch('b', 5 * PT, PT, 0),
        ],
        Spec::Exactly(30 * PT),
        Tolerances::plain(),
        None,
    );
    let ops = shipped(&line.node);
    let dots = ops
        .iter()
        .filter(|op| **op == Op::SetChar('.' as u32))
        .count();
    assert_eq!(dots, 5, "twenty points of leaders at 4pt each: {ops:?}");
    // And the character after them is still where the box says it is: the
    // whole line is 30pt, so `b` starts 25pt in.
    assert_eq!(
        ops.last(),
        Some(&Op::SetChar('b' as u32)),
        "the leaders did not swallow what follows: {ops:?}"
    );
}

#[test]
fn a_special_is_written_where_it_stands() {
    // §1368's `special_out` synchronises BOTH coordinates first, because a
    // `\special` is a message about a place. A colour push written before the
    // reader has been moved colours the wrong part of the page.
    let line = hpack(
        vec![
            ch('a', 5 * PT, PT, 0),
            glue(10 * PT, 0, 0),
            Node::Whatsit("color push rgb 1 0 0".to_string()),
            ch('b', 5 * PT, PT, 0),
        ],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    let ops = shipped(&line.node);
    let at = ops
        .iter()
        .position(|op| matches!(op, Op::Special(_)))
        .expect("the special reached the file");
    assert_eq!(
        ops[at - 1],
        Op::Right(10 * PT as i32),
        "the glue was written before the special, not after: {ops:?}"
    );
    assert_eq!(ops[at + 1], Op::SetChar('b' as u32));
}

#[test]
fn nothing_is_written_for_a_movement_that_is_never_used() {
    // §616's whole purpose. Glue at the end of a line -- `\rightskip`, or the
    // space `post_line_break` turns into one -- moves `cur_h` and is then
    // never drawn at, so `dvi_h` never has to catch up and the file carries no
    // command for it. A shipper that emitted a movement per glue node would
    // write one at the end of every line of every paragraph.
    let line = hpack(
        vec![ch('a', 5 * PT, PT, 0), glue(100 * PT, 0, 0)],
        NATURAL,
        Tolerances::plain(),
        None,
    );
    let ops = shipped(&line.node);
    assert!(
        !ops.iter().any(|op| matches!(op, Op::Right(_))),
        "the trailing glue was written even though nothing follows it: {ops:?}"
    );
}
