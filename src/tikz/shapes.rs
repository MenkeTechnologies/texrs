//! Where a node's border is, and where its anchors sit on it (S17.2, S67).
//!
//! A node is not a rectangle around some text: it is a SHAPE, and a shape is a
//! border path plus a table of named points on it. `(a.north)` is a lookup in
//! that table, and answering it with the node's centre -- which is what this
//! module exists to stop -- draws a line that starts in the middle of a box it
//! was written to start on the edge of.
//!
//! Three numbers decide the box, and PGF spells all three out:
//!
//! - `inner xsep`/`inner ysep`, `.3333em` each, between the text and the
//!   border (`pgfmoduleshapes.code.tex` lines 888-889);
//! - `minimum width`/`minimum height`, which the box is grown to;
//! - `outer xsep`/`outer ysep`, `.5\pgflinewidth` each (lines 891-892), which
//!   the ANCHORS stand off by and the DRAWN border does not -- the rectangle
//!   shape adds it to `\northeast` (line 1009) and its background path takes
//!   it straight back off (lines 1098-1102). So a node's anchor is half a line
//!   width outside the line, which is what keeps an arrow's tip off the
//!   border it points at.
//!
//! lualatex draws `\node (a) at (0,0) [draw] {Hi}` as
//! `-8.44142 -6.72284 16.88286 13.44568 re` and puts `(a.north)` at
//! `0.0 6.92209`: the border at 6.72284 and the anchor 0.19925 -- half of the
//! 0.3985 line width -- beyond it.

use super::options::{Anchor, Shape};
use super::Point;

/// A node's border box, as its anchors see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub shape: Shape,
    /// Half the box, with the outer separation already in it.
    pub half: (f64, f64),
    /// The outer separation, which the drawn outline takes back off.
    pub outer: f64,
}

impl Default for Border {
    fn default() -> Border {
        Border {
            shape: Shape::Coordinate,
            half: (0.0, 0.0),
            outer: 0.0,
        }
    }
}

impl Border {
    /// The border of a shape whose text, with its inner separation, comes to
    /// `text_half` either side of the centre.
    ///
    /// `minimum` is `minimum width` and `minimum height`; `outer` is the outer
    /// separation; `aspect` is `\pgfshapeaspect`, which only the diamond uses.
    pub fn of(
        shape: Shape,
        text_half: (f64, f64),
        minimum: (f64, f64),
        outer: f64,
        aspect: f64,
    ) -> Border {
        let (tx, ty) = text_half;
        let half = match shape {
            // The point a `\coordinate` names has no extent, and every one of
            // its anchors is that point -- `pgfcoreshapes.code.tex` gives the
            // coordinate shape a `\centerpoint` and nothing else.
            Shape::Coordinate => (0.0, 0.0),
            // `pgfmoduleshapes.code.tex` lines 975-1009.
            Shape::Rectangle => (
                tx.max(minimum.0 / 2.0) + outer,
                ty.max(minimum.1 / 2.0) + outer,
            ),
            // Lines 1200-1257: the radius is the length of the vector whose
            // parts are the two half-sizes, then grown to the minimum, then
            // the larger outer separation added.
            Shape::Circle => {
                let radius = circle_radius(tx, ty)
                    .max(minimum.0 / 2.0)
                    .max(minimum.1 / 2.0)
                    + outer;
                (radius, radius)
            }
            // `pgflibraryshapes.geometric.code.tex` lines 22-62: root two
            // times each half-size, so the text's corners sit ON the ellipse
            // rather than inside it.
            Shape::Ellipse => (
                (ROOT_TWO * tx).max(minimum.0 / 2.0) + outer,
                (ROOT_TWO * ty).max(minimum.1 / 2.0) + outer,
            ),
            // Lines 236-276: the half-width is the text's half-width plus the
            // aspect times its half-height, which is what makes the text's
            // corner touch the sloping side.
            Shape::Diamond => {
                let x = tx + aspect * ty;
                let y = match aspect == 0.0 {
                    true => ty,
                    false => tx / aspect + ty,
                };
                (
                    x.max(minimum.0 / 2.0) + outer,
                    y.max(minimum.1 / 2.0) + outer,
                )
            }
        };
        Border { shape, half, outer }
    }

    /// Half the box the border is DRAWN on -- the anchor box less the outer
    /// separation each shape's own background path takes off.
    pub fn drawn(&self) -> (f64, f64) {
        let (hx, hy) = self.half;
        match self.shape {
            Shape::Coordinate => (0.0, 0.0),
            // Rectangle: lines 1098-1102. Circle: lines 1317-1327. Ellipse:
            // lines 183-193 of the geometric library.
            Shape::Rectangle | Shape::Circle | Shape::Ellipse => (hx - self.outer, hy - self.outer),
            // The diamond's sides are sloped, so standing the line off by the
            // outer separation means pulling the CORNERS in by root two times
            // it -- geometric library lines 328-341.
            Shape::Diamond => (
                hx - ROOT_TWO_PGF * self.outer,
                hy - ROOT_TWO_PGF * self.outer,
            ),
        }
    }

    /// Where one of the nine named anchors is, measured from the centre.
    pub fn anchor(&self, anchor: Anchor) -> Point {
        let (hx, hy) = self.half;
        let (dx, dy) = anchor.offset();
        match self.shape {
            // A circle's corner anchors are on the circle and not on the
            // square around it, so they are at cos 45 of the radius --
            // `pgfmoduleshapes.code.tex` lines 1274-1297 write 0.707107 out.
            Shape::Circle | Shape::Ellipse if dx != 0.0 && dy != 0.0 => {
                (dx * COS45 * hx, dy * COS45 * hy)
            }
            // The diamond's corner anchors are the MIDDLE of each sloping
            // side, which is half of each half -- geometric library line 301.
            Shape::Diamond if dx != 0.0 && dy != 0.0 => (dx * hx / 2.0, dy * hy / 2.0),
            _ => (dx * hx, dy * hy),
        }
    }

    /// Where the border is in the direction `degrees`, from the centre.
    ///
    /// This is PGF's `\anchorborder`, which is what `(a.30)` asks for.
    pub fn border_at(&self, degrees: f64) -> Point {
        let (sin, cos) = degrees.to_radians().sin_cos();
        self.toward((cos, sin))
    }

    /// The same, in the direction of an arbitrary vector.
    pub fn toward(&self, (dx, dy): Point) -> Point {
        let (hx, hy) = self.half;
        if (dx == 0.0 && dy == 0.0) || (hx == 0.0 && hy == 0.0) {
            return (0.0, 0.0);
        }
        match self.shape {
            Shape::Coordinate => (0.0, 0.0),
            // `\pgfpointborderrectangle`, `pgfcorepoints.code.tex` lines
            // 972-1010: whichever side the ray leaves by, met at that side.
            Shape::Rectangle => match (dy / dx).abs() * hx <= hy {
                true => (dx.signum() * hx, dy / dx.abs() * hx),
                false => (dx / dy.abs() * hy, dy.signum() * hy),
            },
            // `\pgfpointborderellipse`, lines 1060-1105: the direction is
            // squashed into the unit circle, normalised there and stretched
            // back out, which lands it on the ellipse.
            Shape::Circle | Shape::Ellipse => {
                let (ux, uy) = (dx / hx, dy / hy);
                let length = ux.hypot(uy);
                (hx * ux / length, hy * uy / length)
            }
            // The diamond's border is a line from (hx,0) to (0,hy) in the
            // quadrant the ray points into -- geometric library lines 303-323
            // intersect the ray with exactly that line.
            Shape::Diamond => {
                let (sx, sy) = (dx.signum(), dy.signum());
                // The side is x/hx + y/hy = 1 in the ray's own quadrant.
                let scale = 1.0 / (dx.abs() / hx + dy.abs() / hy);
                (sx * dx.abs() * scale, sy * dy.abs() * scale)
            }
        }
    }
}

/// PGF's own constant for root two, `pgflibraryshapes.geometric.code.tex`
/// lines 39-40.
const ROOT_TWO: f64 = 1.414_213_6;

/// And the one its diamond's background path is written with, line 334, which
/// is the same number to one digit fewer.
const ROOT_TWO_PGF: f64 = 1.414_213;

/// Cosine of 45 degrees as the shape code writes it -- `pgfmoduleshapes`
/// line 1277.
const COS45: f64 = 0.707_107;

/// The circle shape's radius, by the arithmetic `pgfmoduleshapes.code.tex`
/// lines 1213-1235 does it with.
///
/// This is `sqrt(x^2 + y^2)` computed as `x / (x/|v|)`, in TeX's dimensions --
/// which round to a scaled point at every step, and divide by an integer that
/// has itself been truncated. It comes out 0.4% short of the exact answer, and
/// that is the number in the file: lualatex draws the circle of a node reading
/// "Hi" at radius 10.75107bp where the exact hypotenuse is 10.79139bp. Taking
/// the square root instead draws a circle the reader can see is bigger than
/// the one the document has.
fn circle_radius(half_width: f64, half_height: f64) -> f64 {
    let length = half_width.hypot(half_height);
    if length == 0.0 {
        return 0.0;
    }
    // `\pgfpointnormalised` leaves a unit vector as two dimensions, so each
    // part is a whole number of scaled points and nothing finer.
    let unit = |value: f64| (value / length * 65536.0).trunc();
    let (nx, ny) = (unit(half_width), unit(half_height));
    let (bigger, along) = match nx > ny {
        true => (nx, half_width),
        false => (ny, half_height),
    };
    // `\divide\c@pgf@counta by 255`, then `16(16 x)/counta` -- all integer.
    let divisor = (bigger / 255.0).trunc();
    if divisor == 0.0 {
        return length;
    }
    let sp = (along * 65536.0).trunc();
    (16.0 * ((16.0 * sp) / divisor).trunc()) / 65536.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node lualatex was asked to draw: `\node (a) at (0,0) [draw] {Hi}`
    /// in a 10pt document, whose border came out
    /// `-8.44142 -6.72284 16.88286 13.44568 re`.
    fn hi() -> ((f64, f64), f64) {
        ((8.44142, 6.72284), 0.19925)
    }

    #[test]
    fn a_rectangles_anchors_stand_off_by_the_outer_separation() {
        let (half, outer) = hi();
        let border = Border::of(Shape::Rectangle, half, (0.0, 0.0), outer, 1.0);
        // lualatex draws `(a.north)` at `0.0 6.92209` and `(a.north east)` at
        // `8.64067 6.92209`, both a half line width outside the border.
        let (x, y) = border.anchor(Anchor::North);
        assert!(
            (x - 0.0).abs() < 1e-9 && (y - 6.92209).abs() < 1e-5,
            "{x} {y}"
        );
        let (x, y) = border.anchor(Anchor::NorthEast);
        assert!(
            (x - 8.64067).abs() < 1e-5 && (y - 6.92209).abs() < 1e-5,
            "{x} {y}"
        );
        // And the border it DRAWS is the box without that standoff.
        let (dx, dy) = border.drawn();
        assert!((dx - 8.44142).abs() < 1e-9 && (dy - 6.72284).abs() < 1e-9);
    }

    #[test]
    fn a_rectangles_border_at_an_angle_is_where_the_ray_leaves_it() {
        let (half, outer) = hi();
        let border = Border::of(Shape::Rectangle, half, (0.0, 0.0), outer, 1.0);
        // lualatex puts `(a.30)` at `8.64067 4.98892`: out to the right side,
        // because 30 degrees is flatter than the box's own corner.
        let (x, y) = border.border_at(30.0);
        assert!((x - 8.64067).abs() < 1e-4, "{x}");
        assert!((y - 4.98892).abs() < 4e-3, "{y}");
    }

    #[test]
    fn a_circles_radius_is_pgfs_arithmetic_and_not_the_hypotenuse() {
        let (half, outer) = hi();
        let border = Border::of(Shape::Circle, half, (0.0, 0.0), outer, 1.0);
        // lualatex draws the circle of `\node (b) [draw,circle] {Hi}` at
        // radius 10.75107, where the exact hypotenuse of the two half-sizes
        // is 10.79139 -- 0.04bp bigger, which is a visible edge.
        let (drawn, _) = border.drawn();
        assert!((drawn - 10.75107).abs() < 4e-3, "{drawn}");
        assert!(
            (drawn - half.0.hypot(half.1)).abs() > 0.03,
            "and not the hypotenuse"
        );
        // `(b.south)` came out at 10.95033 below the centre: the radius plus
        // the outer separation.
        let (_, y) = border.anchor(Anchor::South);
        assert!((y + 10.95033).abs() < 4e-3, "{y}");
    }

    #[test]
    fn a_circles_border_at_an_angle_is_on_the_circle() {
        let (half, outer) = hi();
        let border = Border::of(Shape::Circle, half, (0.0, 0.0), outer, 1.0);
        // `(b.60)` came out 5.47482 right of and 9.48328 above the centre.
        let (x, y) = border.border_at(60.0);
        assert!((x - 5.47482).abs() < 4e-3, "{x}");
        assert!((y - 9.48328).abs() < 4e-3, "{y}");
    }

    #[test]
    fn a_diamonds_corner_anchors_are_the_middle_of_its_sides() {
        let border = Border::of(Shape::Diamond, (10.0, 5.0), (0.0, 0.0), 0.0, 1.0);
        // Half-width is 10+5 and half-height 10+5 at aspect one -- geometric
        // library lines 250-253.
        assert_eq!(border.half, (15.0, 15.0));
        assert_eq!(border.anchor(Anchor::East), (15.0, 0.0));
        // Line 301: `\pgf@x=.5\pgf@x \pgf@y=.5\pgf@y`.
        assert_eq!(border.anchor(Anchor::NorthEast), (7.5, 7.5));
        // The border along 45 degrees is where x/hx + y/hy = 1 meets it.
        let (x, y) = border.border_at(45.0);
        assert!((x - 7.5).abs() < 1e-9 && (y - 7.5).abs() < 1e-9, "{x} {y}");
    }

    #[test]
    fn a_coordinate_has_no_extent_and_every_anchor_is_the_point() {
        let border = Border::default();
        assert_eq!(border.anchor(Anchor::North), (0.0, 0.0));
        assert_eq!(border.border_at(30.0), (0.0, 0.0));
    }
}
