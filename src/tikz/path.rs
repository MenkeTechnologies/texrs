//! One `\path`'s worth of geometry: the operators between the options and the
//! semicolon, turned into the curves and lines they name.
//!
//! The operators are the PGF manual's §14. What is here is `--` (§14.2.1),
//! `-|` and `|-` (§14.2.2), `..controls..` (§14.3), `rectangle` (§14.4),
//! `circle` and `ellipse` (§14.6), `arc` (§14.7), `grid` (§14.8), `parabola`
//! (§14.9) and `cycle`, plus the `node` and `coordinate` a path may carry
//! along it (§17).
//!
//! Every curve is PGF's own construction rather than a fresh approximation of
//! the same shape. A quarter ellipse is one cubic with its control points at
//! 0.55228475 of the radius (`pgfcorepathconstruct.code.tex` line 357); an arc
//! of more than 90 degrees is cut into 90-degree pieces, or 60-degree ones when
//! the whole sweep is under 115 (lines 307-314); a parabola's two halves use
//! the .1125/.5 and .5/.8875 control fractions the source marks "found by trial
//! and error" (lines 1291-1305). Approximating any of these differently draws a
//! curve that misses the one lualatex draws by a visible amount.

use super::coord::{self, Frame, Relative};
use super::units;
use super::{Point, Segment};

/// PGF's constant for a quarter of an ellipse, `pgfcorepathconstruct` line 357.
const KAPPA: f64 = 0.55228475;

/// One subpath: where it starts, what it draws, and whether it closes.
#[derive(Debug, Clone, PartialEq)]
pub struct Sub {
    pub start: Point,
    pub segments: Vec<Segment>,
    pub closed: bool,
}

impl Sub {
    /// The points the subpath actually passes through, start first.
    pub fn anchors(&self) -> Vec<Point> {
        let mut out = vec![self.start];
        for segment in &self.segments {
            out.push(match segment {
                Segment::Line(to) => *to,
                Segment::Curve(_, _, to) => *to,
            });
        }
        out
    }
}

/// A node met while reading a path, before its text has been measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub name: Option<String>,
    pub at: Point,
    pub text: String,
    pub options: String,
}

/// What one path command came to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Built {
    pub subpaths: Vec<Sub>,
    pub nodes: Vec<Pending>,
}

/// Read the geometry of one path command, advancing `frame` as it goes.
pub fn build(text: &str, frame: &mut Frame) -> Built {
    let mut state = State {
        built: Built::default(),
        open: None,
        frame,
    };
    state.run(text);
    state.close();
    state.built
}

/// The path being read: what is finished, and the subpath still open.
struct State<'a> {
    built: Built,
    open: Option<Sub>,
    frame: &'a mut Frame,
}

impl State<'_> {
    /// Finish whatever subpath is open.
    fn close(&mut self) {
        if let Some(sub) = self.open.take() {
            // A move-to with nothing after it draws nothing, and emitting it
            // would leave a stray `m` in the content stream.
            if !sub.segments.is_empty() {
                self.built.subpaths.push(sub);
            }
        }
    }

    /// Start a new subpath at `at`.
    fn move_to(&mut self, at: Point) {
        self.close();
        self.open = Some(Sub {
            start: at,
            segments: Vec::new(),
            closed: false,
        });
        self.frame.current = at;
        self.frame.last_move = at;
    }

    /// Add a segment to the open subpath, opening one at the current point if
    /// the path has not been started -- which is what an `arc` written first
    /// needs, since it draws from wherever the path already is.
    fn add(&mut self, segment: Segment) {
        if self.open.is_none() {
            self.open = Some(Sub {
                start: self.frame.current,
                segments: Vec::new(),
                closed: false,
            });
            self.frame.last_move = self.frame.current;
        }
        self.frame.current = match segment {
            Segment::Line(to) => to,
            Segment::Curve(_, _, to) => to,
        };
        if let Some(sub) = self.open.as_mut() {
            sub.segments.push(segment);
        }
    }

    /// A whole subpath at once, for the shapes that are one of their own.
    fn whole(&mut self, sub: Sub) {
        self.close();
        self.built.subpaths.push(sub);
    }

    fn run(&mut self, text: &str) {
        let mut rest = text;
        // `--` and its relatives only mean "line to" when a coordinate
        // follows; the connector is remembered until one does.
        let mut connector: Option<Connector> = None;
        while !rest.is_empty() {
            let trimmed = rest.trim_start();
            if trimmed.is_empty() {
                break;
            }
            rest = trimmed;
            if let Some(after) = rest.strip_prefix("--") {
                connector = Some(Connector::Straight);
                rest = after;
            } else if let Some(after) = rest.strip_prefix("-|") {
                connector = Some(Connector::HorizontalFirst);
                rest = after;
            } else if let Some(after) = rest.strip_prefix("|-") {
                connector = Some(Connector::VerticalFirst);
                rest = after;
            } else if rest.starts_with("..") {
                rest = self.curve(rest);
                connector = None;
            } else if let Some(after) = word(rest, "cycle") {
                self.cycle();
                rest = after;
                connector = None;
            } else if let Some(after) = word(rest, "rectangle") {
                let (at, after) = self.next_point(after);
                if let Some(at) = at {
                    self.rectangle(at);
                }
                rest = after;
                connector = None;
            } else if let Some(after) = word(rest, "circle") {
                rest = self.round(after, true);
                connector = None;
            } else if let Some(after) = word(rest, "ellipse") {
                rest = self.round(after, false);
                connector = None;
            } else if let Some(after) = word(rest, "arc") {
                rest = self.arc(after);
                connector = None;
            } else if let Some(after) = word(rest, "grid") {
                let (options, after) = brackets(after);
                let (at, after) = self.next_point(after);
                if let Some(at) = at {
                    self.grid(at, &options);
                }
                rest = after;
                connector = None;
            } else if let Some(after) = word(rest, "parabola") {
                rest = self.parabola(after);
                connector = None;
            } else if let Some(after) = word(rest, "node") {
                rest = self.node(after);
            } else if let Some(after) = word(rest, "coordinate") {
                rest = self.coordinate(after);
            } else if let Some(after) = word(rest, "to") {
                let (options, after) = brackets(after);
                let (target, after) = self.next_point(after);
                if let Some(target) = target {
                    self.path_to(&options, target);
                }
                rest = after;
                connector = None;
            } else if rest.starts_with('(') || rest.starts_with('+') {
                let (at, after) = self.next_point(rest);
                match (at, connector.take()) {
                    (Some(at), Some(how)) => self.connect(how, at),
                    (Some(at), None) => self.move_to(at),
                    (None, _) => {}
                }
                rest = after;
            } else {
                // A character nothing here claims. Skipping it is what keeps a
                // stray `;` or an unknown operator from stopping the path.
                let mut chars = rest.chars();
                chars.next();
                rest = chars.as_str();
            }
        }
    }

    /// A coordinate at the front of `text`, with its `+`/`++` prefix.
    fn next_point<'t>(&mut self, text: &'t str) -> (Option<Point>, &'t str) {
        let text = text.trim_start();
        let (how, text) = match text.strip_prefix("++") {
            Some(rest) => (Relative::Step, rest),
            None => match text.strip_prefix('+') {
                Some(rest) => (Relative::Offset, rest),
                None => (Relative::Absolute, text),
            },
        };
        let text = text.trim_start();
        let Some(open) = text.strip_prefix('(') else {
            return (None, text);
        };
        let Some(close) = coord::matching(open) else {
            return (None, "");
        };
        let after = &open[close + 1..];
        let Some(parsed) = coord::parse(&open[..close]) else {
            return (None, after);
        };
        let (x, y) = parsed.resolve(self.frame);
        let at = match how {
            Relative::Absolute => (x, y),
            // §13.4: both forms measure from the last point; only `++` moves
            // it, which is what makes `++(1,0) -- +(0,1)` draw an L and not a
            // diagonal.
            Relative::Offset | Relative::Step => {
                (self.frame.relative.0 + x, self.frame.relative.1 + y)
            }
        };
        if how != Relative::Offset {
            self.frame.relative = at;
        }
        (Some(at), after)
    }

    /// `--`, `-|` or `|-` to a point (§14.2).
    fn connect(&mut self, how: Connector, to: Point) {
        let from = self.frame.current;
        match how {
            Connector::Straight => self.add(Segment::Line(to)),
            // `-|` goes horizontally first and then vertically; `|-` the other
            // way round. lualatex writes both as two `l` operators.
            Connector::HorizontalFirst => {
                self.add(Segment::Line((to.0, from.1)));
                self.add(Segment::Line(to));
            }
            Connector::VerticalFirst => {
                self.add(Segment::Line((from.0, to.1)));
                self.add(Segment::Line(to));
            }
        }
    }

    /// `to[out=,in=]` and `to[bend left=]`: the `topaths` library's curve.
    ///
    /// A bare `to` is a straight line (§14.13, and `to path/.initial` is
    /// `-- (\tikztotarget)`). `out`, `in`, `bend left` and `bend right` switch
    /// it to a cubic whose controls stand off each end at
    ///
    /// ```text
    /// distance = 0.3915 * |target - start| * looseness
    /// ```
    ///
    /// in the direction the angle names -- `tikzlibrarytopaths.code.tex` lines
    /// 203-230. lualatex draws `(0,0) to[out=90,in=180] (2,2)` as
    /// `0.0 31.39217 25.30144 56.69362 56.69362 56.69362 c`, and 31.392 is
    /// 0.3915 of the 80.176 that the two points are apart.
    fn path_to(&mut self, options: &str, target: Point) {
        // `\def\tikz@to@out{45}` and `\def\tikz@to@in{135}` -- lines 121-122.
        let (mut out, mut into) = (45.0, 135.0);
        let (mut out_loose, mut in_loose) = (1.0, 1.0);
        let mut curved = false;
        let mut relative = false;
        // `\def\tikz@to@bend{30}` -- line 119.
        let mut bend = 30.0;
        for option in super::options::split(options) {
            let (key, value) = match option.split_once('=') {
                Some((key, value)) => (key.trim(), value.trim()),
                None => (option.trim(), ""),
            };
            let angle = |fallback: f64| units::number(value).unwrap_or(fallback);
            match key {
                "out" => {
                    out = angle(out);
                    curved = true;
                }
                "in" => {
                    into = angle(into);
                    curved = true;
                }
                "bend angle" => bend = angle(bend),
                // `bend left` measures its angle from the line itself, which
                // is what `\tikz@to@relativetrue` on lines 39 and 52 says.
                "bend left" => {
                    out = angle(bend);
                    into = 180.0 - out;
                    curved = true;
                    relative = true;
                }
                "bend right" => {
                    out = -angle(bend);
                    into = 180.0 - out;
                    curved = true;
                    relative = true;
                }
                "looseness" => {
                    out_loose = angle(1.0);
                    in_loose = out_loose;
                }
                "out looseness" => out_loose = angle(1.0),
                "in looseness" => in_loose = angle(1.0),
                "relative" => relative = true,
                _ => {}
            }
        }
        let from = self.frame.current;
        let (dx, dy) = (target.0 - from.0, target.1 - from.1);
        if !curved {
            self.add(Segment::Line(target));
            return;
        }
        let along = match relative {
            true => dy.atan2(dx).to_degrees(),
            false => 0.0,
        };
        let distance = 0.3915 * dx.hypot(dy);
        let off = |(x, y): Point, degrees: f64, reach: f64| {
            let radians = (degrees + along).to_radians();
            (x + reach * radians.cos(), y + reach * radians.sin())
        };
        self.add(Segment::Curve(
            off(from, out, distance * out_loose),
            off(target, into, distance * in_loose),
            target,
        ));
    }

    /// `cycle`: a straight line back to the last move-to, and the subpath is
    /// closed rather than merely joined up.
    fn cycle(&mut self) {
        let home = self.frame.last_move;
        if let Some(sub) = self.open.as_mut() {
            sub.closed = true;
        }
        self.frame.current = home;
        self.frame.relative = home;
    }

    /// `.. controls (c) and (d) .. (y)`, and the one-control form (§14.3).
    fn curve<'t>(&mut self, text: &'t str) -> &'t str {
        let rest = text.trim_start_matches('.').trim_start();
        let rest = rest.strip_prefix("controls").unwrap_or(rest);
        let (first, rest) = self.next_point(rest);
        let rest = rest.trim_start();
        let (second, rest) = match rest.strip_prefix("and") {
            Some(after) => self.next_point(after),
            None => (None, rest),
        };
        let rest = rest.trim_start().trim_start_matches('.');
        let (to, rest) = self.next_point(rest);
        let (Some(first), Some(to)) = (first, to) else {
            return rest;
        };
        // With no `and`, TikZ uses the one control point for both:
        // `\let\tikz@curve@second\tikz@curve@first` -- tikz.code.tex line 3188.
        // That is NOT the quadratic curve degree-elevated to a cubic, which is
        // what `\pgfpathquadraticcurveto` would have drawn; it is a different
        // and flatter curve, and it is the one the document gets.
        let second = second.unwrap_or(first);
        self.add(Segment::Curve(first, second, to));
        rest
    }

    /// `rectangle (corner)` (§14.4): a closed subpath, and the corner becomes
    /// the current point.
    ///
    /// The corners come out in the order lualatex writes them -- up the left
    /// side, across the top, down the right -- so the operands match.
    fn rectangle(&mut self, corner: Point) {
        let (x0, y0) = self.frame.current;
        let (x1, y1) = corner;
        self.whole(Sub {
            start: (x0, y0),
            segments: vec![
                Segment::Line((x0, y1)),
                Segment::Line((x1, y1)),
                Segment::Line((x1, y0)),
            ],
            closed: true,
        });
        self.frame.current = corner;
        self.frame.relative = corner;
    }

    /// `circle[radius=r]` and `ellipse[x radius=a, y radius=b]` (§14.6), and
    /// the older `circle (r)` and `ellipse (a and b)` spellings.
    fn round<'t>(&mut self, text: &'t str, circle: bool) -> &'t str {
        let (options, rest) = brackets(text);
        let (mut rx, mut ry) = (0.0, 0.0);
        let mut centre = self.frame.current;
        if options.is_empty() {
            // `circle (1cm)` and `ellipse (2cm and 1cm)` -- the radius in
            // parentheses, which is how TikZ wrote it before version 3.
            let text = rest.trim_start();
            let Some(open) = text.strip_prefix('(') else {
                return rest;
            };
            let Some(close) = coord::matching(open) else {
                return rest;
            };
            let (rx, ry) = radii(&open[..close], self.frame);
            self.ellipse(centre, rx, ry);
            return &open[close + 1..];
        }
        for option in super::options::split(&options) {
            let Some((key, value)) = option.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let on = |axis: f64| -> f64 { on_axis(value, axis) };
            match key {
                "radius" => {
                    rx = on(self.frame.x_scale);
                    ry = on(self.frame.y_scale);
                }
                "x radius" => rx = on(self.frame.x_scale),
                "y radius" => ry = on(self.frame.y_scale),
                "at" => {
                    let inside = value.trim().trim_start_matches('(').trim_end_matches(')');
                    if let Some(parsed) = coord::parse(inside) {
                        centre = parsed.resolve(self.frame);
                    }
                }
                _ => {}
            }
        }
        if circle && ry == 0.0 {
            ry = rx;
        }
        if circle && rx == 0.0 {
            rx = ry;
        }
        self.ellipse(centre, rx, ry);
        rest
    }

    /// The four quarter-ellipse curves `\pgfpathellipse` writes, in its order.
    ///
    /// It starts at the east point, goes anticlockwise, closes, and leaves the
    /// current point at the CENTRE -- which is why `\draw (0,0) circle[...]`
    /// followed by another operator carries on from the middle.
    fn ellipse(&mut self, centre: Point, rx: f64, ry: f64) {
        if rx == 0.0 && ry == 0.0 {
            return;
        }
        let (cx, cy) = centre;
        let (kx, ky) = (KAPPA * rx, KAPPA * ry);
        self.whole(Sub {
            start: (cx + rx, cy),
            segments: vec![
                Segment::Curve(
                    (cx + rx, cy + ky),
                    (cx + kx, cy + ry),
                    (cx, cy + ry),
                ),
                Segment::Curve(
                    (cx - kx, cy + ry),
                    (cx - rx, cy + ky),
                    (cx - rx, cy),
                ),
                Segment::Curve(
                    (cx - rx, cy - ky),
                    (cx - kx, cy - ry),
                    (cx, cy - ry),
                ),
                Segment::Curve(
                    (cx + kx, cy - ry),
                    (cx + rx, cy - ky),
                    (cx + rx, cy),
                ),
            ],
            closed: true,
        });
        self.frame.current = centre;
        self.frame.relative = centre;
    }

    /// `arc[start angle=, end angle=, radius=]` (§14.7).
    ///
    /// The current point is the START of the arc, on the ellipse -- the centre
    /// is wherever that puts it. A document that draws `(2,0) arc[start
    /// angle=0, end angle=90, radius=2cm]` gets a quarter circle whose centre
    /// is at the origin, and reading the current point as the centre instead
    /// would put the whole arc two centimetres out.
    fn arc<'t>(&mut self, text: &'t str) -> &'t str {
        let (options, rest) = brackets(text);
        let (mut start, mut end, mut delta) = (0.0, None, None);
        let (mut rx, mut ry) = (0.0, 0.0);
        if options.is_empty() {
            // `arc (0:90:1cm)` -- the older spelling.
            let text = rest.trim_start();
            let Some(open) = text.strip_prefix('(') else {
                return rest;
            };
            let Some(close) = coord::matching(open) else {
                return rest;
            };
            let mut fields = open[..close].splitn(3, ':');
            start = fields.next().and_then(units::number).unwrap_or(0.0);
            end = fields.next().and_then(units::number);
            let (a, b) = radii(fields.next().unwrap_or(""), self.frame);
            rx = a;
            ry = b;
            self.sweep(start, end.unwrap_or(start), rx, ry);
            return &open[close + 1..];
        }
        for option in super::options::split(&options) {
            let Some((key, value)) = option.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let length = |axis: f64| -> f64 { on_axis(value, axis) };
            match key {
                "start angle" => start = units::number(value).unwrap_or(0.0),
                "end angle" => end = units::number(value),
                "delta angle" => delta = units::number(value),
                "radius" => {
                    rx = length(self.frame.x_scale);
                    ry = length(self.frame.y_scale);
                }
                "x radius" => rx = length(self.frame.x_scale),
                "y radius" => ry = length(self.frame.y_scale),
                _ => {}
            }
        }
        let end = match (end, delta) {
            (Some(end), _) => end,
            (None, Some(delta)) => start + delta,
            (None, None) => start,
        };
        self.sweep(start, end, rx, ry);
        rest
    }

    /// `\pgfpatharc`'s own segmentation: 90-degree pieces above 115 degrees of
    /// sweep and 60-degree ones below it, so an arrow tip on a wide arc still
    /// meets a segment long enough to sit on (source lines 307-314).
    fn sweep(&mut self, start: f64, end: f64, rx: f64, ry: f64) {
        if rx == 0.0 && ry == 0.0 {
            return;
        }
        let step = match (end - start).abs() > 115.0 {
            true => 90.0,
            false => 60.0,
        };
        let mut from = start;
        while (from - end).abs() > 90.0 {
            let to = match end > from {
                true => from + step,
                false => from - step,
            };
            self.quarter(from, to, rx, ry);
            from = to;
        }
        self.quarter(from, end, rx, ry);
    }

    /// One arc segment as the cubic `\pgf@arc` writes (source lines 345-405).
    fn quarter(&mut self, from: f64, to: f64, rx: f64, ry: f64) {
        let delta = (to - from).abs();
        // The source writes the quarter-circle constant out rather than
        // computing it, so a right angle comes out to the same digits.
        let k = match (delta - 90.0).abs() < 1e-9 {
            true => KAPPA,
            false => 4.0 / 3.0 * (delta.to_radians() / 4.0).tan(),
        };
        let polar = |angle: f64, rx: f64, ry: f64| {
            let radians = angle.to_radians();
            (rx * radians.cos(), ry * radians.sin())
        };
        let (sx, sy) = self.frame.current;
        // The end is the current point moved by the difference between where
        // the two angles sit on the ellipse, which is what puts the arc's
        // centre wherever the START point implies.
        let (ax, ay) = polar(from, rx, ry);
        let (bx, by) = polar(to, rx, ry);
        let end = (sx - ax + bx, sy - ay + by);
        let lead = match to > from {
            true => 90.0,
            false => -90.0,
        };
        let (c1x, c1y) = polar(from + lead, k * rx, k * ry);
        let (c2x, c2y) = polar(to - lead, k * rx, k * ry);
        self.add(Segment::Curve(
            (sx + c1x, sy + c1y),
            (end.0 + c2x, end.1 + c2y),
            end,
        ));
    }

    /// `grid (corner)` (§14.8): the horizontal lines first and then the
    /// vertical ones, which is the order `\pgf@pathgrid` writes them in.
    fn grid(&mut self, corner: Point, options: &str) {
        let (mut x0, mut y0) = self.frame.current;
        let (mut x1, mut y1) = corner;
        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if y0 > y1 {
            std::mem::swap(&mut y0, &mut y1);
        }
        let (mut xstep, mut ystep) = (1.0, 1.0);
        for option in super::options::split(options) {
            if let Some((key, value)) = option.split_once('=') {
                let number = units::number(value.trim()).unwrap_or(1.0);
                match key.trim() {
                    "step" => {
                        xstep = number;
                        ystep = number;
                    }
                    "xstep" => xstep = number,
                    "ystep" => ystep = number,
                    _ => {}
                }
            }
        }
        if xstep <= 0.0 || ystep <= 0.0 {
            return;
        }
        let lines = |from: f64, to: f64, step: f64| {
            let first = (from / step).ceil() * step;
            let count = ((to - first) / step).floor() as i64;
            (0..=count.max(-1)).map(move |n| first + n as f64 * step)
        };
        for y in lines(y0, y1, ystep) {
            self.whole(Sub {
                start: (x0, y),
                segments: vec![Segment::Line((x1, y))],
                closed: false,
            });
        }
        for x in lines(x0, x1, xstep) {
            self.whole(Sub {
                start: (x, y0),
                segments: vec![Segment::Line((x, y1))],
                closed: false,
            });
        }
        self.frame.current = corner;
        self.frame.relative = corner;
    }

    /// `parabola bend (b) (c)` (§14.9), as two cubics.
    ///
    /// The control fractions are `\pgfpathparabola`'s, and the source calls
    /// them "found by trial and error" -- .1125 and .5 on the way up to the
    /// bend, .5 and .8875 on the way down (lines 1291-1305).
    fn parabola<'t>(&mut self, text: &'t str) -> &'t str {
        let (_, rest) = brackets(text);
        let rest = rest.trim_start();
        let (bend, rest) = match word(rest, "bend") {
            Some(after) => self.next_point(after),
            None => (None, rest),
        };
        let (to, rest) = self.next_point(rest);
        let Some(to) = to else { return rest };
        let from = self.frame.current;
        match bend {
            Some(bend) => {
                let up = (bend.0 - from.0, bend.1 - from.1);
                self.add(Segment::Curve(
                    (from.0 + 0.1125 * up.0, from.1 + 0.225 * up.1),
                    (from.0 + 0.5 * up.0, from.1 + up.1),
                    bend,
                ));
                let down = (to.0 - bend.0, to.1 - bend.1);
                self.add(Segment::Curve(
                    (bend.0 + 0.5 * down.0, bend.1),
                    (bend.0 + 0.8875 * down.0, bend.1 + 0.775 * down.1),
                    to,
                ));
            }
            // No bend named puts the vertex at the start, which is the
            // `bend at start` PGF falls back to.
            None => {
                let down = (to.0 - from.0, to.1 - from.1);
                self.add(Segment::Curve(
                    (from.0 + 0.5 * down.0, from.1),
                    (from.0 + 0.8875 * down.0, from.1 + 0.775 * down.1),
                    to,
                ));
            }
        }
        rest
    }

    /// `node[opts] (name) at (c) {text}` on a path (§17.2).
    fn node<'t>(&mut self, text: &'t str) -> &'t str {
        let (options, rest) = brackets(text);
        let rest = rest.trim_start();
        let (name, rest) = parenthesised(rest);
        let rest = rest.trim_start();
        let (at, rest) = match word(rest, "at") {
            Some(after) => self.next_point(after),
            None => (None, rest),
        };
        let rest = rest.trim_start();
        let (text, rest) = braced(rest);
        let at = at.unwrap_or(self.frame.current);
        if let Some(name) = name.clone() {
            self.frame.named.insert(name, at);
        }
        self.built.nodes.push(Pending {
            name,
            at,
            text,
            options,
        });
        rest
    }

    /// `coordinate (name) at (c)` -- a name for a point and nothing drawn.
    fn coordinate<'t>(&mut self, text: &'t str) -> &'t str {
        let (_, rest) = brackets(text);
        let rest = rest.trim_start();
        let (name, rest) = parenthesised(rest);
        let rest = rest.trim_start();
        let (at, rest) = match word(rest, "at") {
            Some(after) => self.next_point(after),
            None => (Some(self.frame.current), rest),
        };
        if let (Some(name), Some(at)) = (name, at) {
            self.frame.named.insert(name, at);
        }
        rest
    }
}

/// How two coordinates are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connector {
    Straight,
    HorizontalFirst,
    VerticalFirst,
}

/// A length in an option value, in picture units on the axis it lies along.
///
/// A bare number is already a picture unit; one written with a unit is a real
/// length on the paper, and is divided by the scale here so that multiplying it
/// back at render time lands it where the paper says.
fn on_axis(value: &str, axis: f64) -> f64 {
    let Some((length, _)) = units::scan(value) else {
        return 0.0;
    };
    match length.points {
        Some(points) if axis != 0.0 => points / axis,
        Some(points) => points,
        None => length.value,
    }
}

/// `2cm` or `2cm and 1cm`, in picture units on each axis.
fn radii(text: &str, frame: &Frame) -> (f64, f64) {
    match text.split_once("and") {
        Some((first, second)) => (on_axis(first, frame.x_scale), on_axis(second, frame.y_scale)),
        None => (on_axis(text, frame.x_scale), on_axis(text, frame.y_scale)),
    }
}

/// A keyword at the front of `text`, only when it stands as a word.
///
/// `circle` must not be found inside `circled`, and `to` must not be found
/// inside `topaz` -- a partial match here silently rewrites the path.
fn word<'t>(text: &'t str, keyword: &str) -> Option<&'t str> {
    let rest = text.strip_prefix(keyword)?;
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() => None,
        _ => Some(rest),
    }
}

/// `[...]` at the front, and what follows it.
fn brackets(text: &str) -> (String, &str) {
    let text = text.trim_start();
    let Some(open) = text.strip_prefix('[') else {
        return (String::new(), text);
    };
    let mut depth = 0i32;
    for (at, ch) in open.char_indices() {
        match ch {
            '[' | '{' => depth += 1,
            '}' => depth -= 1,
            ']' if depth == 0 => return (open[..at].to_string(), &open[at + 1..]),
            ']' => depth -= 1,
            _ => {}
        }
    }
    (String::new(), text)
}

/// `(name)` at the front, when what is inside is a plain name.
fn parenthesised(text: &str) -> (Option<String>, &str) {
    let Some(open) = text.strip_prefix('(') else {
        return (None, text);
    };
    let Some(close) = coord::matching(open) else {
        return (None, text);
    };
    (
        Some(open[..close].trim().to_string()),
        &open[close + 1..],
    )
}

/// `{...}` at the front, brace-counted, and what follows it.
fn braced(text: &str) -> (String, &str) {
    let Some(open) = text.strip_prefix('{') else {
        return (String::new(), text);
    };
    let mut depth = 0i32;
    for (at, ch) in open.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 0 => return (open[..at].to_string(), &open[at + 1..]),
            '}' => depth -= 1,
            _ => {}
        }
    }
    (String::new(), text)
}
