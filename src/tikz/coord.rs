//! The ways a TikZ picture says where a point is.
//!
//! `(1,2)` is only the first of them. The manual's §13 is a whole chapter:
//! polar (§13.2.1), named (§13.2.4), relative to the last point (§13.4), and
//! the `calc` library's `($ ... $)` arithmetic (§13.5). A picture that draws a
//! hexagon with `(30:2cm)` and steps round it with `++(1,0)` is using three of
//! them in one line, and reading only the first form draws none of it.
//!
//! Everything comes back in the picture's own units -- the number a coordinate
//! was written with, which `to_pdf_ops` then multiplies by `x=`/`y=`. A length
//! written WITH a unit (`1cm`) is a length on the paper and must not be scaled
//! twice, so it is divided by the picture's scale here and multiplied back
//! there, which lands it where the paper says however the picture is scaled.

use super::units::{self, Length};
use std::collections::BTreeMap;

/// A point in the picture's own units.
pub type Point = (f64, f64);

/// What a coordinate is read against: the points a name has been given, the
/// picture's scale, and where the path currently is.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    /// `\coordinate (a) at ...` and `\node (a)`, in picture units.
    pub named: BTreeMap<String, Point>,
    /// The point the path last reached.
    pub current: Point,
    /// What `+(...)` and `++(...)` are measured from (§13.4).
    pub relative: Point,
    /// Where the last move-to was, which is where `cycle` closes back to.
    pub last_move: Point,
    /// The picture's `x=`/`y=`, so a length in real units can be un-scaled.
    pub x_scale: f64,
    pub y_scale: f64,
}

impl Frame {
    /// A frame for a picture scaled by `x_scale` and `y_scale`.
    pub fn new(x_scale: f64, y_scale: f64) -> Frame {
        Frame {
            x_scale,
            y_scale,
            ..Frame::default()
        }
    }

    /// A length written with a unit, in picture units on the given axis.
    fn on_axis(&self, length: Length, scale: f64) -> f64 {
        match length.points {
            // A degenerate scale would divide by zero and put the point at
            // infinity; the number itself is the least wrong answer.
            Some(points) if scale != 0.0 => points / scale,
            Some(points) => points,
            None => length.value,
        }
    }
}

/// How a coordinate was written, before it is resolved against a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Coord {
    /// `(1,2)`, `(1cm,2pt)`.
    Cartesian(Length, Length),
    /// `(30:2cm)` -- an angle in degrees and a radius (§13.2.1).
    Polar(f64, Length, Length),
    /// `(a)`, or `(a.north)` for one of a node's anchors.
    Named(String, Option<String>),
    /// `($ ... $)` -- a sum of scaled points (§13.5).
    Sum(Vec<(f64, Coord)>),
    /// `($(a)!f!(b)$)` -- the point `f` of the way from `a` to `b`.
    Partway(Box<Coord>, f64, Box<Coord>),
}

/// How a coordinate is measured: on its own, or from where the path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relative {
    /// `(1,2)`.
    Absolute,
    /// `+(1,2)` -- offset from the last point, which stays the last point.
    Offset,
    /// `++(1,2)` -- offset from the last point, and becomes the last point.
    Step,
}

impl Coord {
    /// Where this coordinate is, in the picture's units.
    pub fn resolve(&self, frame: &Frame) -> Point {
        match self {
            Coord::Cartesian(x, y) => (
                frame.on_axis(*x, frame.x_scale),
                frame.on_axis(*y, frame.y_scale),
            ),
            Coord::Polar(angle, rx, ry) => {
                let radians = angle.to_radians();
                (
                    frame.on_axis(*rx, frame.x_scale) * radians.cos(),
                    frame.on_axis(*ry, frame.y_scale) * radians.sin(),
                )
            }
            Coord::Named(name, anchor) => {
                let at = frame.named.get(name).copied().unwrap_or((0.0, 0.0));
                match anchor {
                    // An anchor on a name whose node this frame does not carry
                    // is the node's centre, which is where the name points.
                    Some(_) | None => at,
                }
            }
            Coord::Sum(terms) => terms.iter().fold((0.0, 0.0), |(x, y), (factor, term)| {
                let (tx, ty) = term.resolve(frame);
                (x + factor * tx, y + factor * ty)
            }),
            Coord::Partway(from, fraction, to) => {
                let (ax, ay) = from.resolve(frame);
                let (bx, by) = to.resolve(frame);
                (ax + (bx - ax) * fraction, ay + (by - ay) * fraction)
            }
        }
    }
}

/// Read the text between a coordinate's own parentheses.
///
/// The parentheses are already off: this takes `1,2`, `30:2cm`, `a`, `a.north`
/// or `$(a)+(1,0)$`.
pub fn parse(inside: &str) -> Option<Coord> {
    let text = inside.trim();
    if let Some(body) = text.strip_prefix('$').and_then(|t| t.strip_suffix('$')) {
        return calc(body);
    }
    // A colon that is not inside parentheses separates an angle from a radius.
    if let Some(at) = top_level(text, ':') {
        let angle = units::number(&text[..at])?;
        let (rx, ry) = radii(&text[at + 1..])?;
        return Some(Coord::Polar(angle, rx, ry));
    }
    if let Some(at) = top_level(text, ',') {
        let (x, rest) = units::scan(&text[..at])?;
        let (y, tail) = units::scan(&text[at + 1..])?;
        if !rest.trim().is_empty() || !tail.trim().is_empty() {
            return None;
        }
        return Some(Coord::Cartesian(x, y));
    }
    if text.is_empty() {
        return None;
    }
    // Anything left that is not a number is a name, with an optional anchor.
    match text.split_once('.') {
        Some((name, anchor)) if !anchor.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            Some(Coord::Named(name.trim().to_string(), Some(anchor.trim().to_string())))
        }
        _ => Some(Coord::Named(text.to_string(), None)),
    }
}

/// `2cm` on its own, or `2cm and 1cm` for an ellipse's two radii.
fn radii(text: &str) -> Option<(Length, Length)> {
    let (first, rest) = units::scan(text)?;
    match rest.trim().strip_prefix("and") {
        Some(second) => {
            let (second, tail) = units::scan(second)?;
            match tail.trim().is_empty() {
                true => Some((first, second)),
                false => None,
            }
        }
        None if rest.trim().is_empty() => Some((first, first)),
        None => None,
    }
}

/// The `calc` library's arithmetic, as far as a picture uses it (§13.5).
///
/// Two forms: a sum of points each optionally scaled -- `(a)+2*(1,0)` -- and
/// the partway modifier `(a)!.5!(b)`, which is how a midpoint is written.
fn calc(body: &str) -> Option<Coord> {
    if let Some(at) = top_level(body, '!') {
        let rest = &body[at + 1..];
        let close = top_level(rest, '!')?;
        let from = calc(&body[..at])?;
        let fraction = units::number(&rest[..close])?;
        let to = calc(&rest[close + 1..])?;
        return Some(Coord::Partway(Box::new(from), fraction, Box::new(to)));
    }
    let mut terms = Vec::new();
    let mut sign = 1.0;
    let mut rest = body.trim();
    while !rest.is_empty() {
        let (factor, after) = match units::scan(rest) {
            Some((number, after)) if after.trim_start().starts_with('*') => {
                (number.value, after.trim_start()[1..].trim_start())
            }
            _ => (1.0, rest),
        };
        let after = after.trim_start();
        let open = after.strip_prefix('(')?;
        let close = matching(open)?;
        terms.push((sign * factor, parse(&open[..close])?));
        rest = open[close + 1..].trim_start();
        match rest.chars().next() {
            Some('+') => sign = 1.0,
            Some('-') => sign = -1.0,
            None => break,
            _ => return None,
        }
        rest = rest[1..].trim_start();
    }
    match terms.is_empty() {
        true => None,
        false => Some(Coord::Sum(terms)),
    }
}

/// Where `wanted` appears outside any parentheses, which is where a separator
/// belongs: the `,` of `($(a)+(1,0)$)` inside the inner point is not one.
fn top_level(text: &str, wanted: char) -> Option<usize> {
    let mut depth = 0i32;
    for (at, ch) in text.char_indices() {
        match ch {
            '(' | '{' => depth += 1,
            ')' | '}' => depth -= 1,
            c if c == wanted && depth == 0 => return Some(at),
            _ => {}
        }
    }
    None
}

/// The index of the `)` that closes a `(` already consumed.
pub fn matching(after_open: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (at, ch) in after_open.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(at),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        let mut frame = Frame::new(1.0, 1.0);
        frame.named.insert("a".to_string(), (1.0, 1.0));
        frame.named.insert("b".to_string(), (3.0, 5.0));
        frame
    }

    #[test]
    fn a_polar_coordinate_is_the_point_the_angle_names() {
        // lualatex draws `(30:2cm)` at (49.098pt, 28.347pt) -- 2cm is 56.906pt
        // and cos 30 is 0.866. Reading the `:` as part of a name would put it
        // at the origin instead.
        let point = parse("30:2cm").unwrap().resolve(&frame());
        let two_cm = 2.0 * 72.27 / 2.54;
        assert!((point.0 - two_cm * 30f64.to_radians().cos()).abs() < 1e-9);
        assert!((point.1 - two_cm * 30f64.to_radians().sin()).abs() < 1e-9);
    }

    #[test]
    fn a_name_is_the_point_it_was_given() {
        assert_eq!(parse("a").unwrap().resolve(&frame()), (1.0, 1.0));
        // `(a.north)` is the node's own anchor; without the node it is the
        // point the name stands for, which is where the name is.
        assert_eq!(parse("a.north").unwrap().resolve(&frame()), (1.0, 1.0));
    }

    #[test]
    fn calc_adds_points_and_walks_between_them() {
        assert_eq!(parse("$(a)+(1,0)$").unwrap().resolve(&frame()), (2.0, 1.0));
        assert_eq!(parse("$(b)-(a)$").unwrap().resolve(&frame()), (2.0, 4.0));
        assert_eq!(parse("$2*(a)$").unwrap().resolve(&frame()), (2.0, 2.0));
        // The midpoint of a and b, which is what `!.5!` is written for.
        assert_eq!(parse("$(a)!.5!(b)$").unwrap().resolve(&frame()), (2.0, 3.0));
    }

    #[test]
    fn a_length_with_a_unit_survives_the_pictures_scale() {
        // `x=0.5pt` halves every bare number, and `to_pdf_ops` does that
        // multiplication -- so a coordinate written as a real length has to
        // come back pre-divided or it lands at half the length it names.
        let frame = Frame::new(0.5, 0.5);
        let point = parse("1cm,0").unwrap().resolve(&frame);
        assert!((point.0 * 0.5 - 72.27 / 2.54).abs() < 1e-9);
        // A bare number is a picture unit and is not touched.
        assert_eq!(parse("4,2").unwrap().resolve(&frame), (4.0, 2.0));
    }
}
