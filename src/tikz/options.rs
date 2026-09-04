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
///
/// `rectangle` and `circle` are PGF's own (`pgfmoduleshapes.code.tex`);
/// `ellipse` and `diamond` come from `shapes.geometric`. A `\coordinate` is a
/// shape too -- one with no extent, whose every anchor is the point itself,
/// which is what makes `(a.north)` on a coordinate the coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Rectangle,
    Circle,
    Ellipse,
    Diamond,
    Coordinate,
}

impl Shape {
    /// The shape a name asks for, if this knows it.
    pub fn named(name: &str) -> Option<Shape> {
        match name.trim() {
            "rectangle" => Some(Shape::Rectangle),
            "circle" => Some(Shape::Circle),
            "ellipse" => Some(Shape::Ellipse),
            "diamond" => Some(Shape::Diamond),
            "coordinate" => Some(Shape::Coordinate),
            _ => None,
        }
    }
}

/// Which of PGF's shadings a `\shade` paints (§15.5).
///
/// The three come out of `tikz.code.tex` lines 628-654, colour stop by colour
/// stop; `shading::Ramp` is where those numbers are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shading {
    /// `axis` -- a band of colour along one direction. `top color` and
    /// `bottom color` set it at angle 0, `left color` and `right color` at 90.
    #[default]
    Axis,
    /// `radial` -- rings out from the middle, `inner color` to `outer color`.
    Radial,
    /// `ball` -- the radial shading `ball color` lights from the upper left.
    Ball,
}

/// A decoration, by the name `decoration={...}` gave it (§24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    /// `snake` -- a wave along the path
    /// (`pgflibrarydecorations.pathmorphing.code.tex` line 162).
    Snake,
    /// `zigzag` -- straight lines up and down (line 31).
    Zigzag,
    /// `saw` -- a rising edge and a vertical drop (line 64).
    Saw,
    /// `brace` -- the curly brace `pathreplacing` draws (line 140).
    Brace,
}

impl Decoration {
    /// The decoration a name asks for.
    pub fn named(name: &str) -> Option<Decoration> {
        match name.trim() {
            "snake" => Some(Decoration::Snake),
            "zigzag" => Some(Decoration::Zigzag),
            "saw" => Some(Decoration::Saw),
            "brace" => Some(Decoration::Brace),
            _ => None,
        }
    }
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
    /// The space between a node's text and its border, in points, on each
    /// axis -- `inner xsep` and `inner ysep` (§17.2.2).
    pub inner_sep: (f64, f64),
    /// `outer sep`, in points, or `None` for PGF's `.5\pgflinewidth`
    /// (`pgfmoduleshapes.code.tex` lines 891-892), which depends on the line
    /// width in force and so cannot be a number here.
    pub outer_sep: Option<f64>,
    /// `minimum width` and `minimum height`, in points.
    pub minimum: (f64, f64),
    /// `\pgfshapeaspect`, which the diamond's proportions come from
    /// (`pgflibraryshapes.geometric.code.tex` line 213: initially 1).
    pub aspect: f64,
    /// The size a node's text is set at, in points.
    pub font_size: f64,
    /// `rounded corners=` -- the radius the corners of a path are cut back
    /// and arced by, or zero for `sharp corners` (`tikz.code.tex` lines
    /// 282-283, where the default is 4pt).
    pub rounded: f64,
    /// What a `\shade` paints, if any option asked for one.
    pub shade: Option<Shade>,
    /// `decorate` with a `decoration=`, and the two lengths every decoration
    /// in `pathmorphing` is written in terms of.
    pub decoration: Option<Decoration>,
    /// `decorate`, which is the key that actually applies it -- a
    /// `decoration=` on its own only names one.
    pub decorate: bool,
    pub segment_length: f64,
    pub amplitude: f64,
    /// `aspect`, which the brace's shoulder sits at.
    pub decoration_aspect: f64,
    /// `label=` -- the direction a second node goes in, in degrees, and what
    /// it says.
    pub label: Option<(f64, String)>,
    /// How far a label stands off the border it labels (`label distance`,
    /// initially 0pt).
    pub label_distance: f64,
    /// `pattern=` -- a tiling pattern nothing here writes, which turns the
    /// fill OFF rather than filling the area with the fill colour.
    pub pattern: bool,
}

/// What a `\shade` paints: which shading, turned how far, in what colours.
///
/// The colours are the ones `tikz.code.tex` lines 602-623 set, and the
/// defaults on lines 635-654 -- a picture that writes `\shade` and no colour
/// at all gets a grey-to-white axis shading, not nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    pub kind: Shading,
    /// `shading angle`, in degrees.
    pub angle: f64,
    /// `top color` and `bottom color`, which `left`/`right` set as well --
    /// `left color` is the top of a shading turned through 90 degrees.
    pub top: Rgb,
    pub bottom: Rgb,
    /// `middle color`, which every other axis key recomputes as the even mix
    /// of the two ends (lines 604, 608, 615, 619) until one sets it outright.
    pub middle: Rgb,
    pub middle_set: bool,
    pub inner: Rgb,
    pub outer: Rgb,
    pub ball: Rgb,
}

impl Default for Shade {
    fn default() -> Shade {
        // `tikz.code.tex` lines 635-637, 646, 653-654: gray, half gray, white
        // for the axis; blue for a ball; gray to white for the radial.
        let gray = (0.5, 0.5, 0.5);
        let white = (1.0, 1.0, 1.0);
        Shade {
            kind: Shading::Axis,
            angle: 0.0,
            top: gray,
            bottom: white,
            middle: (0.75, 0.75, 0.75),
            middle_set: false,
            inner: gray,
            outer: white,
            ball: (0.0, 0.0, 1.0),
        }
    }
}

impl Shade {
    /// Put the middle colour back half way between the ends, which is what
    /// every one of the four axis keys does after setting its own end.
    fn remix(&mut self) {
        if !self.middle_set {
            self.middle = (
                (self.top.0 + self.bottom.0) / 2.0,
                (self.top.1 + self.bottom.1) / 2.0,
                (self.top.2 + self.bottom.2) / 2.0,
            );
        }
    }
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
            inner_sep: (3.333, 3.333),
            outer_sep: None,
            minimum: (0.0, 0.0),
            // `pgflibraryshapes.geometric.code.tex` line 232.
            aspect: 1.0,
            font_size: 10.0,
            rounded: 0.0,
            shade: None,
            decoration: None,
            decorate: false,
            // `pgfmoduledecorations.code.tex` lines 41-44.
            segment_length: 10.0,
            amplitude: 2.5,
            decoration_aspect: 0.5,
            label: None,
            label_distance: 0.0,
            pattern: false,
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
            // `\tikzoption{draw}` and `\tikzoption{fill}` (tikz.code.tex lines
            // 507-532) each test their argument against `none` FIRST: `none`
            // turns the mode off, and anything else -- a colour or nothing at
            // all -- turns it on and sets the colour if one was named. So
            // `\draw[fill=none]` is a path that is stroked and not filled, and
            // reading `none` as "a colour I do not know" left it filled black.
            "draw" => match value == "none" {
                true => self.draw = false,
                false => {
                    self.draw = true;
                    if let Some(rgb) = colour(value, colours) {
                        self.stroke = rgb;
                    }
                }
            },
            "fill" => match value == "none" {
                true => self.filled = false,
                false => {
                    self.filled = true;
                    if let Some(rgb) = colour(value, colours) {
                        self.fill = rgb;
                    }
                }
            },
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
            "rectangle" | "circle" | "ellipse" | "diamond" => {
                if let Some(shape) = Shape::named(key) {
                    self.shape = shape;
                }
            }
            "shape" => {
                if let Some(shape) = Shape::named(value) {
                    self.shape = shape;
                }
            }
            "aspect" => {
                if let Some(ratio) = units::number(value) {
                    self.aspect = ratio;
                }
            }
            "inner sep" => {
                if let Some(sep) = units::dimension(value) {
                    self.inner_sep = (sep, sep);
                }
            }
            "inner xsep" => {
                if let Some(sep) = units::dimension(value) {
                    self.inner_sep.0 = sep;
                }
            }
            "inner ysep" => {
                if let Some(sep) = units::dimension(value) {
                    self.inner_sep.1 = sep;
                }
            }
            "outer sep" => self.outer_sep = units::dimension(value),
            // `tikz.code.tex` lines 282-283: the radius defaults to 4pt, and
            // `sharp corners` is the same key set back to nothing.
            "rounded corners" => {
                self.rounded = units::dimension(value).unwrap_or(4.0);
            }
            "sharp corners" => self.rounded = 0.0,
            // The shading keys, `tikz.code.tex` lines 600-623. Each of them
            // turns shading ON as well as setting its colour, which is what
            // makes `\shade[left color=red]` a complete instruction.
            "shading" => {
                let shade = self.shade.get_or_insert_with(Shade::default);
                shade.kind = match value {
                    "radial" => Shading::Radial,
                    "ball" => Shading::Ball,
                    _ => Shading::Axis,
                };
            }
            "shading angle" => {
                if let Some(angle) = units::number(value) {
                    self.shade.get_or_insert_with(Shade::default).angle = angle;
                }
            }
            "top color" | "bottom color" | "left color" | "right color" | "middle color" => {
                if let Some(rgb) = colour(value, colours) {
                    let shade = self.shade.get_or_insert_with(Shade::default);
                    shade.kind = Shading::Axis;
                    match key {
                        "top color" => shade.top = rgb,
                        "bottom color" => shade.bottom = rgb,
                        // `left color` sets the shading's TOP and turns the
                        // whole thing through a right angle -- line 616.
                        "left color" => {
                            shade.top = rgb;
                            shade.angle = 90.0;
                        }
                        "right color" => {
                            shade.bottom = rgb;
                            shade.angle = 90.0;
                        }
                        _ => {
                            shade.middle = rgb;
                            shade.middle_set = true;
                        }
                    }
                    if matches!(key, "top color" | "bottom color") {
                        shade.angle = 0.0;
                    }
                    shade.remix();
                }
            }
            "inner color" | "outer color" => {
                if let Some(rgb) = colour(value, colours) {
                    let shade = self.shade.get_or_insert_with(Shade::default);
                    shade.kind = Shading::Radial;
                    match key {
                        "inner color" => shade.inner = rgb,
                        _ => shade.outer = rgb,
                    }
                }
            }
            "ball color" => {
                if let Some(rgb) = colour(value, colours) {
                    let shade = self.shade.get_or_insert_with(Shade::default);
                    shade.kind = Shading::Ball;
                    shade.ball = rgb;
                }
            }
            // `decorate` says to apply one; `decoration=` says which, and
            // carries the lengths it is drawn with.
            "decorate" => self.decorate = true,
            // A pattern is a tiling pattern in the page's `/Pattern`
            // resource, painted through the `/Pattern` colour space (PDF
            // 32000-1 S8.7.3), and nothing here writes one. Filling the path
            // with the fill colour instead would put a solid block where the
            // document asked for hatching, so the fill is turned OFF: the
            // area comes out empty rather than black.
            "pattern" | "pattern color" => self.pattern = true,
            "decoration" => self.decoration(value),
            "label" => self.label = label(value),
            "label distance" => {
                if let Some(distance) = units::dimension(value) {
                    self.label_distance = distance;
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

    /// `decoration={snake, amplitude=2mm, segment length=3mm}` (§24.1).
    ///
    /// The keys are `pgfmoduledecorations.code.tex` lines 48-62; the name may
    /// be written bare or as `name=`, which is what `\pgfdeclaredecoration`
    /// registers it under either way.
    fn decoration(&mut self, value: &str) {
        for option in split(value) {
            let (key, inner) = match option.split_once('=') {
                Some((key, inner)) => (key.trim(), strip_braces(inner.trim())),
                None => (option.trim(), ""),
            };
            match key {
                "name" => self.decoration = Decoration::named(inner),
                "amplitude" => {
                    if let Some(length) = units::dimension(inner) {
                        self.amplitude = length;
                    }
                }
                "segment length" => {
                    if let Some(length) = units::dimension(inner) {
                        self.segment_length = length;
                    }
                }
                "aspect" => {
                    if let Some(ratio) = units::number(inner) {
                        self.decoration_aspect = ratio;
                    }
                }
                // A bare word is the decoration's name. One this does not
                // know leaves the path undecorated rather than decorated with
                // something else.
                other => {
                    if let Some(decoration) = Decoration::named(other) {
                        self.decoration = Some(decoration);
                    }
                }
            }
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

    /// The outer separation in force: what `outer sep=` said, or half the
    /// line width, which is what `pgfmoduleshapes.code.tex` lines 891-892
    /// make it when nothing says otherwise.
    pub fn outer(&self) -> f64 {
        self.outer_sep.unwrap_or(self.width / 2.0)
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

/// `label=above:text`, `label=45:text` or `label=text` (§17.10.1).
///
/// The direction comes back in degrees, because that is what it is: PGF puts
/// the label at the border in that direction and anchors it by the border
/// point 180 degrees round from it, so `label=30:x` is a real placement and
/// not one of eight. A placement this cannot read gives no label at all --
/// putting the text in the middle of the node it was meant to sit outside is
/// a picture that is drawn wrong rather than drawn short.
fn label(value: &str) -> Option<(f64, String)> {
    // `label={[red]above:text}` -- an option list of the label's own, which
    // changes how it is painted and not where it goes.
    let value = value.trim();
    let value = match value.strip_prefix('[') {
        Some(rest) => rest.split_once(']').map(|(_, tail)| tail).unwrap_or(rest),
        None => value,
    };
    let (where_, text) = match value.split_once(':') {
        Some((placement, text)) => (placement.trim(), text.trim()),
        // `label=text` is `label=above:text` -- `\tikz@label@angle` starts at
        // 90 (`tikz.code.tex`'s `label position` default).
        None => ("above", value.trim()),
    };
    let degrees = match where_ {
        "above" | "north" => 90.0,
        "below" | "south" => 270.0,
        "left" | "west" => 180.0,
        "right" | "east" => 0.0,
        "above left" | "north west" => 135.0,
        "above right" | "north east" => 45.0,
        "below left" | "south west" => 225.0,
        "below right" | "south east" => 315.0,
        other => units::number(other)?,
    };
    Some((degrees, strip_braces(text).to_string()))
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
        assert_eq!(
            arrow_spec("latex-latex"),
            Some((Some(Tip::Latex), Some(Tip::Latex)))
        );
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
