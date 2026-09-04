//! The options in a `[...]`, and what they change about the bytes drawn.
//!
//! TikZ's key list is enormous; this reads the part of it that changes the PDF
//! -- colour, line width, dash pattern, caps and joins, opacity, the canvas
//! transformation, the arrow spec, and the keys a node is placed by. A key it
//! does not know is left alone rather than guessed at, which is the same rule
//! the rest of this module works to.
//!
//! The numbers are PGF's own, read out of `tikz.code.tex` rather than
//! remembered: `thin` is 0.4pt and `ultra thick` is 1.6pt (lines 1575-1581),
//! and `dashed` is `on 3pt off 3pt` where `dotted` is `on \pgflinewidth off
//! 2pt` (lines 1583-1601). A dash pattern written from memory comes out as a
//! line that is dashed differently from the one the document asked for, which
//! is a difference a reader sees.

use crate::colour::{Colours, Rgb};

use super::coord;
use super::units;

/// PDF's line cap styles (S8.4.3.3): the shape drawn at an open end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

impl Cap {
    /// The operand PDF's `J` operator takes.
    pub fn code(self) -> u8 {
        match self {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        }
    }
}

/// PDF's line join styles (S8.4.3.4): the shape drawn where two segments meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

impl Join {
    /// The operand PDF's `j` operator takes.
    pub fn code(self) -> u8 {
        match self {
            Join::Miter => 0,
            Join::Round => 1,
            Join::Bevel => 2,
        }
    }
}

/// An arrow tip, by the name the document wrote (§16.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tip {
    /// `>` and `<`, PGF's `to`: two stroked curves meeting at a point.
    To,
    /// `stealth`: a filled concave quadrilateral.
    Stealth,
    /// `latex`: the filled tip LaTeX's own picture environment draws.
    Latex,
}

impl Tip {
    /// The tip a name in an arrow spec asks for, if this knows it.
    fn named(name: &str) -> Option<Tip> {
        match name {
            ">" | "<" | "to" | "To" => Some(Tip::To),
            "stealth" | "Stealth" => Some(Tip::Stealth),
            "latex" | "Latex" => Some(Tip::Latex),
            _ => None,
        }
    }
}

/// The canvas transformation, in the picture's own units.
///
/// TikZ applies `rotate`, `scale` and `shift` to the COORDINATES rather than
/// wrapping the path in a `cm` matrix -- `\draw[rotate=30,scale=2] (0,0) --
/// (2,0)` comes out of lualatex as `0.0 0.0 m 98.19649 56.69362 l`, which is
/// the rotated and scaled endpoint written out in full. This does the same, so
/// what is emitted is the same operators with the same operands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

impl Transform {
    /// Where this transformation sends a point.
    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    /// The same, without the translation -- for a vector rather than a point.
    pub fn apply_vector(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.a * x + self.c * y, self.b * x + self.d * y)
    }

    /// `self` and then `outer`, which is the order a scope nests in: the inner
    /// option list is applied first and the enclosing scope's on top of it.
    pub fn then(&self, outer: &Transform) -> Transform {
        Transform {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            tx: outer.a * self.tx + outer.c * self.ty + outer.tx,
            ty: outer.b * self.tx + outer.d * self.ty + outer.ty,
        }
    }
}

/// Which of a node's nine anchors sits on the coordinate (§17.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Center,
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl Anchor {
    /// The anchor a name asks for, PGF's own spelling.
    pub fn named(name: &str) -> Option<Anchor> {
        match name.trim() {
            "center" | "centre" => Some(Anchor::Center),
            "north" => Some(Anchor::North),
            "south" => Some(Anchor::South),
            "east" => Some(Anchor::East),
            "west" => Some(Anchor::West),
            "north east" => Some(Anchor::NorthEast),
            "north west" => Some(Anchor::NorthWest),
            "south east" => Some(Anchor::SouthEast),
            "south west" => Some(Anchor::SouthWest),
            _ => None,
        }
    }

    /// Which way the anchor lies from the node's centre, as a fraction of the
    /// node's half-width and half-height.
    pub fn offset(self) -> (f64, f64) {
        match self {
            Anchor::Center => (0.0, 0.0),
            Anchor::North => (0.0, 1.0),
            Anchor::South => (0.0, -1.0),
            Anchor::East => (1.0, 0.0),
            Anchor::West => (-1.0, 0.0),
            Anchor::NorthEast => (1.0, 1.0),
            Anchor::NorthWest => (-1.0, 1.0),
            Anchor::SouthEast => (1.0, -1.0),
            Anchor::SouthWest => (-1.0, -1.0),
        }
    }
}

/// The shape a node is drawn as (§17.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Rectangle,
    Circle,
}

/// Everything an option list can say about how a path is painted.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    /// The stroke colour, and whether the path is stroked at all.
    pub stroke: Rgb,
    pub draw: bool,
    /// The fill colour, and whether the path is filled at all.
    pub fill: Rgb,
    pub filled: bool,
    /// Stroke width in points -- `line width=`, or one of the named widths.
    pub width: f64,
    /// The dash pattern in points, on and off alternating. Empty is solid.
    pub dash: Vec<f64>,
    pub cap: Cap,
    pub join: Join,
    /// `draw opacity=` and `fill opacity=`, both in 0..=1.
    pub draw_opacity: f64,
    pub fill_opacity: f64,
    /// `even odd rule` -- which of PDF's two interior tests fills the path.
    pub even_odd: bool,
    pub arrow_start: Option<Tip>,
    pub arrow_end: Option<Tip>,
    pub transform: Transform,
    /// The colour a node's text is set in.
    pub text: Rgb,
    pub anchor: Anchor,
    pub shape: Shape,
    /// The space between a node's text and its border, in points (§17.2.2).
    pub inner_sep: f64,
    /// `minimum width` and `minimum height`, in points.
    pub minimum: (f64, f64),
    /// The size a node's text is set at, in points.
    pub font_size: f64,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            stroke: (0.0, 0.0, 0.0),
            draw: false,
            fill: (0.0, 0.0, 0.0),
            filled: false,
            // PGF's initial width, `tikz.code.tex` line 1577's `thin`.
            width: 0.4,
            dash: Vec::new(),
            cap: Cap::Butt,
            join: Join::Miter,
            draw_opacity: 1.0,
            fill_opacity: 1.0,
            even_odd: false,
            arrow_start: None,
            arrow_end: None,
            transform: Transform::default(),
            text: (0.0, 0.0, 0.0),
            anchor: Anchor::Center,
            shape: Shape::Rectangle,
            // `inner xsep/.initial = .3333em` -- pgfmoduleshapes line 888, at
            // the 10pt a picture in body text is set in.
            inner_sep: 3.333,
            minimum: (0.0, 0.0),
            font_size: 10.0,
        }
    }
}

impl Style {
    /// Read one option list into a copy of this style.
    ///
    /// The list is the text between the brackets. Options are applied left to
    /// right, so a later one wins -- which is what makes a scope's options an
    /// inherited default and a path's own an override.
    pub fn with(&self, options: &str, colours: &Colours) -> Style {
        let mut style = self.clone();
        for option in split(options) {
            style.apply(option.trim(), colours);
        }
        style
    }

    /// One `key=value`, or a bare key.
    fn apply(&mut self, option: &str, colours: &Colours) {
        let (key, value) = match option.split_once('=') {
            Some((key, value)) => (key.trim(), strip_braces(value.trim())),
            None => (option, ""),
        };
        match key {
            "draw" => {
                self.draw = true;
                if let Some(rgb) = colour(value, colours) {
                    self.stroke = rgb;
                }
            }
            "fill" => {
                self.filled = true;
                if let Some(rgb) = colour(value, colours) {
                    self.fill = rgb;
                }
            }
            "color" | "text" => {
                if let Some(rgb) = colour(value, colours) {
                    self.stroke = rgb;
                    self.fill = rgb;
                    self.text = rgb;
                }
            }
            "line width" => {
                if let Some(width) = units::dimension(value) {
                    self.width = width;
                }
            }
            // `tikz.code.tex` lines 1575-1581.
            "ultra thin" => self.width = 0.1,
            "very thin" => self.width = 0.2,
            "thin" => self.width = 0.4,
            "semithick" => self.width = 0.6,
            "thick" => self.width = 0.8,
            "very thick" => self.width = 1.2,
            "ultra thick" => self.width = 1.6,
            // `tikz.code.tex` lines 1583-1601. The `\pgflinewidth` in the
            // dotted patterns is the width in force, so it is read here and
            // not written as a constant.
            "solid" => self.dash.clear(),
            "dotted" => self.dash = vec![self.width, 2.0],
            "densely dotted" => self.dash = vec![self.width, 1.0],
            "loosely dotted" => self.dash = vec![self.width, 4.0],
            "dashed" => self.dash = vec![3.0, 3.0],
            "densely dashed" => self.dash = vec![3.0, 2.0],
            "loosely dashed" => self.dash = vec![3.0, 6.0],
            "dash dot" | "dashdotted" => self.dash = vec![3.0, 2.0, self.width, 2.0],
            "densely dash dot" | "densely dashdotted" => {
                self.dash = vec![3.0, 1.0, self.width, 1.0]
            }
            "loosely dash dot" | "loosely dashdotted" => {
                self.dash = vec![3.0, 4.0, self.width, 4.0]
            }
            "dash pattern" => self.dash = dash_pattern(value),
            "line cap" => {
                self.cap = match value {
                    "round" => Cap::Round,
                    "rect" => Cap::Square,
                    _ => Cap::Butt,
                }
            }
            "line join" => {
                self.join = match value {
                    "round" => Join::Round,
                    "bevel" => Join::Bevel,
                    _ => Join::Miter,
                }
            }
            "opacity" => {
                if let Some(alpha) = units::number(value) {
                    self.draw_opacity = alpha;
                    self.fill_opacity = alpha;
                }
            }
            "draw opacity" => {
                if let Some(alpha) = units::number(value) {
                    self.draw_opacity = alpha;
                }
            }
            "fill opacity" => {
                if let Some(alpha) = units::number(value) {
                    self.fill_opacity = alpha;
                }
            }
            "even odd rule" => self.even_odd = true,
            "nonzero rule" => self.even_odd = false,
            "rotate" => {
                if let Some(degrees) = units::number(value) {
                    let (sin, cos) = degrees.to_radians().sin_cos();
                    self.compose(Transform {
                        a: cos,
                        b: sin,
                        c: -sin,
                        d: cos,
                        tx: 0.0,
                        ty: 0.0,
                    });
                }
            }
            "scale" => {
                if let Some(factor) = units::number(value) {
                    self.compose(Transform {
                        a: factor,
                        d: factor,
                        ..Transform::default()
                    });
                }
            }
            "xscale" => {
                if let Some(factor) = units::number(value) {
                    self.compose(Transform {
                        a: factor,
                        ..Transform::default()
                    });
                }
            }
            "yscale" => {
                if let Some(factor) = units::number(value) {
                    self.compose(Transform {
                        d: factor,
                        ..Transform::default()
                    });
                }
            }
            "xshift" => self.shift(units::dimension(value).unwrap_or(0.0), 0.0),
            "yshift" => self.shift(0.0, units::dimension(value).unwrap_or(0.0)),
            "shift" => {
                let inside = value.trim().trim_start_matches('(').trim_end_matches(')');
                if let Some(point) = coord::parse(inside) {
                    let frame = coord::Frame::new(1.0, 1.0);
                    let (x, y) = point.resolve(&frame);
                    self.shift(x, y);
                }
            }
            "anchor" => {
                if let Some(anchor) = Anchor::named(value) {
                    self.anchor = anchor;
                }
            }
            // The placement keys are anchors written the other way round: a
            // node ABOVE its coordinate has its SOUTH on it. `tikz.code.tex`
            // line 1008 is `\tikzoption{above}[]{\def\tikz@anchor{south}...}`.
            "above" => self.anchor = Anchor::South,
            "below" => self.anchor = Anchor::North,
            "left" => self.anchor = Anchor::East,
            "right" => self.anchor = Anchor::West,
            "above left" => self.anchor = Anchor::SouthEast,
            "above right" => self.anchor = Anchor::SouthWest,
            "below left" => self.anchor = Anchor::NorthEast,
            "below right" => self.anchor = Anchor::NorthWest,
            "rectangle" => self.shape = Shape::Rectangle,
            "circle" => self.shape = Shape::Circle,
            "shape" => {
                self.shape = match value {
                    "circle" => Shape::Circle,
                    _ => Shape::Rectangle,
                }
            }
            "inner sep" => {
                if let Some(sep) = units::dimension(value) {
                    self.inner_sep = sep;
                }
            }
            "minimum size" => {
                if let Some(size) = units::dimension(value) {
                    self.minimum = (size, size);
                }
            }
            "minimum width" => {
                if let Some(size) = units::dimension(value) {
                    self.minimum.0 = size;
                }
            }
            "minimum height" => {
                if let Some(size) = units::dimension(value) {
                    self.minimum.1 = size;
                }
            }
            "font size" => {
                if let Some(size) = units::dimension(value) {
                    self.font_size = size;
                }
            }
            other => self.bare(other, colours),
        }
    }

    /// A key with no `=`: an arrow spec, or a colour named on its own.
    fn bare(&mut self, key: &str, colours: &Colours) {
        if let Some((start, end)) = arrow_spec(key) {
            self.arrow_start = start;
            self.arrow_end = end;
            return;
        }
        // TikZ lets a colour be written as a bare option -- `\draw[red]` and
        // `\draw[color=red]` are the same path, and the corpus writes the
        // first. A name nothing defined is left alone rather than made black.
        if let Some(rgb) = colour(key, colours) {
            self.stroke = rgb;
            self.fill = rgb;
            self.text = rgb;
        }
    }

    fn compose(&mut self, inner: Transform) {
        self.transform = inner.then(&self.transform);
    }

    fn shift(&mut self, x: f64, y: f64) {
        self.compose(Transform {
            tx: x,
            ty: y,
            ..Transform::default()
        });
    }
}

/// An arrow spec: `->`, `<-`, `<->`, `-stealth`, `latex-latex` (§16.1).
///
/// Returns the tip at each end, or nothing if this is not an arrow spec at all
/// -- which matters, because `even odd rule` has no dash in it but `dash dot`
/// does, and reading a style name as an arrow would drop the style.
pub fn arrow_spec(key: &str) -> Option<(Option<Tip>, Option<Tip>)> {
    let key = key.trim();
    let at = key.find('-')?;
    let (start, end) = (&key[..at], &key[at + 1..]);
    let tip = |name: &str| match name.is_empty() {
        true => Some(None),
        false => Tip::named(name).map(Some),
    };
    Some((tip(start)?, tip(end)?))
}

/// `on 3pt off 3pt` as the alternating lengths PDF's `d` operator wants.
fn dash_pattern(value: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut words = value.split_whitespace();
    while let Some(word) = words.next() {
        if matches!(word, "on" | "off") {
            if let Some(length) = words.next().and_then(units::dimension) {
                out.push(length);
            }
        }
    }
    out
}

/// A colour name, with xcolor's `!` mixing if it carries any.
///
/// `red!50` is half red and half white, and `red!30!blue` is thirty percent
/// red mixed into blue -- xcolor's own rule, and the one a TikZ picture writes
/// a tint with.
pub fn colour(spec: &str, colours: &Colours) -> Option<Rgb> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let mut parts = spec.split('!');
    let mut current = colours.get(parts.next()?)?;
    while let Some(percent) = parts.next() {
        let fraction = units::number(percent)? / 100.0;
        let against = match parts.next() {
            Some(name) => colours.get(name)?,
            None => (1.0, 1.0, 1.0),
        };
        current = (
            current.0 * fraction + against.0 * (1.0 - fraction),
            current.1 * fraction + against.1 * (1.0 - fraction),
            current.2 * fraction + against.2 * (1.0 - fraction),
        );
    }
    Some(current)
}

/// The commas of an option list that are not inside a `{...}` or a `(...)`.
///
/// `shift={(1,2)}` has a comma in it that does not end the option, and
/// splitting on every comma would leave `shift={(1` behind.
pub fn split(options: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (at, ch) in options.char_indices() {
        match ch {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&options[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&options[start..]);
    out.into_iter().filter(|o| !o.trim().is_empty()).collect()
}

/// `{value}` written for a value that carries a comma.
fn strip_braces(value: &str) -> &str {
    match value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
        Some(inner) => inner.trim(),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(options: &str) -> Style {
        Style::default().with(options, &Colours::new())
    }

    #[test]
    fn the_named_widths_are_pgfs_own_numbers() {
        // tikz.code.tex lines 1575-1581. A remembered value here is a line
        // drawn at a width the document did not ask for.
        assert_eq!(style("ultra thin").width, 0.1);
        assert_eq!(style("thin").width, 0.4);
        assert_eq!(style("thick").width, 0.8);
        assert_eq!(style("ultra thick").width, 1.6);
        assert_eq!(style("line width=1.52pt").width, 1.52);
    }

    #[test]
    fn the_dash_patterns_are_pgfs_own_numbers() {
        // tikz.code.tex lines 1584-1589. lualatex writes `[ 2.98883 2.98883 ]
        // 0.0 d` for `dashed`, which is 3pt each in big points.
        assert_eq!(style("dashed").dash, vec![3.0, 3.0]);
        assert_eq!(style("densely dashed").dash, vec![3.0, 2.0]);
        assert_eq!(style("loosely dashed").dash, vec![3.0, 6.0]);
        // `dotted` is on \pgflinewidth off 2pt, so the width in force is part
        // of the pattern -- lualatex writes `[ 0.99628 1.99255 ]` at 1pt.
        assert_eq!(style("line width=1pt,dotted").dash, vec![1.0, 2.0]);
        assert_eq!(style("dotted").dash, vec![0.4, 2.0]);
        assert_eq!(style("dashed,solid").dash, Vec::<f64>::new());
        assert_eq!(style("dash pattern=on 4pt off 1pt").dash, vec![4.0, 1.0]);
    }

    #[test]
    fn an_arrow_spec_is_told_apart_from_a_style_name() {
        assert_eq!(arrow_spec("->"), Some((None, Some(Tip::To))));
        assert_eq!(arrow_spec("<-"), Some((Some(Tip::To), None)));
        assert_eq!(arrow_spec("<->"), Some((Some(Tip::To), Some(Tip::To))));
        assert_eq!(arrow_spec("-stealth"), Some((None, Some(Tip::Stealth))));
        assert_eq!(arrow_spec("latex-latex"), Some((Some(Tip::Latex), Some(Tip::Latex))));
        // `dash dot` has no hyphen and `even-odd` is not a tip name: neither
        // is an arrow, and reading either as one loses the style it names.
        assert_eq!(arrow_spec("dash dot"), None);
        assert_eq!(arrow_spec("even-odd"), None);
    }

    #[test]
    fn a_colour_reaches_the_style_by_every_spelling() {
        let red = (1.0, 0.0, 0.0);
        assert_eq!(style("red").stroke, red);
        assert_eq!(style("color=red").stroke, red);
        assert_eq!(style("draw=red").stroke, red);
        assert_eq!(style("fill=red").fill, red);
        // `draw=` also turns drawing ON, which is what makes `\path[draw=red]`
        // a stroked path.
        assert!(style("draw=red").draw);
        assert!(style("fill=blue").filled);
        // xcolor's mixing: half red is half way to white.
        assert_eq!(style("red!50").stroke, (1.0, 0.5, 0.5));
        assert_eq!(style("red!0!blue").stroke, (0.0, 0.0, 1.0));
        // A name nothing defined leaves the colour alone rather than blacking
        // it out, which is what an unknown key must do.
        assert_eq!(style("draw=red,nosuchcolour").stroke, red);
    }

    #[test]
    fn a_transform_lands_where_lualatex_puts_it() {
        // `\draw[rotate=30,scale=2] (0,0) -- (2,0)` comes out of lualatex as
        // `98.19649 56.69362 l` for a picture in centimetres, which is the
        // point (2,0) scaled by two and turned through thirty degrees.
        let style = style("rotate=30,scale=2");
        let (x, y) = style.transform.apply((2.0, 0.0));
        assert!((x - 4.0 * 30f64.to_radians().cos()).abs() < 1e-9, "{x}");
        assert!((y - 4.0 * 30f64.to_radians().sin()).abs() < 1e-9, "{y}");
        // A shift is a translation and does not scale what follows it.
        let style = style.clone();
        let shifted = Style::default().with("xshift=3pt", &Colours::new());
        assert_eq!(shifted.transform.apply((1.0, 1.0)), (4.0, 1.0));
        let _ = style;
    }

    #[test]
    fn an_option_value_may_carry_its_own_comma() {
        // Splitting on every comma would leave `shift={(1` as a key.
        assert_eq!(split("a,shift={(1,2)},b"), vec!["a", "shift={(1,2)}", "b"]);
        let style = style("shift={(1,2)}");
        assert_eq!(style.transform.apply((0.0, 0.0)), (1.0, 2.0));
    }
}
