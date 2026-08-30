//! Glue: a dimension that can stretch and shrink.
//!
//! Three dimensions and two orders. The orders are what make glue more than a
//! triple of lengths: `1fil` is infinitely more stretch than any number of
//! points, `1fill` infinitely more than any `fil`, and TeX writes the unit
//! back as it was given rather than converting between orders.

/// How infinite a stretch or shrink component is: 0 is finite (points), and
/// 1, 2, 3 are `fil`, `fill` and `filll`.
pub type Order = i64;

/// The unit an order is written with.
pub fn order_unit(order: Order) -> &'static str {
    match order {
        1 => "fil",
        2 => "fill",
        3 => "filll",
        _ => "pt",
    }
}

/// The order a run of letters names, if it names one.
///
/// `fil`, `fill` and `filll` and nothing else: TeX stops at three l's, so
/// `fillll` is not a wider infinity, it is an error.
pub fn order_of(unit: &str) -> Option<Order> {
    match unit {
        "fil" => Some(1),
        "fill" => Some(2),
        "filll" => Some(3),
        _ => None,
    }
}

/// A glue as TeX writes it (`tex.web` §178).
///
/// A zero component is not written at all -- `1pt plus 0pt` reads back as
/// `1.0pt` -- which is why this cannot be three calls to `print_scaled` with
/// the words between them.
pub fn print_glue(
    natural: i64,
    stretch: i64,
    stretch_order: Order,
    shrink: i64,
    shrink_order: Order,
) -> String {
    let mut out = format!("{}pt", crate::dimen::print_scaled(natural));
    if stretch != 0 {
        out.push_str(&format!(
            " plus {}{}",
            crate::dimen::print_scaled(stretch),
            order_unit(stretch_order)
        ));
    }
    if shrink != 0 {
        out.push_str(&format!(
            " minus {}{}",
            crate::dimen::print_scaled(shrink),
            order_unit(shrink_order)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimen::UNITY;

    /// Read off `tex -ini`, not computed here.
    #[test]
    fn a_glue_is_written_the_way_tex_writes_one() {
        assert_eq!(print_glue(UNITY, 0, 0, 0, 0), "1.0pt");
        assert_eq!(print_glue(UNITY, 2 * UNITY, 0, 0, 0), "1.0pt plus 2.0pt");
        assert_eq!(
            print_glue(UNITY, 2 * UNITY, 0, 3 * UNITY, 0),
            "1.0pt plus 2.0pt minus 3.0pt"
        );
        assert_eq!(print_glue(0, UNITY, 1, 0, 0), "0.0pt plus 1.0fil");
        assert_eq!(
            print_glue(0, UNITY, 2, 2 * UNITY, 3),
            "0.0pt plus 1.0fill minus 2.0filll"
        );
    }
}
