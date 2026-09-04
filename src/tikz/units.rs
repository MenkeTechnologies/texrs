//! The lengths a TikZ option or coordinate is written in.
//!
//! TikZ takes a dimension anywhere it takes a number, and the two are not the
//! same thing: `(1,0)` is one of the picture's own units and `(1cm,0)` is a
//! centimetre on the paper however the picture was scaled. Reading `1cm` as the
//! number 1 puts the point 28 times too close to the origin, so the unit is
//! read rather than skipped.
//!
//! The factors are TeX's own (`texbook`, Chapter 10): a point is the unit, and
//! everything else is a ratio to it. `bp` -- the big point -- is the one that
//! catches people out, because it is PostScript's point and PDF's, and 72 of
//! them make an inch where 72.27 TeX points do.

/// How many points one of each unit comes to.
///
/// `em` and `ex` are font-relative and have no fixed answer; the pair here is
/// the one a 10pt roman gives, which is what a `tikzpicture` in body text is
/// set in unless the document says otherwise.
const UNITS: &[(&str, f64)] = &[
    ("pt", 1.0),
    ("bp", 72.27 / 72.0),
    ("mm", 72.27 / 25.4),
    ("cm", 72.27 / 2.54),
    ("in", 72.27),
    ("pc", 12.0),
    ("dd", 1238.0 / 1157.0),
    ("cc", 12.0 * 1238.0 / 1157.0),
    ("sp", 1.0 / 65536.0),
    ("em", 10.0),
    ("ex", 4.305),
];

/// A number, and the unit it was written with if it had one.
///
/// The unit is kept apart from the number because the two mean different things
/// to a coordinate: a bare `32` is 32 of whatever `x=` made a unit, and `32pt`
/// is 32 points whatever `x=` says. Collapsing them here would lose that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    pub value: f64,
    /// `None` for a bare number, `Some(points)` once a unit named one.
    pub points: Option<f64>,
}

impl Length {
    /// The length in points, taking a bare number as `unit` points each.
    pub fn points(&self, unit: f64) -> f64 {
        match self.points {
            Some(pt) => pt,
            None => self.value * unit,
        }
    }
}

/// Read `-3.5cm`, `2`, `.5pt` or `+4` off the front of `text`.
///
/// Returns the length and what is left, so a caller scanning `1cm and 2cm` can
/// go on from where this stopped.
pub fn scan(text: &str) -> Option<(Length, &str)> {
    let text = text.trim_start();
    let bytes = text.as_bytes();
    let mut at = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        at += 1;
    }
    let digits = at;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        at += 1;
    }
    if at < bytes.len() && bytes[at] == b'.' {
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
    }
    // A sign and a dot with no digit between them is not a number, and parsing
    // it as one would turn `..controls` into a coordinate.
    if at == digits || text[digits..at].chars().all(|c| c == '.') {
        return None;
    }
    let value: f64 = text[..at].parse().ok()?;
    let rest = &text[at..];
    for (name, factor) in UNITS {
        if let Some(after) = rest.strip_prefix(name) {
            return Some((
                Length {
                    value,
                    points: Some(value * factor),
                },
                after,
            ));
        }
    }
    Some((
        Length {
            value,
            points: None,
        },
        rest,
    ))
}

/// The whole of `text` as a dimension in points, or nothing if it is not one.
///
/// A bare number is taken as points, which is what every TikZ option that wants
/// a length does with one -- `line width=2` is two points.
pub fn dimension(text: &str) -> Option<f64> {
    let (length, rest) = scan(text)?;
    match rest.trim().is_empty() {
        true => Some(length.points(1.0)),
        false => None,
    }
}

/// The whole of `text` as a plain number, for `scale=`, `opacity=` and angles.
pub fn number(text: &str) -> Option<f64> {
    let (length, rest) = scan(text)?;
    match rest.trim().is_empty() {
        true => Some(length.value),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_is_read_and_not_skipped() {
        // 1cm is 28.45pt, and a coordinate that read it as 1 would land 28
        // times too close to the origin.
        assert_eq!(dimension("1cm"), Some(72.27 / 2.54));
        assert_eq!(dimension("2pt"), Some(2.0));
        assert_eq!(dimension("1in"), Some(72.27));
        // A big point is PostScript's, and 72 of them make an inch.
        assert_eq!(dimension("72bp"), Some(72.27));
        assert_eq!(dimension("-1.5pt"), Some(-1.5));
        // A bare number is points where a length is wanted.
        assert_eq!(dimension("3"), Some(3.0));
    }

    #[test]
    fn a_bare_number_keeps_its_unitlessness() {
        let (length, rest) = scan("32,58").unwrap();
        assert_eq!(length.points, None, "no unit was written");
        assert_eq!(length.value, 32.0);
        assert_eq!(rest, ",58");
        // Which is what lets the picture's own `x=` scale it later.
        assert_eq!(length.points(0.38), 32.0 * 0.38);
        // A unit overrules the picture's scale, as it does in TikZ.
        let (length, _) = scan("1cm").unwrap();
        assert_eq!(length.points(0.38), 72.27 / 2.54);
    }

    #[test]
    fn what_is_not_a_number_is_not_read_as_one() {
        // `..controls` begins with the dots a decimal point is made of, and
        // reading it as a number would swallow the curve operator.
        assert!(scan("..controls (1,1)").is_none());
        assert!(scan("cycle").is_none());
        assert_eq!(dimension("2pt junk"), None);
    }
}
