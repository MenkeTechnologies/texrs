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

/// The five numbers a glue is.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Glue {
    pub natural: i64,
    pub stretch: i64,
    pub stretch_order: Order,
    pub shrink: i64,
    pub shrink_order: Order,
}

/// Combine two stretch or shrink components.
///
/// An infinite one beats any finite one however large, and a higher infinity
/// beats a lower: measured against LuaTeX, `2pt plus 4fil` is `4fil` and
/// `2fill plus 4fil` is `2fill`. Only components of the SAME order add, which
/// is why this cannot be a plain sum.
fn combine(a: (i64, Order), b: (i64, Order)) -> (i64, Order) {
    match a.1.cmp(&b.1) {
        std::cmp::Ordering::Greater => a,
        std::cmp::Ordering::Less => b,
        std::cmp::Ordering::Equal => (a.0 + b.0, a.1),
    }
}

impl Glue {
    /// A finite length with no stretch or shrink.
    pub fn fixed(natural: i64) -> Glue {
        Glue {
            natural,
            ..Glue::default()
        }
    }

    /// Multiply every component, which is what a glue expression's `*` does.
    pub fn scale(self, by: i64) -> Glue {
        Glue {
            natural: self.natural * by,
            stretch: self.stretch * by,
            shrink: self.shrink * by,
            ..self
        }
    }

    /// Divide every component, by the expression rule -- rounding, not
    /// truncating, exactly as `\numexpr` divides.
    pub fn divide(self, by: i64, round: impl Fn(i64, i64) -> i64) -> Glue {
        Glue {
            natural: round(self.natural, by),
            stretch: round(self.stretch, by),
            shrink: round(self.shrink, by),
            ..self
        }
    }
}

/// Adding glue adds the natural widths and combines each of the two
/// infinity-orders separately, which is what `\advance` on a skip does.
///
/// Written as the std trait rather than as an inherent `add`, because an
/// inherent method by that name is the one thing a reader may take for the
/// operator and it is not -- clippy rejects the pair outright. Every existing
/// `a + b` call still resolves, to this.
impl std::ops::Add for Glue {
    type Output = Glue;

    fn add(self, other: Glue) -> Glue {
        let (stretch, stretch_order) = combine(
            (self.stretch, self.stretch_order),
            (other.stretch, other.stretch_order),
        );
        let (shrink, shrink_order) = combine(
            (self.shrink, self.shrink_order),
            (other.shrink, other.shrink_order),
        );
        Glue {
            natural: self.natural + other.natural,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        }
    }
}

/// Subtracting negates the components and adds, so the order arithmetic stays
/// in one place.
impl std::ops::Sub for Glue {
    type Output = Glue;

    fn sub(self, other: Glue) -> Glue {
        self + Glue {
            natural: -other.natural,
            stretch: -other.stretch,
            shrink: -other.shrink,
            ..other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimen::UNITY;

    /// Read off `tex -ini`, not computed here.
    /// Read off LuaTeX 1.24.0, not derived here.
    #[test]
    fn an_infinite_component_beats_a_finite_one() {
        let pt = |n: i64| n * UNITY;
        // 1pt plus 2pt + 3pt plus 4fil => 4.0pt plus 4.0fil
        let a = Glue {
            natural: pt(1),
            stretch: pt(2),
            stretch_order: 0,
            ..Glue::default()
        };
        let b = Glue {
            natural: pt(3),
            stretch: pt(4),
            stretch_order: 1,
            ..Glue::default()
        };
        let sum = a + b;
        assert_eq!(
            print_glue(sum.natural, sum.stretch, sum.stretch_order, 0, 0),
            "4.0pt plus 4.0fil"
        );
        // 1pt plus 2fill + 3pt plus 4fil => 4.0pt plus 2.0fill
        let a = Glue {
            natural: pt(1),
            stretch: pt(2),
            stretch_order: 2,
            ..Glue::default()
        };
        let sum = a + b;
        assert_eq!(
            print_glue(sum.natural, sum.stretch, sum.stretch_order, 0, 0),
            "4.0pt plus 2.0fill"
        );
        // equal orders add
        let a = Glue {
            natural: pt(1),
            stretch: pt(2),
            stretch_order: 1,
            ..Glue::default()
        };
        let sum = a + b;
        assert_eq!(
            print_glue(sum.natural, sum.stretch, sum.stretch_order, 0, 0),
            "4.0pt plus 6.0fil"
        );
    }

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
