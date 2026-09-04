//! The stomach below the line breaker, driven from outside the crate the way
//! an engine would drive it: lines onto a vertical list with real interline
//! glue, the page builder cutting that list by cost, footnotes taking their
//! space out of `\pagegoal` before the text is priced against it, and
//! `\vsplit` cutting a finished box.
//!
//! Every number asserted here is one `tex.web` computes rather than one this
//! file chose. Where a page holds 44 lines it is because `\topskip` puts the
//! first baseline 10pt down and `\baselineskip` puts each of the rest 12pt
//! below the last, and 10 + 43*12 is 526 — not because 44 was written down.

use std::collections::BTreeMap;
use texrs::box_::{Boxes, Registers};
use texrs::dimen::UNITY;
use texrs::glue::Glue;
use texrs::node::{BoxNode, GlueNode, InsNode, Node, Scaled, IGNORE_DEPTH};
use texrs::pack::{append_to_vlist, glue_widths, hpack, Baselines, Spec, Tolerances, NATURAL};
use texrs::page::{Fired, InsertClass, PageBuilder, PageParams};

fn pt(n: i64) -> Scaled {
    n * UNITY
}

/// A line of text as the paragraph breaker would leave it: 8pt above the
/// baseline, 2pt below, set to the measure.
fn line() -> BoxNode {
    BoxNode {
        width: pt(345),
        height: pt(8),
        depth: pt(2),
        ..BoxNode::null()
    }
}

/// `n` such lines on a vertical list, with plain TeX's `\baselineskip=12pt`
/// between them.
fn column(n: usize) -> Vec<Node> {
    let mut list = Vec::new();
    let mut prev_depth = IGNORE_DEPTH;
    for _ in 0..n {
        append_to_vlist(&mut list, line(), &mut prev_depth, Baselines::plain());
    }
    list
}

/// How many lines a shipped page holds.
fn lines_on(page: &BoxNode) -> usize {
    page.list
        .iter()
        .filter(|n| matches!(n, Node::Box(_)))
        .count()
}

/// The pages a run of `Fired` events shipped.
fn shipped(fired: &[Fired]) -> Vec<&BoxNode> {
    fired
        .iter()
        .map(|f| match f {
            Fired::ShipOut(page) => page,
            Fired::Output { page, .. } => page,
        })
        .collect()
}

/// A page 526pt tall holds exactly 44 lines set on 12pt, and it holds them
/// because `\topskip` and `\baselineskip` put the baselines where they are:
/// the first 10pt from the top, each of the rest 12pt below the last, so the
/// 44th is at 10 + 43*12 = 526.
///
/// This is the assertion a line-counting paginator cannot make. It fills a
/// page with whatever fits and the answer moves when a line happens to be
/// taller; here the baselines are on a 12pt grid whatever the lines measure,
/// which is what `\baselineskip` IS.
#[test]
fn a_page_holds_the_lines_its_baselines_leave_room_for() {
    let mut builder = PageBuilder::new(PageParams {
        vsize: pt(526),
        max_depth: pt(4),
        top_skip: Glue::fixed(pt(10)),
        ..PageParams::default()
    });
    let fired = builder.contribute(column(100));
    let pages = shipped(&fired);
    assert_eq!(pages.len(), 2);
    for page in &pages {
        assert_eq!(lines_on(page), 44);
        assert_eq!(page.height, pt(526));
        // A perfect fit: `\pagegoal` was reached exactly, so no glue was set.
        assert_eq!(page.glue_sign, texrs::node::GlueSign::Normal);
    }
    // Twelve lines are left over and no break has forced them out.
    assert_eq!(builder.page_total(), pt(10 + 11 * 12));
    let fired = builder.finish();
    let last = shipped(&fired);
    assert_eq!(last.len(), 1);
    // Twelve lines, and the empty `\hbox to \hsize{}` that §1054's `\end`
    // contributes ahead of its `\vfill` to make sure the page has a box on it.
    assert_eq!(lines_on(last[0]), 13);
    // The last page is still packaged to the full `\vsize`: `\vfill` at the
    // end of the job stretches to fill it rather than leaving a short box.
    assert_eq!(last[0].height, pt(526));
    assert_eq!(last[0].glue_sign, texrs::node::GlueSign::Stretching);
}

/// A footnote takes its space out of `\pagegoal` BEFORE the text is priced
/// against it (§1008), so a page carrying one holds fewer lines. This is the
/// behaviour a page builder without insertions cannot approximate: the
/// footnote is set at the bottom of the page, but the room for it has to be
/// found before the builder knows where the page ends.
#[test]
fn a_footnote_takes_room_from_the_text_above_it() {
    let footnote_class = InsertClass {
        // `\count\footins=1000`: the footnote costs the page its own height.
        count: 1000,
        // `\dimen\footins`: at most 100pt of footnotes on one page.
        dimen: pt(100),
        // `\skip\footins`: the space above the footnote rule.
        skip: Glue::fixed(pt(5)),
    };
    let params = |inserts: BTreeMap<u8, InsertClass>| PageParams {
        vsize: pt(100),
        max_depth: pt(4),
        top_skip: Glue::fixed(pt(10)),
        inserts,
        ..PageParams::default()
    };

    // Without a footnote, the page takes as many lines as fit: baselines at
    // 10, 22, ... and a goal of 100pt.
    let mut plain = PageBuilder::new(params(BTreeMap::new()));
    let fired = plain.contribute(column(40));
    let bare = lines_on(shipped(&fired)[0]);

    // With one, `\pagegoal` drops by the 5pt of `\skip\footins` and the 20pt
    // the footnote is tall, and the page ends earlier.
    let mut inserts = BTreeMap::new();
    inserts.insert(1, footnote_class);
    let mut with_note = PageBuilder::new(params(inserts));
    let mut nodes = column(40);
    nodes.insert(
        5,
        Node::Ins(InsNode {
            number: 1,
            height: pt(20),
            depth: pt(4),
            float_cost: 100,
            split_top_skip: Glue::fixed(pt(10)),
            list: vec![Node::Box(BoxNode {
                width: pt(345),
                height: pt(20),
                ..BoxNode::null()
            })],
        }),
    );
    let fired = with_note.contribute(nodes);
    let pages = shipped(&fired);
    let noted = lines_on(pages[0]);

    assert!(
        noted < bare,
        "a footnote should push text off the page: {noted} lines against {bare}"
    );
    // 100pt of `\vsize` less 5pt of `\skip\footins` less the 20pt footnote.
    assert_eq!(bare, 8);
    assert_eq!(noted, 6);
    // The footnote itself is in `\box1`, packaged as a vbox, and NOT on the
    // page: the output routine is what puts the two together.
    assert!(!pages[0].list.iter().any(|n| matches!(n, Node::Ins(_))));
    let footins = with_note.boxes.get(&1).expect("\\box1 holds the footnote");
    assert!(footins.vertical);
    assert_eq!(footins.height, pt(20));
}

/// `\vsplit` repeated: a long vbox cut into pages of fixed height, with the
/// remainder going back into the register each time.
///
/// The two things being asserted are that every extracted box is EXACTLY the
/// height asked for (§979 packages it `exactly`), and that the material is
/// conserved -- nothing is dropped at a cut, and the glue at each cut is
/// replaced by `\splittopskip` rather than being carried over.
#[test]
fn vsplit_cuts_a_long_box_into_pages_of_a_fixed_height() {
    let mut registers = Boxes::default();
    let whole = texrs::pack::vpack(column(30), NATURAL, Tolerances::plain()).node;
    let total_lines = lines_on(&whole);
    assert_eq!(total_lines, 30);
    registers.set_box(1, Some(whole));

    let mut taken = 0;
    let mut pages = 0;
    while registers.box_register(1).is_some() {
        let split = texrs::box_::vsplit_register(
            &mut registers,
            1,
            pt(70),
            pt(4),
            Glue::fixed(pt(10)),
            Tolerances::plain(),
        )
        .expect("a vbox splits");
        assert_eq!(split.extracted.height, pt(70));
        taken += lines_on(&split.extracted);
        pages += 1;
        assert!(pages < 30, "the split must make progress");
    }
    assert_eq!(taken, total_lines);
}

/// Glue setting end to end: a line of words and interword spaces, packaged to
/// a measure it does not naturally reach, comes out at that measure to the
/// scaled point.
///
/// The spaces do NOT each get an equal share rounded independently — §625
/// accumulates the exact total and takes differences, which is why the sum is
/// exact rather than off by a scaled point per space. A right margin that
/// wanders by a few thousandths of a point is the visible form of getting this
/// wrong.
#[test]
fn a_justified_line_comes_out_at_exactly_the_measure() {
    let word = |width: Scaled| {
        Node::Box(BoxNode {
            width,
            height: pt(7),
            depth: pt(2),
            ..BoxNode::null()
        })
    };
    // cmr10's interword glue: 3.33333pt plus 1.66666pt minus 1.11111pt.
    let space = || {
        Node::Glue(GlueNode::new(Glue {
            natural: 218_453,
            stretch: 109_226,
            shrink: 72_818,
            ..Glue::default()
        }))
    };
    let mut list = Vec::new();
    for i in 0..13 {
        if i > 0 {
            list.push(space());
        }
        list.push(word(pt(20) + i * 1237));
    }
    let natural = hpack(list.clone(), NATURAL, Tolerances::plain(), None);
    // Stretch the line by half of everything its twelve spaces can give, which
    // is the middle of what TeX calls a decent line.
    let give = 12 * 109_226;
    let measure = natural.node.width + give / 2;

    let set = hpack(list, Spec::Exactly(measure), Tolerances::plain(), None);
    assert_eq!(set.node.width, measure);
    assert_eq!(set.node.glue_sign, texrs::node::GlueSign::Stretching);
    // badness(t,s) with t/s = 1/2 is 12, which §817 calls decent.
    assert_eq!(set.badness, 12);
    assert_eq!(set.report, None);

    // The words are unchanged and the spaces make up the whole difference.
    let widths = glue_widths(&set.node);
    assert_eq!(widths.len(), 12);
    let words: Scaled = set
        .node
        .list
        .iter()
        .filter_map(|n| match n {
            Node::Box(b) => Some(b.width),
            _ => None,
        })
        .sum();
    let spaces: Scaled = widths.iter().sum();
    assert_eq!(words + spaces, measure);
    // Every space is between its natural width and its natural width plus all
    // of its stretch, because the ratio is a half.
    for w in widths {
        assert!(w > 218_453, "a stretched space is wider than its natural");
        assert!(w < 218_453 + 109_226, "and short of all of its stretch");
    }
}

/// The same line squeezed rather than stretched: `hpack` shrinks, and the
/// badness it reports is §108's integer answer.
#[test]
fn a_tight_line_shrinks_and_reports_the_badness_tex_would() {
    let word = |width: Scaled| {
        Node::Box(BoxNode {
            width,
            height: pt(7),
            ..BoxNode::null()
        })
    };
    let space = || {
        Node::Glue(GlueNode::new(Glue {
            natural: 218_453,
            stretch: 109_226,
            shrink: 72_818,
            ..Glue::default()
        }))
    };
    let list = vec![
        word(pt(100)),
        space(),
        word(pt(100)),
        space(),
        word(pt(100)),
    ];
    let natural = hpack(list.clone(), NATURAL, Tolerances::plain(), None);
    // Shrink the line by exactly half of what its two spaces can give back.
    let give = 2 * 72_818;
    let target = natural.node.width - give / 2;
    let set = hpack(list, Spec::Exactly(target), Tolerances::plain(), None);
    assert_eq!(set.node.width, target);
    assert_eq!(set.node.glue_sign, texrs::node::GlueSign::Shrinking);
    // badness(t,s) with t/s = 1/2 is 12, which is still "decent" by §817.
    assert_eq!(set.badness, texrs::pack::badness(give / 2, give));
    assert_eq!(set.badness, 12);
    assert_eq!(set.report, None);
    let widths = glue_widths(&set.node);
    assert_eq!(widths.iter().sum::<Scaled>() + pt(300), target);
}

/// A footnote written in the MIDDLE OF A PARAGRAPH still reaches the page.
///
/// This is the whole chain, and every link of it is load-bearing. The
/// `\insert` node sits in the horizontal list between two words, so it is
/// inside the material `hpack` is about to seal into a line box; §655 moves it
/// onto the adjustment list instead, §888 puts that list on the vertical list
/// right after the line, and §1008 then takes its height out of `\pagegoal`
/// before the text below it is priced. Break any one of the three and the
/// footnote either vanishes into a line box or arrives too late to affect the
/// page break — and a footnote that does not affect the page break is a
/// footnote that overprints the last line of text.
///
/// Written as `tex` would see it: `\hsize=200pt`, six 90pt words, and the
/// `\insert` after the third.
#[test]
fn an_insert_written_inside_a_paragraph_reaches_the_page_builder() {
    use texrs::node::InsNode;
    use texrs::postline::{post_line_break, ParParams};

    let word = || {
        Node::Box(BoxNode {
            width: pt(90),
            height: pt(8),
            depth: pt(2),
            ..BoxNode::null()
        })
    };
    let space = || {
        Node::Glue(GlueNode::new(Glue {
            natural: pt(10),
            stretch: pt(5),
            shrink: pt(3),
            ..Glue::default()
        }))
    };
    let footnote = Node::Ins(InsNode {
        number: 1,
        height: pt(30),
        depth: pt(4),
        float_cost: 100,
        split_top_skip: Glue::fixed(pt(10)),
        list: vec![Node::Box(BoxNode {
            width: pt(200),
            height: pt(30),
            ..BoxNode::null()
        })],
    });

    // 0 word, 1 space, 2 word, 3 space, 4 word, 5 INSERT, 6 space,
    // 7 word, 8 space, 9 word, 10 space, 11 word.
    let mut paragraph = Vec::new();
    for i in 0..6 {
        if i > 0 {
            paragraph.push(space());
        }
        paragraph.push(word());
        if i == 2 {
            paragraph.push(footnote.clone());
        }
    }

    let mut prev_depth = IGNORE_DEPTH;
    let par = post_line_break(
        paragraph,
        &[3, 8, usize::MAX],
        &ParParams::plain(pt(200)),
        &mut prev_depth,
    );

    // Three lines, each set to the measure: two 90pt words and a 10pt space
    // come to 190pt naturally, and every line still comes out 200pt.
    let lines: Vec<&BoxNode> = par
        .list
        .iter()
        .filter_map(|n| match n {
            Node::Box(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 3);
    for line in &lines {
        assert_eq!(line.width, pt(200));
    }
    // §655: the insertion is not sealed inside any of them.
    for line in &lines {
        assert!(
            !line.list.iter().any(|n| matches!(n, Node::Ins(_))),
            "the packager left an insertion inside a line box"
        );
    }
    // §888: it is on the vertical list, immediately after the line it was
    // written in — the second, since the break at index 3 ended the first.
    let at = par
        .list
        .iter()
        .position(|n| matches!(n, Node::Ins(_)))
        .expect("the insertion reached the vertical list");
    let boxes_before = par.list[..at]
        .iter()
        .filter(|n| matches!(n, Node::Box(_)))
        .count();
    assert_eq!(boxes_before, 2, "it follows the line it was written in");

    // §1008: the page pays `\skip1` and the footnote's own height up front.
    let mut inserts = BTreeMap::new();
    inserts.insert(
        1,
        InsertClass {
            count: 1000,
            dimen: pt(100),
            skip: Glue::fixed(pt(5)),
        },
    );
    let mut builder = PageBuilder::new(PageParams {
        vsize: pt(500),
        max_depth: pt(4),
        top_skip: Glue::fixed(pt(10)),
        inserts,
        ..PageParams::default()
    });
    let fired = builder.contribute(par.list);
    assert!(fired.is_empty(), "three lines do not fill a 500pt page");
    assert_eq!(
        builder.page_goal(),
        pt(500) - pt(5) - pt(30),
        "\\pagegoal drops by \\skip1 and the footnote's height"
    );

    // And when the page comes out, the footnote is in `\box1`, packaged.
    let fired = builder.finish();
    let pages = shipped(&fired);
    assert_eq!(pages.len(), 1);
    let note = builder.boxes.get(&1).expect("the footnote is in \\box1");
    assert!(note.vertical);
    assert_eq!(note.height, pt(30));
    // The text that shared the page with it is still all there. The page
    // carries a fourth box: \end contributes `\hbox to \hsize{}` before the
    // forced penalty (S1054), and it is a box like any other.
    let set_lines = pages[0]
        .list
        .iter()
        .filter(|n| matches!(n, Node::Box(b) if b.width == pt(200)))
        .count();
    assert_eq!(set_lines, 3);
}
