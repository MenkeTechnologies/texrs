//! Dimensions: scaled points, the units that reach them, and how TeX prints
//! one back.
//!
//! A TeX dimension is an integer count of scaled points, 65536 to the printer's
//! point, and every unit is a rational multiple of a point held as an exact
//! integer ratio (`tex.web` §458) rather than as a float — which is why
//! `1in` is `72.26999pt` and not `72.27pt`. Printing is Knuth's `print_scaled`
//! (§103): the shortest decimal that reads back as the same integer.

/// Scaled points to the point.
pub const UNITY: i64 = 65536;

/// The largest dimension TeX will hold, `16383.99998pt` (§421).
pub const MAX_DIMEN: i64 = 0x3FFF_FFFF;

/// The exact ratio a unit bears to a point (`tex.web` §458).
///
/// Held as a ratio and applied with one multiplication and one division, so the
/// result is the integer TeX computes rather than whatever a float rounds to.
pub fn unit_ratio(unit: &str) -> Option<(i64, i64)> {
    Some(match unit {
        "pt" => (1, 1),
        "in" => (7227, 100),
        "pc" => (12, 1),
        "cm" => (7227, 254),
        "mm" => (7227, 2540),
        "bp" => (7227, 7200),
        "dd" => (1238, 1157),
        "cc" => (14856, 1157),
        // A scaled point is the unit the number is already in.
        "sp" => (0, 0),
        _ => return None,
    })
}

/// A run of decimal digits as a fraction of 65536 (`tex.web` §452).
///
/// Read back to front so the nearest sixteen-bit fraction comes out, rather
/// than a truncation: `.5` is exactly 32768 and `.1` is 6554, not 6553.
pub fn round_decimals(digits: &str) -> i64 {
    let mut f = 0i64;
    for digit in digits.bytes().rev().take(17) {
        f = (f + i64::from(digit.wrapping_sub(b'0')) * 0x2_0000) / 10;
    }
    (f + 1) / 2
}

/// `n` units, in scaled points, where `n` is given as an integer part and a
/// fraction already scaled to 65536ths.
///
/// The fraction is carried through the same ratio as the integer part, which is
/// what keeps `0.5in` and `1in`/2 the same number.
pub fn to_scaled(int: i64, frac: i64, unit: &str) -> Option<i64> {
    let (num, den) = unit_ratio(unit)?;
    if unit == "sp" {
        return Some(int);
    }
    let sp = int * UNITY + frac;
    // Multiply first, then divide: the ratio is exact and the product of a
    // dimension with a small numerator cannot leave i64.
    Some(sp.saturating_mul(num) / den)
}

/// A dimension as TeX writes it, without the unit (`tex.web` §103).
///
/// Ported rather than reinvented, because the rule it implements is not
/// "print with five decimals": it is "print the FEWEST digits that read back as
/// this integer", which is why 1pt is `1.0` and 1sp is `0.00002`.
pub fn print_scaled(mut s: i64) -> String {
    let mut out = String::new();
    if s < 0 {
        out.push('-');
        s = -s;
    }
    out.push_str(&(s / UNITY).to_string());
    out.push('.');
    // §103 exactly: ten times the remainder plus five, with the rounding
    // correction applied once delta has grown past a whole unit.
    s = 10 * (s % UNITY) + 5;
    let mut delta = 10i64;
    loop {
        if delta > UNITY {
            s += 0x8000 - 50000;
        }
        out.push(char::from_digit((s / UNITY) as u32, 10).unwrap_or('0'));
        s = 10 * (s % UNITY);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these was read off `tex -ini`, not computed here.
    #[test]
    fn units_reach_the_scaled_points_tex_computes() {
        for (unit, want) in [
            ("pt", "1.0"),
            ("in", "72.26999"),
            ("cm", "28.45274"),
            ("mm", "2.84526"),
            ("bp", "1.00374"),
            ("pc", "12.0"),
            ("dd", "1.07"),
            ("cc", "12.8401"),
        ] {
            let sp = to_scaled(1, 0, unit).expect("a known unit");
            assert_eq!(print_scaled(sp), want, "1{unit}");
        }
    }

    #[test]
    fn a_scaled_point_is_the_smallest_step() {
        assert_eq!(print_scaled(to_scaled(1, 0, "sp").expect("sp")), "0.00002");
        assert_eq!(print_scaled(UNITY), "1.0");
    }

    #[test]
    fn fractions_and_signs_print_as_tex_prints_them() {
        assert_eq!(
            print_scaled(to_scaled(0, UNITY / 2, "pt").expect("pt")),
            "0.5"
        );
        assert_eq!(print_scaled(-(3 * UNITY + UNITY / 4)), "-3.25");
    }
}
