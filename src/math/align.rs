//! Columns in a display, and the equation number beside it.
//!
//! `tex.web` §768-§812 and §1204-§1206.
//!
//! ## What is ported, and what is not
//!
//! `&` is `\halign`'s alignment tab, and §768-§800 is the whole of `\halign`:
//! a PREAMBLE of templates `u_j # v_j`, an `align_peek` that reads a row at a
//! time, `unset_node`s that record each entry's natural width, span nodes for
//! `\span` and `\omit`, and a `fin_align` (§800-§812) that decides the column
//! widths and then sets every unset box to them.
//!
//! None of that preamble reaches this engine. `\halign` is not a command a
//! formula can contain here, and the environments that DO put an `&` in a
//! formula -- amsmath's `align`, LaTeX's `eqnarray` -- write their preamble in
//! package macros this port does not run: `\align@preamble`
//! (amsmath.sty:2358-2370) and the three-column preamble inside `\eqnarray`
//! (latex.ltx:15977-15980). Porting the general machinery would therefore
//! give a `\halign` nothing can call, and would still leave `\begin{align}`
//! setting one row, because the preamble it needs would not be there.
//!
//! So what is ported is the part that decides what a reader sees: §810's rule
//! that a column is as wide as its widest entry, and §1204-§1206's arithmetic
//! for where an equation number goes. The two preambles above are read as
//! tables of column ALIGNMENTS rather than as templates, which is all they
//! say once the `\displaystyle` and the `\hfil`s are separated out.
//!
//! ## What that costs, exactly
//!
//! - `\span`, `\omit`, `\noalign`, `\multispan` and `\tabskip` do nothing:
//!   they are preamble machinery and there is no preamble.
//! - Each row is set as a display line of its own rather than as one vertical
//!   box, because `src/typeset.rs` steps a line by the leading and not by a
//!   formula's height -- a two-row box would be drawn over the line above it.
//!   The rows still line up, because every row is packed to the same width and
//!   centred by the same rule; what they do not share is the SPACING between
//!   them, which comes out as the space between two displays rather than as
//!   one `\baselineskip`.

use super::font::MathFonts;
use super::mlist;
use super::noad::{Atom, Class, Field, Noad, Scaled, Style};
use crate::glue::Glue;
use crate::node::{BoxNode, GlueNode, Node};
use crate::pack::{hpack, Spec, Tolerances, NATURAL};

/// Where an entry sits inside its column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    /// `\hfil#`: flush right, which is what the first column of an `align` and
    /// of an `eqnarray` is.
    Right,
    /// `\hfil#\hfil`.
    Centre,
    /// `#\hfil`.
    Left,
}

/// A display's columns: how each is aligned, what goes in front of it, and
/// how much room stands before it.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub align: Align,
    /// The space in front of this column, in scaled points.
    pub gap: Scaled,
    /// Whether the column's entry begins with an empty Ord.
    ///
    /// amsmath writes its even columns `{{}##}` (amsmath.sty:2365) so that a
    /// row written `a &= b` sets the `=` as a RELATION: §728 turns a Bin or a
    /// Rel with nothing in front of it into an Ord, and the empty group is the
    /// something.
    pub empty_ord: bool,
}

/// `\arraycolsep = 5pt` (article.cls:443); `eqnarray` puts two of them in
/// front of its second and third columns (latex.ltx:15978-15980).
const ARRAY_COL_SEP: Scaled = 5 * 65536;

/// `\minalignsep = 10pt` (amsmath.sty:1316): the room between one `rl` pair of
/// an `align` and the next.
const MIN_ALIGN_SEP: Scaled = 10 * 65536;

/// The columns an environment sets, for a row of `n` of them.
///
/// `align` and its siblings repeat amsmath's `rl` pair (amsmath.sty:2358-2370)
/// and `eqnarray` is latex.ltx's `rcl` (latex.ltx:15977-15980). Anything else
/// -- `gather`, `multline`, a `$$…&…$$` written by hand -- has no preamble at
/// all and gets centred columns, which is what a display with no alignment is.
pub fn columns(environment: &str, n: usize) -> Vec<Column> {
    let pattern: &[Align] = match environment {
        "align" | "align*" | "aligned" | "alignat" | "alignat*" | "flalign" | "flalign*" => {
            &[Align::Right, Align::Left]
        }
        "eqnarray" | "eqnarray*" => &[Align::Right, Align::Centre, Align::Left],
        _ => &[Align::Centre],
    };
    let pairs = matches!(pattern, [Align::Right, Align::Left]);
    (0..n)
        .map(|j| {
            let align = pattern[j % pattern.len()];
            Column {
                align,
                gap: match (j, pairs) {
                    (0, _) => 0,
                    // A new `rl` pair, which is where `\alignsep@` goes.
                    (_, true) if j % 2 == 0 => MIN_ALIGN_SEP,
                    (_, true) => 0,
                    // `eqnarray` writes `\hskip 2\arraycolsep` in front of its
                    // second and third columns.
                    _ => 2 * ARRAY_COL_SEP,
                },
                empty_ord: align == Align::Left,
            }
        })
        .collect()
}

/// `\hfil` — the glue that pushes an entry to one side of its column.
fn fil() -> Node {
    Node::Glue(GlueNode::new(Glue {
        natural: 0,
        stretch: 65536,
        stretch_order: 1,
        shrink: 0,
        shrink_order: 0,
    }))
}

/// Every row of a display, each packed to the same width.
///
/// §810: "the |width| of column |j| is the maximum of the natural widths of
/// its entries". The entries are converted first, their widths compared, and
/// then each is packed to its column's width with the `\hfil`s its preamble
/// asks for -- which is what `fin_align` does to the unset boxes, done with
/// `hpack` rather than with a second copy of §810's box-fixing loop.
pub fn rows(
    fonts: &MathFonts,
    rows: &[Vec<Vec<Noad>>],
    style: Style,
    columns: &[Column],
) -> Vec<BoxNode> {
    // Every entry, converted once: a column's width cannot be known until
    // every row has been measured, and measuring means setting.
    let entries: Vec<Vec<BoxNode>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(j, cell)| {
                    let empty = columns.get(j).map(|c| c.empty_ord).unwrap_or(false);
                    let mut list = Vec::with_capacity(cell.len() + 1);
                    if empty {
                        list.push(Noad::Atom(Atom::new(Class::Ord, Field::List(Vec::new()))));
                    }
                    list.extend(cell.iter().cloned());
                    mlist::set(fonts, &list, style)
                })
                .collect()
        })
        .collect();
    let count = entries.iter().map(|r| r.len()).max().unwrap_or(0);
    let widths: Vec<Scaled> = (0..count)
        .map(|j| {
            entries
                .iter()
                .filter_map(|row| row.get(j))
                .map(|b| b.width)
                .max()
                .unwrap_or(0)
        })
        .collect();
    entries
        .iter()
        .map(|row| {
            let mut list: Vec<Node> = Vec::new();
            for (j, width) in widths.iter().enumerate() {
                let column = columns.get(j).copied().unwrap_or(Column {
                    align: Align::Centre,
                    gap: 0,
                    empty_ord: false,
                });
                if column.gap != 0 {
                    list.push(Node::Kern {
                        width: column.gap,
                        explicit: true,
                    });
                }
                let entry = row.get(j).cloned().unwrap_or_else(BoxNode::null);
                list.push(Node::Box(in_column(entry, *width, column.align)));
            }
            hpack(list, NATURAL, Tolerances::plain(), None).node
        })
        .collect()
}

/// One entry, packed to its column's width on the side its preamble names.
fn in_column(entry: BoxNode, width: Scaled, align: Align) -> BoxNode {
    let inner = vec![Node::Box(entry)];
    let list = match align {
        Align::Right => {
            let mut l = vec![fil()];
            l.extend(inner);
            l
        }
        Align::Left => {
            let mut l = inner;
            l.push(fil());
            l
        }
        Align::Centre => {
            let mut l = vec![fil()];
            l.extend(inner);
            l.push(fil());
            l
        }
    };
    hpack(list, Spec::Exactly(width), Tolerances::plain(), None).node
}

/// "Finish displayed math" (§1204-§1206): one display line, `z` wide, with its
/// equation number beside it.
///
/// `z` is `\displaywidth`, `w` the formula's own width and `e` the number's.
/// The formula is centred on `z` unless the number would come within its own
/// width of it, and the number then sits hard against the right edge -- or the
/// left edge for a `\leqno` (§1206's `l`).
///
/// The result is a box exactly `z` wide, so setting it at the left margin and
/// centring it on a measure of `z` are the same placement. That is why this
/// travels the same centred line every other display does.
pub fn numbered(
    fonts: &MathFonts,
    body: BoxNode,
    number: BoxNode,
    z: Scaled,
    left: bool,
) -> BoxNode {
    let mut body = body;
    let mut w = body.width;
    let mut e = number.width;
    // §1204: `q` is the number's width plus one quad, the room the display may
    // not run into.
    let q = match e {
        0 => 0,
        _ => e + fonts.math_quad(super::font::TEXT_SIZE),
    };
    // §1205: "Squeeze the equation as much as possible". There is no shrink to
    // report from here, so the branch taken is the one for a formula that
    // cannot be squeezed: the number goes on a line of its own, which is what
    // `e := 0` means, and the formula is set to the measure if it overruns it.
    if w + q > z {
        e = 0;
        if w > z {
            body = hpack(
                vec![Node::Box(body)],
                Spec::Exactly(z),
                Tolerances::plain(),
                None,
            )
            .node;
            w = body.width;
        }
    }
    // §1206: the displacement of the left edge of the equation.
    let mut d = half(z - w);
    if e > 0 && d < 2 * e {
        d = half(z - w - e);
    }
    let mut list: Vec<Node> = Vec::new();
    match (e != 0, left) {
        // §1206: `link(a):=r; link(r):=b; b:=a; d:=0` -- a `\leqno` puts the
        // number first and the display then starts where the kern leaves it.
        (true, true) => {
            list.push(Node::Box(number));
            list.push(Node::Kern {
                width: z - w - e - d,
                explicit: true,
            });
            list.push(Node::Box(body));
        }
        (true, false) => {
            list.push(Node::Kern {
                width: d,
                explicit: true,
            });
            list.push(Node::Box(body));
            list.push(Node::Kern {
                width: z - w - e - d,
                explicit: true,
            });
            list.push(Node::Box(number));
        }
        // §1206's third piece: a number that did not fit beside the display is
        // set flush right on a line of its own. There is one line here, so it
        // goes at the right edge of it.
        (false, _) => {
            list.push(Node::Kern {
                width: d,
                explicit: true,
            });
            list.push(Node::Box(body));
            if number.width > 0 {
                let rest = z - d - w - number.width;
                list.push(Node::Kern {
                    width: rest,
                    explicit: true,
                });
                list.push(Node::Box(number));
            }
        }
    }
    hpack(list, Spec::Exactly(z), Tolerances::plain(), None).node
}

/// `half(x)` (§100), as `mlist` computes it.
fn half(x: Scaled) -> Scaled {
    match x % 2 == 0 {
        true => x / 2,
        false => (x + 1) / 2,
    }
}
