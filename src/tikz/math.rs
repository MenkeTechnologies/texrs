//! `\pgfmath`: the arithmetic a coordinate may be written as (§89).
//!
//! `(2*\i, {sin(30)})` is a coordinate, and reading only its literal numbers
//! puts it at the origin. PGF hands every coordinate component to its own
//! parser, so `\draw (0,0) -- (3*2, sqrt(9))` is a line to (6,3) and not a
//! line that could not be read.
//!
//! What is here is the operators and the functions a picture writes in a
//! coordinate: `+ - * / ^`, unary minus, parentheses, and the function table
//! of `pgfmathfunctions.*.code.tex`. The trigonometric ones take DEGREES,
//! which is PGF's own convention (`pgfmathfunctions.trigonometric.code.tex`:
//! `\pgfmathsin@` is documented as taking degrees) -- reading them as radians
//! puts `sin(30)` at -0.988 instead of 0.5, which is a point in the wrong
//! half of the picture.
//!
//! A number written with a unit keeps it: `2*1cm` is two centimetres, because
//! the unit multiplies through. An expression this cannot read comes back as
//! nothing rather than as zero, so an unreadable coordinate drops its path
//! rather than pinning it to the origin.

use super::units::{self, Length};

/// The value of an expression, in points if anything in it had a unit.
pub fn length(text: &str) -> Option<Length> {
    let mut parser = Parser { rest: text.trim() };
    let value = parser.sum()?;
    match parser.rest.trim().is_empty() {
        true => Some(value),
        false => None,
    }
}

/// The value of an expression as a plain number.
pub fn eval(text: &str) -> Option<f64> {
    length(text).map(|value| value.value)
}

/// Where the reader is in the expression.
struct Parser<'a> {
    rest: &'a str,
}

impl Parser<'_> {
    /// `a + b - c`, lowest precedence.
    fn sum(&mut self) -> Option<Length> {
        let mut left = self.product()?;
        loop {
            self.skip();
            let operator = match self.rest.chars().next() {
                Some(c @ ('+' | '-')) => c,
                _ => return Some(left),
            };
            self.rest = &self.rest[1..];
            let right = self.product()?;
            left = combine(left, right, |a, b| match operator {
                '+' => a + b,
                _ => a - b,
            });
        }
    }

    /// `a * b / c`.
    fn product(&mut self) -> Option<Length> {
        let mut left = self.power()?;
        loop {
            self.skip();
            let operator = match self.rest.chars().next() {
                Some(c @ ('*' | '/')) => c,
                _ => return Some(left),
            };
            self.rest = &self.rest[1..];
            let right = self.power()?;
            if operator == '/' && right.value == 0.0 {
                return None;
            }
            left = combine(left, right, |a, b| match operator {
                '*' => a * b,
                _ => a / b,
            });
        }
    }

    /// `a ^ b`, which binds tighter than a product and to the right.
    fn power(&mut self) -> Option<Length> {
        let base = self.unary()?;
        self.skip();
        let Some(rest) = self.rest.strip_prefix('^') else {
            return Some(base);
        };
        self.rest = rest;
        let exponent = self.power()?;
        Some(Length {
            value: base.value.powf(exponent.value),
            points: None,
        })
    }

    /// `-a`, and `+a` for symmetry.
    fn unary(&mut self) -> Option<Length> {
        self.skip();
        if let Some(rest) = self.rest.strip_prefix('-') {
            self.rest = rest;
            let value = self.unary()?;
            return Some(Length {
                value: -value.value,
                points: value.points.map(|points| -points),
            });
        }
        if let Some(rest) = self.rest.strip_prefix('+') {
            self.rest = rest;
            return self.unary();
        }
        self.atom()
    }

    /// A number, a bracketed expression, a constant or a function call.
    fn atom(&mut self) -> Option<Length> {
        self.skip();
        if let Some(rest) = self.rest.strip_prefix('(') {
            self.rest = rest;
            let value = self.sum()?;
            self.skip();
            self.rest = self.rest.strip_prefix(')')?;
            return Some(value);
        }
        // `{...}` is how a coordinate hides a comma from the reader that cut
        // it into components, and the braces are not part of the sum.
        if let Some(rest) = self.rest.strip_prefix('{') {
            self.rest = rest;
            let value = self.sum()?;
            self.skip();
            self.rest = self.rest.strip_prefix('}')?;
            return Some(value);
        }
        if let Some((number, rest)) = units::scan(self.rest) {
            self.rest = rest;
            return Some(number);
        }
        let name: String = self
            .rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        if name.is_empty() {
            return None;
        }
        self.rest = &self.rest[name.len()..];
        self.skip();
        // A name with no arguments is a constant.
        let Some(rest) = self.rest.strip_prefix('(') else {
            return constant(&name).map(|value| Length {
                value,
                points: None,
            });
        };
        self.rest = rest;
        let mut arguments = Vec::new();
        loop {
            arguments.push(self.sum()?.value);
            self.skip();
            match self.rest.chars().next() {
                Some(',') => self.rest = &self.rest[1..],
                Some(')') => {
                    self.rest = &self.rest[1..];
                    break;
                }
                _ => return None,
            }
        }
        call(&name, &arguments).map(|value| Length {
            value,
            points: None,
        })
    }

    fn skip(&mut self) {
        self.rest = self.rest.trim_start();
    }
}

/// Two operands, keeping the unit if either had one.
///
/// `2*1cm` is a length and `2*3` is a number, which is the difference between
/// a coordinate that ignores the picture's scale and one that does not.
fn combine(a: Length, b: Length, operation: impl Fn(f64, f64) -> f64) -> Length {
    match (a.points, b.points) {
        (None, None) => Length {
            value: operation(a.value, b.value),
            points: None,
        },
        // A length times a number keeps the length: `2*1cm` is 2cm, so the
        // operation runs on the POINTS and the bare number stands in for
        // itself on the other side.
        _ => {
            let (x, y) = (a.points.unwrap_or(a.value), b.points.unwrap_or(b.value));
            Length {
                value: operation(a.value, b.value),
                points: Some(operation(x, y)),
            }
        }
    }
}

/// `pi` and `e`, the two constants a coordinate is written with.
fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        _ => None,
    }
}

/// `pgfmath`'s function table, as far as a coordinate uses it.
///
/// The angles are in degrees both ways round -- `sin`, `cos` and `tan` take
/// them and `asin`, `acos`, `atan` and `atan2` give them back.
fn call(name: &str, arguments: &[f64]) -> Option<f64> {
    let one = arguments.first().copied();
    let two = arguments.get(1).copied();
    match (name, arguments.len()) {
        ("sin", 1) => Some(one?.to_radians().sin()),
        ("cos", 1) => Some(one?.to_radians().cos()),
        ("tan", 1) => Some(one?.to_radians().tan()),
        ("asin", 1) => Some(one?.asin().to_degrees()),
        ("acos", 1) => Some(one?.acos().to_degrees()),
        ("atan", 1) => Some(one?.atan().to_degrees()),
        ("atan2", 2) => Some(two?.atan2(one?).to_degrees()),
        ("sqrt", 1) => Some(one?.sqrt()),
        ("abs", 1) => Some(one?.abs()),
        ("exp", 1) => Some(one?.exp()),
        ("ln", 1) => Some(one?.ln()),
        ("log10", 1) => Some(one?.log10()),
        ("log2", 1) => Some(one?.log2()),
        ("pow", 2) => Some(one?.powf(two?)),
        ("min", 2) => Some(one?.min(two?)),
        ("max", 2) => Some(one?.max(two?)),
        ("mod", 2) => Some(one? % two?),
        ("round", 1) => Some(one?.round()),
        ("floor", 1) => Some(one?.floor()),
        ("ceil", 1) => Some(one?.ceil()),
        ("int", 1) => Some(one?.trunc()),
        ("veclen", 2) => Some(one?.hypot(two?)),
        // A function this does not know is not guessed at: the coordinate
        // comes back unreadable, and its path is dropped rather than drawn
        // through a point nobody asked for.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_binds_the_way_it_reads() {
        assert_eq!(eval("2*3"), Some(6.0));
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("(2+3)*4"), Some(20.0));
        assert_eq!(eval("-2^2"), Some(4.0), "unary minus binds first");
        assert_eq!(eval("2^3^2"), Some(512.0), "and the power to the right");
        assert_eq!(eval("10/4"), Some(2.5));
        assert_eq!(eval("1/0"), None, "which is not a number");
    }

    #[test]
    fn the_trigonometric_functions_take_degrees() {
        // PGF's own convention. In radians `sin(30)` is -0.988, which is a
        // point in the wrong half of the picture.
        let sin = eval("sin(30)").unwrap();
        assert!((sin - 0.5).abs() < 1e-12, "{sin}");
        let cos = eval("cos(60)").unwrap();
        assert!((cos - 0.5).abs() < 1e-12, "{cos}");
        assert_eq!(eval("sqrt(9)"), Some(3.0));
        assert_eq!(eval("veclen(3,4)"), Some(5.0));
        assert_eq!(eval("max(2,7)"), Some(7.0));
    }

    #[test]
    fn a_unit_multiplies_through_and_is_kept() {
        // `2*1cm` is two centimetres, and losing the unit would make it two
        // of the picture's own units instead.
        let centimetres = length("2*1cm").unwrap();
        assert!((centimetres.points.unwrap() - 2.0 * 72.27 / 2.54).abs() < 1e-9);
        // A sum with no unit in it stays unitless.
        assert_eq!(length("2+3").unwrap().points, None);
    }

    #[test]
    fn what_is_not_an_expression_is_not_read_as_one() {
        assert_eq!(eval("north east"), None);
        assert_eq!(eval("cycle"), None);
        assert_eq!(eval("nosuchfunction(2)"), None);
        assert_eq!(eval("2*"), None);
        // A bare name that is not a constant is not zero.
        assert_eq!(eval("a"), None);
    }
}
