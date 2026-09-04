//! Arrow tips, drawn as the paths PGF draws them as.
//!
//! An arrow in TikZ is not a decoration on a line: the line is SHORTENED by the
//! tip's forward reach and the tip is a path of its own, placed at the new end
//! and turned to face along it. Drawing the full-length line and putting a
//! triangle over it leaves the line's own end poking through the tip, which is
//! visible at any width above hairline.
//!
//! The three tips here are `pgfcorearrows.code.tex`'s own, read out of it
//! rather than measured off a picture -- lines 1118-1141 for `stealth`,
//! 1143-1169 for `to` (which is what `>` and `<` mean), and 1198-1227 for
//! `latex`. All three are built on one length,
//!
//! ```text
//! u = 0.28pt + 0.3 * line width
//! ```
//!
//! which is why a thicker line gets a bigger arrowhead. At the default 0.4pt
//! that is exactly 0.4pt, and lualatex writes `stealth`'s tip point at
//! `1.99252` big points -- five of those u in PDF's units, to the digit.

use super::options::Tip;
use super::{Point, Segment};

/// One arrow tip, in its own space: the line runs along +x and ends at 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    /// Where the tip's outline starts, and the segments that draw it.
    pub start: Point,
    pub segments: Vec<Segment>,
    /// Filled (`stealth`, `latex`) or stroked (`to`).
    pub filled: bool,
    /// The width the outline is stroked at, when it is stroked.
    pub width: f64,
    /// How far back along the line the tip reaches forward -- PGF's
    /// `\pgfarrowsrightextend`, and the distance the line is shortened by.
    pub reach: f64,
}

/// The tip `which` draws on a line of `line_width` points.
pub fn head(which: Tip, line_width: f64) -> Head {
    // `\pgfutil@tempdima=0.28pt \advance\pgfutil@tempdima by.3\pgflinewidth`
    // -- pgfcorearrows.code.tex lines 1130-1135, and the same two lines again
    // at 1153-1154 and 1210-1215.
    let u = 0.28 + 0.3 * line_width;
    match which {
        // Lines 1136-1140: a filled quadrilateral with the notch at the back.
        Tip::Stealth => Head {
            start: (5.0 * u, 0.0),
            segments: vec![
                Segment::Line((-3.0 * u, 4.0 * u)),
                Segment::Line((0.0, 0.0)),
                Segment::Line((-3.0 * u, -4.0 * u)),
            ],
            filled: true,
            width: line_width,
            // `\pgfarrowsrightextend{+5\pgfutil@tempdima}` -- line 1127.
            reach: 5.0 * u,
        },
        // Lines 1216-1226: two curves and the straight back edge between them.
        Tip::Latex => Head {
            start: (9.0 * u, 0.0),
            segments: vec![
                Segment::Curve(
                    (6.3333 * u, 0.5 * u),
                    (2.0 * u, 2.0 * u),
                    (-u, 3.75 * u),
                ),
                Segment::Line((-u, -3.75 * u)),
                Segment::Curve(
                    (2.0 * u, -2.0 * u),
                    (6.3333 * u, -0.5 * u),
                    (9.0 * u, 0.0),
                ),
            ],
            filled: true,
            width: line_width,
            // `\pgfarrowsrightextend{+9\pgfutil@tempdima}` -- line 1207.
            reach: 9.0 * u,
        },
        // Lines 1159-1168: one stroked stroke, at four fifths the line's
        // width, with round caps and joins.
        Tip::To => Head {
            start: (-3.0 * u, 4.0 * u),
            segments: vec![
                Segment::Curve(
                    (-2.75 * u, 2.5 * u),
                    (0.0, 0.25 * u),
                    (0.75 * u, 0.0),
                ),
                Segment::Curve(
                    (0.0, -0.25 * u),
                    (-2.75 * u, -2.5 * u),
                    (-3.0 * u, -4.0 * u),
                ),
            ],
            filled: false,
            // `\pgfsetlinewidth{0.8\pgflinewidth}` -- line 1155.
            width: 0.8 * line_width,
            // `0.21pt + 0.625\pgflinewidth` -- lines 1147-1148.
            reach: 0.21 + 0.625 * line_width,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PDF big point in TeX points, which is what lualatex's numbers are in.
    const BP: f64 = 72.27 / 72.0;

    #[test]
    fn stealth_is_the_shape_lualatex_draws() {
        // lualatex, `\draw[-stealth] (0,0) -- (2,0);` at the default width,
        // writes the tip as
        //   1.99252 0.0 m -1.19551 1.59401 l 0.0 0.0 l -1.19551 -1.59401 l f
        // and shortens the line from 56.69362 to 54.7011 -- 1.99252 of reach.
        let head = head(Tip::Stealth, 0.4);
        assert!((head.start.0 / BP - 1.99252).abs() < 1e-4, "{:?}", head.start);
        assert!((head.reach / BP - 1.99252).abs() < 1e-4, "{}", head.reach);
        assert_eq!(head.segments.len(), 3);
        match head.segments[0] {
            Segment::Line((x, y)) => {
                assert!((x / BP + 1.19551).abs() < 1e-4, "{x}");
                assert!((y / BP - 1.59401).abs() < 1e-4, "{y}");
            }
            ref other => panic!("straight, not {other:?}"),
        }
        assert!(head.filled, "stealth is filled, and lualatex ends it with `f`");
    }

    #[test]
    fn latex_is_the_shape_lualatex_draws() {
        // lualatex writes
        //   3.58653 0.0 m 2.52383 0.19925 0.797 0.797 -0.3985 1.49438 c
        //   -0.3985 -1.49438 l 0.797 -0.797 2.52383 -0.19925 3.58653 0.0 c f
        let head = head(Tip::Latex, 0.4);
        assert!((head.start.0 / BP - 3.58653).abs() < 1e-4, "{:?}", head.start);
        match head.segments[0] {
            Segment::Curve((c1x, c1y), (c2x, c2y), (x, y)) => {
                assert!((c1x / BP - 2.52383).abs() < 1e-3, "{c1x}");
                assert!((c1y / BP - 0.19925).abs() < 1e-4, "{c1y}");
                assert!((c2x / BP - 0.797).abs() < 1e-4, "{c2x}");
                assert!((c2y / BP - 0.797).abs() < 1e-4, "{c2y}");
                assert!((x / BP + 0.3985).abs() < 1e-4, "{x}");
                assert!((y / BP - 1.49438).abs() < 1e-4, "{y}");
            }
            ref other => panic!("a curve, not {other:?}"),
        }
    }

    #[test]
    fn the_default_arrow_is_stroked_and_not_filled() {
        // `->` is PGF's `to`: two curves stroked at four fifths of the line's
        // width, which lualatex writes as `0.31879 w` under a 0.3985 line.
        let head = head(Tip::To, 0.4);
        assert!(!head.filled);
        assert!((head.width / BP - 0.31879).abs() < 1e-4, "{}", head.width);
        assert!((head.start.0 / BP + 1.19551).abs() < 1e-4, "{:?}", head.start);
        assert!((head.start.1 / BP - 1.59401).abs() < 1e-4, "{:?}", head.start);
        // The line is shortened by 0.4583pt, which is where lualatex's
        // 56.23534 comes from against a full length of 56.69362.
        assert!((head.reach / BP - 0.45828).abs() < 1e-4, "{}", head.reach);
    }

    #[test]
    fn a_thicker_line_gets_a_bigger_head() {
        // u = 0.28pt + 0.3 * line width, so the tip grows with the line but
        // not in proportion to it -- at 1pt lualatex writes 3u as 1.73352.
        let head = head(Tip::To, 1.0);
        let u = 0.28 + 0.3;
        assert!((head.start.0 + 3.0 * u).abs() < 1e-9);
        assert!((3.0 * u / BP - 1.73352).abs() < 1e-4);
    }
}
