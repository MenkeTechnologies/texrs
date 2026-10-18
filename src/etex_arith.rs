//! The arithmetic of e-TeX's `\numexpr` and `\dimexpr`, as luatex's
//! `scanning.c` does it (`add_or_sub`, `quotient`, `fract`, and tex.web §105's
//! `mult_and_add` behind `mult_integers` and `nx_plus_y`).
//!
//! One copy, used twice: by the lowerer, which folds an expression whose every
//! operand it knows, and by the run, which evaluates one that reads a register
//! (`crate::ir::Num::Expr`). An operation that leaves the range is
//! `num_error` there -- `arith_error` set, the value zero -- and the whole
//! expression then comes out as zero with `! Arithmetic overflow` (the end of
//! `scan_expr`). [`ERROR`] carries that through the steps that follow, so the
//! last one knows.

/// §109's `infinity`: the largest integer an integer expression may reach.
pub const INFINITY: i64 = 0x7FFF_FFFF;
/// §421's `max_dimen`: the largest dimension a dimension expression may reach.
pub const MAX_DIMEN: i64 = 0x3FFF_FFFF;
/// A value no step can produce: an operation that went out of range, and
/// every step after it.
pub const ERROR: i64 = i64::MIN;

/// One step of `scan_expr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `add_or_sub(x, y, max, false)`.
    Add,
    /// `add_or_sub(x, y, max, true)`.
    Sub,
    /// `mult_integers` for an integer expression (`max` is [`INFINITY`]),
    /// `nx_plus_y(t, f, 0)` for a dimension (`max` is [`MAX_DIMEN`]).
    Mul,
    /// `quotient(t, f)`: the quotient rounded half away from zero.
    Div,
    /// `fract(t, n, f, max)`: `t*n/f` rounded, a multiplication followed by a
    /// division taken as one operation so the product never overflows.
    Scale,
    /// The range check `scan_expr` makes on every factor it reads.
    Check,
    /// The magnitude, which `\ifabsnum` and `\ifabsdim` compare
    /// (conditional.c negates a negative side).
    Abs,
    // The glue steps of `\glueexpr` and `\muexpr`. A glue is four values --
    // its width, stretch and shrink, and the two orders packed as
    // `stretch_order * 4 + shrink_order` -- and a step answers ONE of them:
    // for these `max` is not a bound but which (0 to 3). See [`apply_glue`].
    /// The sum of two glues (`args`: the four of each).
    GlueAdd,
    /// Their difference.
    GlueSub,
    /// A glue times an integer (the four, then the integer).
    GlueMul,
    /// A glue divided by an integer.
    GlueDiv,
    /// A glue times `n/d` taken as one scaling (the four, then `n` and `d`).
    GlueScale,
    /// The range check on a glue factor (the four).
    GlueCheck,
    /// An order out of a glue's packed orders (`args`: the packed value): for
    /// this `max` says which -- 0 the stretch's and 1 the shrink's in e-TeX's
    /// numbering (`\eTeXgluestretchorder`), 2 and 3 the same in LuaTeX's,
    /// which counts its `fi` order and so is one more for every infinite
    /// order (`\gluestretchorder`).
    GlueOrder,
    /// The end of a glue expression for a value [`ERROR`] does not report:
    /// zero for it. One component of the four reports, and the rest settle.
    Settle,
}

impl Op {
    pub fn code(self) -> i64 {
        self as i64
    }

    pub fn from_code(code: i64) -> Option<Op> {
        [
            Op::Add,
            Op::Sub,
            Op::Mul,
            Op::Div,
            Op::Scale,
            Op::Check,
            Op::Abs,
            Op::GlueAdd,
            Op::GlueSub,
            Op::GlueMul,
            Op::GlueDiv,
            Op::GlueScale,
            Op::GlueCheck,
            Op::GlueOrder,
            Op::Settle,
        ]
        .into_iter()
        .find(|op| op.code() == code)
    }
}

/// Apply `op` to `args` under the bound `max`. [`ERROR`] in, [`ERROR`] out.
pub fn apply(op: Op, max: i64, args: &[i64]) -> i64 {
    if op == Op::Settle {
        return match args.first() {
            Some(&ERROR) | None => 0,
            Some(&v) => v,
        };
    }
    if args.contains(&ERROR) {
        return ERROR;
    }
    if matches!(
        op,
        Op::GlueAdd | Op::GlueSub | Op::GlueMul | Op::GlueDiv | Op::GlueScale | Op::GlueCheck
    ) {
        return apply_glue(op, max, args);
    }
    let arg = |i: usize| args.get(i).copied().unwrap_or(0);
    let v = match op {
        Op::Add => add_or_sub(arg(0), arg(1), max, false),
        Op::Sub => add_or_sub(arg(0), arg(1), max, true),
        Op::Mul => mult_and_add(arg(0), arg(1), max),
        Op::Div => quotient(arg(0), arg(1)),
        Op::Scale => fract(arg(0), arg(1), arg(2), max),
        Op::Check => (arg(0).abs() <= max).then_some(arg(0)),
        Op::Abs => Some(arg(0).abs()),
        Op::GlueOrder => {
            let order = match max % 2 {
                0 => arg(0) / 4,
                _ => arg(0) % 4,
            };
            Some(match max >= 2 && order > 0 {
                true => order + 1,
                false => order,
            })
        }
        _ => None,
    };
    v.unwrap_or(ERROR)
}

/// `add_or_sub`: the sum or difference, if its magnitude is at most `max`.
fn add_or_sub(x: i64, y: i64, max: i64, negative: bool) -> Option<i64> {
    let y = if negative { -y } else { y };
    let sum = x + y;
    (sum.abs() <= max).then_some(sum)
}

/// §105's `mult_and_add(n, x, 0, max)`: the product, if its magnitude is at
/// most `max`.
fn mult_and_add(n: i64, x: i64, max: i64) -> Option<i64> {
    let product = n.checked_mul(x)?;
    (product.abs() <= max).then_some(product)
}

/// `quotient(n, d)`: `n/d` rounded half away from zero; no answer for a zero
/// divisor.
fn quotient(n: i64, d: i64) -> Option<i64> {
    if d == 0 {
        return None;
    }
    let negative = (n < 0) != (d < 0);
    let (n, d) = (n.abs(), d.abs());
    let mut a = n / d;
    // The remainder against the divisor: half or more rounds up.
    if 2 * (n - a * d) >= d {
        a += 1;
    }
    Some(if negative { -a } else { a })
}

/// `fract(x, n, d, max)`: `x*n/d` rounded half up in magnitude, if it is at
/// most `max`. scanning.c simulates the double-length product in 32-bit
/// arithmetic; 128 bits hold it exactly, which is the same answer.
fn fract(x: i64, n: i64, d: i64, max: i64) -> Option<i64> {
    if d == 0 {
        return None;
    }
    let negative = ((x < 0) != (n < 0)) != (d < 0);
    let (x, n, d) = (x.abs() as i128, n.abs() as i128, d.abs() as i128);
    let product = x * n;
    let mut a = product / d;
    if 2 * (product - a * d) >= d {
        a += 1;
    }
    if a > max as i128 {
        return None;
    }
    let a = a as i64;
    Some(if negative { -a } else { a })
}

/// A glue at `\glueexpr`'s level.
pub type Glue = crate::glue::Glue;

/// `glue_error`: what a glue expression that went out of range comes to.
pub fn glue_error() -> Glue {
    Glue {
        natural: 0,
        stretch: 0,
        stretch_order: 0,
        shrink: 0,
        shrink_order: 0,
    }
}

/// `normalize_glue`: a component of zero has no order.
pub fn normalize_glue(mut g: Glue) -> Glue {
    if g.stretch == 0 {
        g.stretch_order = 0;
    }
    if g.shrink == 0 {
        g.shrink_order = 0;
    }
    g
}

/// The range check on a glue factor: each of its three widths at most
/// [`MAX_DIMEN`]; `None` is `glue_error`.
pub fn check_glue(g: Glue) -> Option<Glue> {
    [g.natural, g.stretch, g.shrink]
        .iter()
        .all(|v| v.abs() <= MAX_DIMEN)
        .then_some(g)
}

/// `op` (`Mul`, `Div` or `Scale`) on each of the three widths, as
/// `expr_m`, `expr_d` and `expr_s` do to `width`, `stretch` and `shrink`;
/// `None` if any of them leaves the range. `args` are the integer operands
/// after the glue (one, or the numerator and denominator of a scale).
pub fn scale_glue(op: Op, g: Glue, args: &[i64]) -> Option<Glue> {
    let each = |v: i64| {
        let mut operands = vec![v];
        operands.extend_from_slice(args);
        match apply(op, MAX_DIMEN, &operands) {
            ERROR => None,
            r => Some(r),
        }
    };
    Some(Glue {
        natural: each(g.natural)?,
        stretch: each(g.stretch)?,
        shrink: each(g.shrink)?,
        ..g
    })
}

/// The sum (or, `negative`, the difference) of two glue specs (luatex's
/// `scan_expr`, "Compute the sum or difference of two glue specs"): the
/// widths add; a stretch or shrink adds to one of the same order, and one of
/// a higher order that is not zero REPLACES it -- unnegated, even when it is
/// being subtracted, which is what the code does.
pub fn add_glue(e: Glue, t: Glue, negative: bool) -> Option<Glue> {
    let a = |x: i64, y: i64| add_or_sub(x, y, MAX_DIMEN, negative);
    let mut out = e;
    out.natural = a(e.natural, t.natural)?;
    if e.stretch_order == t.stretch_order {
        out.stretch = a(e.stretch, t.stretch)?;
    } else if e.stretch_order < t.stretch_order && t.stretch != 0 {
        out.stretch = t.stretch;
        out.stretch_order = t.stretch_order;
    }
    if e.shrink_order == t.shrink_order {
        out.shrink = a(e.shrink, t.shrink)?;
    } else if e.shrink_order < t.shrink_order && t.shrink != 0 {
        out.shrink = t.shrink;
        out.shrink_order = t.shrink_order;
    }
    Some(normalize_glue(out))
}

/// A glue step (`Op::GlueAdd` and its siblings) on the packed glues in
/// `args`, answering component `which` of the result; [`ERROR`] for every
/// component when the step leaves the range, as luatex's `glue_error` zeroes
/// the whole glue.
fn apply_glue(op: Op, which: i64, args: &[i64]) -> i64 {
    let unpack = |a: &[i64]| Glue {
        natural: a[0],
        stretch: a[1],
        shrink: a[2],
        stretch_order: a[3] / 4,
        shrink_order: a[3] % 4,
    };
    if args.len() < 4 {
        return ERROR;
    }
    let e = unpack(&args[..4]);
    let result = match op {
        Op::GlueAdd | Op::GlueSub if args.len() >= 8 => {
            add_glue(e, unpack(&args[4..8]), op == Op::GlueSub)
        }
        Op::GlueMul | Op::GlueDiv if args.len() >= 5 => {
            let int = if op == Op::GlueMul { Op::Mul } else { Op::Div };
            scale_glue(int, e, &args[4..5])
        }
        Op::GlueScale if args.len() >= 6 => scale_glue(Op::Scale, e, &args[4..6]),
        Op::GlueCheck => check_glue(e),
        _ => None,
    };
    let Some(g) = result else {
        return ERROR;
    };
    match which {
        0 => g.natural,
        1 => g.stretch,
        2 => g.shrink,
        _ => g.stretch_order * 4 + g.shrink_order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn division_rounds_half_away_from_zero() {
        // luatex: \numexpr 7/2 is 4, -7/2 is -4, 5/2 is 3, 6/4 is 2.
        assert_eq!(apply(Op::Div, INFINITY, &[7, 2]), 4);
        assert_eq!(apply(Op::Div, INFINITY, &[-7, 2]), -4);
        assert_eq!(apply(Op::Div, INFINITY, &[5, 2]), 3);
        assert_eq!(apply(Op::Div, INFINITY, &[6, 4]), 2);
        assert_eq!(apply(Op::Div, INFINITY, &[1, 0]), ERROR);
    }

    #[test]
    fn a_scale_never_overflows_in_the_middle() {
        // 2^30 * 3 overflows 32 bits; divided by 4 it fits.
        assert_eq!(apply(Op::Scale, INFINITY, &[1 << 30, 3, 4]), 805_306_368);
        assert_eq!(apply(Op::Scale, INFINITY, &[7, 11, 3]), 26);
        assert_eq!(apply(Op::Scale, INFINITY, &[-7, 11, 3]), -26);
        assert_eq!(apply(Op::Scale, MAX_DIMEN, &[MAX_DIMEN, 2, 1]), ERROR);
    }

    #[test]
    fn an_error_stays_an_error() {
        let over = apply(Op::Mul, INFINITY, &[INFINITY, 2]);
        assert_eq!(over, ERROR);
        assert_eq!(apply(Op::Add, INFINITY, &[over, 1]), ERROR);
        assert_eq!(apply(Op::Add, INFINITY, &[INFINITY, 0]), INFINITY);
        assert_eq!(apply(Op::Add, MAX_DIMEN, &[MAX_DIMEN, 1]), ERROR);
    }
}
