//! TikZ: the picture language, as far as a document draws with it.
//!
//! TikZ is a language in its own right -- coordinates, transformations, path
//! operators, styles, nodes, arrow tips, loops -- sitting on top of PGF, which
//! is itself a portable path model over half a dozen drivers. What is here is
//! the part of it that reaches the page as PDF operators:
//!
//! - **Path operators** (§14): `--`, `-|` and `|-`, `..controls..`, `rectangle`,
//!   `circle`, `ellipse`, `arc`, `grid`, `parabola`, `to` and `cycle`.
//! - **Actions** (§15): `\draw`, `\fill`, `\filldraw`, `\path`, `\clip`,
//!   `\shade` and `\shadedraw`.
//! - **Shadings** (§15.5): `axis`, `radial` and `ball`, out of `left color=`,
//!   `right color=`, `top color=`, `bottom color=`, `middle color=`, `inner
//!   color=`, `outer color=` and `ball color=`, painted by PDF's `sh` through
//!   the path as a clip -- which is what `\pgfshadepath` does.
//! - **Decorations** (§24): `snake`, `zigzag`, `saw` and `brace`, each the
//!   state machine its `\pgfdeclaredecoration` is, on a straight segment.
//! - **Options**: colour by name or by `draw=`/`fill=`, `line width=` and the
//!   seven named widths, the dash patterns, caps and joins, opacity, the
//!   even-odd rule, and the canvas transformations `rotate`, `scale`, `shift`,
//!   `xshift` and `yshift`.
//! - **Arrows** (§16): `->`, `<-`, `<->`, `-stealth` and `-latex`, drawn as the
//!   paths PGF draws them as, on a line shortened to make room for them.
//! - **Nodes** (§17): `\node`, a `node` on a path, the nine anchors AND the
//!   border at any angle, the `rectangle`, `circle`, `ellipse` and `diamond`
//!   shapes, `label=`, and text set through the engine's own typesetter.
//! - **Coordinates** (§13): cartesian, polar, named, a named node's anchors,
//!   `+`/`++` relative, `\pgfmath` arithmetic (§89), and enough of `calc` for
//!   `($(a)+(1,0)$)` and `($(a)!.5!(b)$)`.
//! - **Loops and scopes** (§12.3.1, §88): `\foreach` including `1,...,5`
//!   ranges, and `scope` environments whose options are inherited.
//!
//! Everything is read against PGF's own source rather than guessed at: the
//! curve constants, the arrow geometry, the dash patterns and the named line
//! widths are cited where they are used, and the comments name the file and
//! line they came from. A curve approximated differently or a dash pattern
//! remembered wrong is not a missing feature -- it is a picture that is drawn
//! and drawn incorrectly, which is worse.
//!
//! What is NOT here: patterns, the `matrix` and `graph` libraries, `pic`s,
//! node shapes beyond rectangle, circle, ellipse and diamond, and the
//! decorations outside `snake`, `zigzag`, `saw` and `brace`. A path built out
//! of those comes out with whatever else it also had, and no more -- and a
//! `pattern=` turns the fill OFF rather than painting a solid block where the
//! document asked for hatching.
//!
//! **How a document reaches this.** `\begin{tikzpicture}` is read RAW by
//! `lower::picture_environment` -- a picture body is TikZ and not TeX, and
//! expanding it is exactly what must not happen -- and travels the text stream
//! as one `typeset::PICTURE` marker. `typeset::to_pdf` decodes it, parses it
//! here against the document's own font metrics, and calls [`draw_on`] with
//! the origin that puts the picture's bounding box where the line it is on
//! sits. [`Picture::bounds`] is what decides how much of the page that is.
//!
//! A shading is painted by `sh`, which names an entry in the page's
//! `/Shading` resource the way `gs` names one in its `/ExtGState`; `draw_on`
//! registers both through `pdf::Page`, so a `\shade` reaches the page with its
//! ramp. `to_pdf_ops` writes the operators alone, for a caller that carries
//! the resources itself -- `Picture::shadings` and `Picture::ext_gstates` are
//! the lists of what such a caller has to carry.

pub mod arrows;
pub mod coord;
pub mod decorate;
pub mod math;
pub mod options;
pub mod path;
pub mod render;
pub mod scan;
pub mod shading;
pub mod shapes;
pub mod units;

use crate::colour::{Colours, Rgb};

pub use options::{Anchor, Cap, Decoration, Join, Shade, Shading, Shape, Style, Tip, Transform};
pub use scan::Action;
pub use shading::Ramp;
pub use shapes::Border;

/// A point in the picture's own units.
pub type Point = (f64, f64);

/// One piece of a path, from wherever the path already is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// A straight line to a point -- PDF's `l`.
    Line(Point),
    /// A cubic Bezier through two control points -- PDF's `c`.
    Curve(Point, Point, Point),
}

/// One subpath, with everything the command that built it asked for.
///
/// A `\draw` that carries two disjoint runs of coordinates makes two of these;
/// they share a `group`, and `to_pdf_ops` paints them as one path object with
/// one operator, which is what PGF emits and what the even-odd rule needs to
/// see both of them at once.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// The points the path passes through, start first, in picture units.
    ///
    /// A curve contributes its endpoint and not its control points, so this is
    /// the path's extent and not its shape -- `segments` is its shape.
    pub points: Vec<Point>,
    /// Where the subpath begins.
    pub start: Point,
    /// What it draws from there.
    pub segments: Vec<Segment>,
    /// Whether it closes back to its first point (`-- cycle`).
    pub closed: bool,
    /// Stroke width in points.
    pub width: f64,
    /// Which path command this subpath came from.
    pub group: usize,
    pub action: Action,
    pub stroke: Rgb,
    pub fill: Rgb,
    /// Whether the command asked for a stroke and a fill at all -- `\path[draw]`
    /// strokes where a bare `\path` does not.
    pub draw: bool,
    pub filled: bool,
    pub even_odd: bool,
    /// The dash pattern in points, on and off alternating; empty is solid.
    pub dash: Vec<f64>,
    pub cap: Cap,
    pub join: Join,
    pub draw_opacity: f64,
    pub fill_opacity: f64,
    pub arrow_start: Option<Tip>,
    pub arrow_end: Option<Tip>,
    /// `rounded corners=` -- the radius each corner is cut back and arced by,
    /// in points, or zero for a corner drawn sharp.
    pub rounded: f64,
    /// What a `\shade` paints over this path, if it asked for one.
    pub shade: Option<Shade>,
}

impl Path {
    /// The path's points in page coordinates.
    pub fn anchor_points(&self, pic: &Picture, ox: f64, oy: f64) -> Vec<Point> {
        self.points
            .iter()
            .map(|(x, y)| (ox + x * pic.x_scale, oy + y * pic.y_scale))
            .collect()
    }
}

/// A node: a piece of text with a shape around it, placed by one of its
/// anchors (§17).
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The name the picture gave it, if any.
    pub name: Option<String>,
    /// Where the node's `anchor` sits, in picture units.
    pub at: Point,
    pub text: String,
    pub anchor: Anchor,
    pub shape: Shape,
    /// The text's width, height above the baseline and depth below it, in
    /// points, as the metrics passed to `parse_with` measured it.
    pub measured: (f64, f64, f64),
    /// The space between the text and the border, on each axis (§17.2.2).
    pub inner_sep: (f64, f64),
    /// How far the node's ANCHORS stand outside its drawn border.
    pub outer_sep: f64,
    /// `minimum width` and `minimum height`, in points.
    pub minimum: (f64, f64),
    /// `\pgfshapeaspect`, which only the diamond's proportions use.
    pub aspect: f64,
    pub font_size: f64,
    pub stroke: Rgb,
    pub fill: Rgb,
    pub text_colour: Rgb,
    pub draw: bool,
    pub filled: bool,
    pub width: f64,
}

impl Node {
    /// The text with its inner separation, half either side of the centre --
    /// which is what every shape's saved anchors are computed from.
    pub fn text_half(&self) -> (f64, f64) {
        let (width, height, depth) = self.measured;
        (
            width / 2.0 + self.inner_sep.0,
            (height + depth) / 2.0 + self.inner_sep.1,
        )
    }

    /// The node's border, as its anchors see it.
    pub fn border(&self) -> Border {
        Border::of(
            self.shape,
            self.text_half(),
            self.minimum,
            self.outer_sep,
            self.aspect,
        )
    }

    /// Half the node's border box, in points, outer separation included.
    pub fn half_size(&self) -> (f64, f64) {
        self.border().half
    }

    /// Where the node's centre is, given that its `anchor` sits on `at`.
    ///
    /// Both are in POINTS: a node is sized by its text and its anchors are
    /// where the text put them, so this is not in the picture's own units and
    /// a caller working in those has to divide by `x=`/`y=` first.
    pub fn centre(&self, at: Point) -> Point {
        let (dx, dy) = self.border().anchor(self.anchor);
        (at.0 - dx, at.1 - dy)
    }

    /// How far the text's baseline sits below the node's centre.
    ///
    /// PGF's `\centerpoint` for a text node is half its height less half its
    /// depth above the baseline (`pgfmoduleshapes.code.tex` lines 1192-1196),
    /// so the baseline is that far below the centre.
    pub fn baseline_drop(&self) -> f64 {
        let (_, height, depth) = self.measured;
        (height - depth) / 2.0
    }
}

/// A whole `tikzpicture`: what it draws, and the scale its options set.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub paths: Vec<Path>,
    pub nodes: Vec<Node>,
    /// `x=0.38pt` and `y=0.38pt` scale every coordinate.
    pub x_scale: f64,
    pub y_scale: f64,
}

impl Default for Picture {
    fn default() -> Picture {
        Picture {
            paths: Vec::new(),
            nodes: Vec::new(),
            x_scale: 1.0,
            y_scale: 1.0,
        }
    }
}

impl Picture {
    /// The picture's extent in points, after scaling.
    pub fn size(&self) -> (f64, f64) {
        let mut w: f64 = 0.0;
        let mut h: f64 = 0.0;
        for p in &self.paths {
            for (x, y) in &p.points {
                w = w.max(x * self.x_scale);
                h = h.max(y * self.y_scale);
            }
        }
        (w, h)
    }

    /// The picture's bounding box in points, as PGF protocols it.
    ///
    /// `(min_x, min_y, max_x, max_y)`, and it is what decides how much of a
    /// page a picture takes: a `tikzpicture` is a box the height of this box,
    /// set with the box's BOTTOM on the baseline -- which is what
    /// `\endpgfpicture` makes of a picture that gave no `baseline=` option.
    ///
    /// Three things go into it, and each is what PGF's own does:
    ///
    ///   * every point of every path, INCLUDING a curve's two control points
    ///     -- `\pgf@lt@curveto` protocols all three of its points
    ///     (`pgfcorepathconstruct.code.tex` lines 92-97), so a curve reserves
    ///     the hull it is drawn inside rather than the endpoints it passes
    ///     through;
    ///   * half the line width around a path that is STROKED, added once to
    ///     the path's extent rather than to each of its points
    ///     (`pgfcorepathusage.code.tex` lines 116-131) -- a `thick` line
    ///     drawn along y=0 reaches 0.39851pt below it, and lualatex places
    ///     the picture that much above the baseline to make room for it;
    ///   * a node's border box, which is where its anchors are.
    ///
    /// A picture that draws nothing has no box at all, and answers with the
    /// empty one at the origin.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut seen = false;
        let mut widen = |(x, y): Point, pad: f64| {
            seen = true;
            min = (min.0.min(x - pad), min.1.min(y - pad));
            max = (max.0.max(x + pad), max.1.max(y + pad));
        };
        for path in &self.paths {
            let place = |(x, y): Point| (x * self.x_scale, y * self.y_scale);
            // The stroke's own width, or nothing at all for a path that is
            // only filled: a fill reaches exactly as far as its outline.
            let half = match path.draw {
                true => path.width / 2.0,
                false => 0.0,
            };
            widen(place(path.start), half);
            for segment in &path.segments {
                match segment {
                    Segment::Line(to) => widen(place(*to), half),
                    Segment::Curve(a, b, to) => {
                        widen(place(*a), half);
                        widen(place(*b), half);
                        widen(place(*to), half);
                    }
                }
            }
        }
        for node in &self.nodes {
            let at = (node.at.0 * self.x_scale, node.at.1 * self.y_scale);
            let (cx, cy) = node.centre(at);
            let (hx, hy) = node.half_size();
            widen((cx - hx, cy - hy), 0.0);
            widen((cx + hx, cy + hy), 0.0);
        }
        match seen {
            true => (min.0, min.1, max.0, max.1),
            false => (0.0, 0.0, 0.0, 0.0),
        }
    }

    /// What the picture comes to on the page: the width and height of
    /// [`bounds`](Self::bounds).
    pub fn extent(&self) -> (f64, f64) {
        let (min_x, min_y, max_x, max_y) = self.bounds();
        (max_x - min_x, max_y - min_y)
    }

    /// The `/ExtGState` entries the emitted operators name.
    ///
    /// Constant alpha is not an operator: it is a graphics-state dictionary
    /// looked up by name (PDF 32000-1 S11.6.4.4), so a page carrying a picture
    /// with `opacity=` set has to carry the dictionaries too. Each entry is
    /// the name, the key (`CA` for stroking, `ca` for non-stroking) and the
    /// value.
    pub fn ext_gstates(&self) -> Vec<(String, &'static str, f64)> {
        let mut out: Vec<(String, &'static str, f64)> = Vec::new();
        for path in &self.paths {
            for (alpha, key, prefix) in [
                (path.draw_opacity, "CA", "pgf@CA"),
                (path.fill_opacity, "ca", "pgf@ca"),
            ] {
                if alpha == 1.0 {
                    continue;
                }
                let name = format!("{prefix}{}", render::number(alpha));
                if !out.iter().any(|(existing, _, _)| existing == &name) {
                    out.push((name, key, alpha));
                }
            }
        }
        out
    }

    /// The distinct shadings the picture's paths ask for, in the order they
    /// are met -- which is what fixes the name each one is called by.
    pub fn shades(&self) -> Vec<Shade> {
        let mut out: Vec<Shade> = Vec::new();
        for path in &self.paths {
            if let Some(shade) = path.shade {
                if !out.contains(&shade) {
                    out.push(shade);
                }
            }
        }
        out
    }

    /// The `/Shading` entries the emitted operators name.
    ///
    /// `sh` (PDF 32000-1 §8.7.4.5) paints a shading looked up BY NAME out of
    /// the page's `/Shading` resource, the way `gs` looks up a graphics state:
    /// there is no operator that carries a ramp. So a page carrying a shaded
    /// picture has to carry the dictionaries too, and this is the list of
    /// them. `Ramp::dictionary` is each one as PDF source.
    pub fn shadings(&self) -> Vec<Ramp> {
        self.shades()
            .iter()
            .enumerate()
            .map(|(at, shade)| Ramp::of(shade, &format!("pgfsh{at}")))
            .collect()
    }
}

/// What a node's text comes to on the page.
///
/// A node's border is drawn around its text, so its size is not in the source:
/// it has to be measured. The engine's own font chain measures it exactly;
/// `Estimate` stands in where no font has been loaded, and says so.
pub trait Metrics {
    /// The width of `text` set at `size` points.
    fn width_of(&self, text: &str, size: f64) -> f64;

    /// How far the text reaches above and below its baseline, in points.
    fn height_of(&self, _text: &str, size: f64) -> (f64, f64) {
        (0.7 * size, 0.2 * size)
    }
}

/// A stand-in for real metrics: half an em per character.
///
/// This is NOT a measurement -- it is the width a monospaced half-em font would
/// come to, and a node sized by it is the right shape and the wrong size. It is
/// what `parse` uses, because `parse` has no font; `parse_with` takes the
/// engine's own `FontChain` and gets the real advance widths out of the font's
/// `hmtx`.
pub struct Estimate;

impl Metrics for Estimate {
    fn width_of(&self, text: &str, size: f64) -> f64 {
        text.chars().count() as f64 * 0.5 * size
    }
}

impl Metrics for crate::typeset::FontChain {
    /// The advance widths of the font the document is set in, through the same
    /// character-to-slot resolution the paragraph builder uses.
    fn width_of(&self, text: &str, size: f64) -> f64 {
        crate::typeset::FontChain::width_of(self, text, size)
    }
}

/// Read a `tikzpicture` body, with the options from its `[...]`.
///
/// The palette is xcolor's defaults, and node text is sized by `Estimate`. A
/// document that defines its own colours or sets nodes wants `parse_with`.
pub fn parse(options: &str, body: &str) -> Picture {
    parse_with(options, body, &Colours::new(), &Estimate)
}

/// PGF's own unit vectors, in points.
///
/// `\pgfsetxvec{\pgfpoint{1cm}{0cm}}` and `\pgfsetyvec{\pgfpoint{0cm}{1cm}}`
/// (`pgfcorepoints.code.tex` lines 922-925): one of a picture's own units is
/// one centimetre unless the picture's `x=`/`y=` says otherwise.
pub const UNIT: f64 = 72.27 / 2.54;

/// The same, against a document's own palette and font metrics.
///
/// A bare number is ONE POINT here. That is not what TikZ means by it -- see
/// [`UNIT`] -- and it is what this answers in, because a picture is read in
/// its own units and multiplied by `x=`/`y=` where it is drawn: a caller that
/// scales the result itself would otherwise scale it twice. A caller drawing a
/// DOCUMENT's picture wants [`parse_document`], which is this at PGF's unit.
pub fn parse_with(options: &str, body: &str, colours: &Colours, metrics: &dyn Metrics) -> Picture {
    parse_units(options, body, colours, metrics, 1.0)
}

/// A document's picture, at PGF's own unit vectors.
///
/// This is the entry the typesetting path takes, and the difference is one
/// number: a `\draw (0,0) -- (3,0)` in a real document is three CENTIMETRES
/// long, and read at a point to the unit it comes out a twenty-eighth of the
/// size the document drew it.
pub fn parse_document(
    options: &str,
    body: &str,
    colours: &Colours,
    metrics: &dyn Metrics,
) -> Picture {
    parse_units(options, body, colours, metrics, UNIT)
}

/// The same again, saying what a bare number means where the picture does not.
fn parse_units(
    options: &str,
    body: &str,
    colours: &Colours,
    metrics: &dyn Metrics,
    unit: f64,
) -> Picture {
    let mut pic = Picture {
        x_scale: option_length(options, "x").unwrap_or(unit),
        y_scale: option_length(options, "y").unwrap_or(unit),
        ..Picture::default()
    };
    let base = Style::default().with(options, colours);
    let mut frame = coord::Frame::new(pic.x_scale, pic.y_scale);
    let mut group = 0usize;
    walk(
        &scan::commands(body),
        &base,
        colours,
        metrics,
        &mut frame,
        &mut pic,
        &mut group,
    );
    pic
}

/// One `x=0.38pt` out of a picture's option list.
fn option_length(options: &str, key: &str) -> Option<f64> {
    options::split(options)
        .into_iter()
        .filter_map(|option| option.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .and_then(|(_, value)| units::dimension(value.trim()))
}

/// Every command of a body, with the options it has inherited.
fn walk(
    chunks: &[scan::Chunk],
    base: &Style,
    colours: &Colours,
    metrics: &dyn Metrics,
    frame: &mut coord::Frame,
    pic: &mut Picture,
    group: &mut usize,
) {
    for chunk in chunks {
        match chunk {
            // A scope's options are the defaults for everything inside it,
            // and its own transformation composes with the one it is inside.
            scan::Chunk::Scope { options, body } => {
                let inner = base.with(options, colours);
                walk(
                    &scan::commands(body),
                    &inner,
                    colours,
                    metrics,
                    frame,
                    pic,
                    group,
                );
            }
            scan::Chunk::Path {
                action,
                options,
                body,
            } => {
                let style = base.with(options, colours);
                // Each path starts from the origin: TikZ resets the current
                // point at every command, so a `+(1,0)` at the head of a path
                // is measured from (0,0) and not from where the last one
                // happened to end.
                frame.current = (0.0, 0.0);
                frame.relative = (0.0, 0.0);
                frame.last_move = (0.0, 0.0);
                let built = path::build(body, frame);
                emit(&built, *action, &style, metrics, frame, pic, group);
            }
        }
    }
}

/// Turn one command's geometry into the picture's paths and nodes.
fn emit(
    built: &path::Built,
    action: Action,
    style: &Style,
    metrics: &dyn Metrics,
    frame: &mut coord::Frame,
    pic: &mut Picture,
    group: &mut usize,
) {
    let (draw, filled) = match action {
        Action::Draw => (true, style.filled),
        Action::Fill => (style.draw, true),
        Action::FillDraw => (true, true),
        // `\shadedraw` is `\shade` with the border on: `\tikz@mode@drawtrue`
        // as well as `\tikz@mode@shadetrue`.
        Action::ShadeDraw => (true, style.filled),
        // `\path[draw]` and `\path[fill]` are what `\draw` and `\fill` stand
        // for, so a bare `\path` paints exactly what its options asked for.
        Action::None | Action::Shade | Action::Clip | Action::Node => (style.draw, style.filled),
    };
    // A `pattern=` is a tiling pattern in the page's `/Pattern` resource, and
    // nothing here writes one. The area is left blank rather than filled with
    // a colour the document never named -- a solid block where the picture
    // asked for hatching is drawn, and drawn wrong.
    let filled = filled && !style.pattern;
    let action = match (action, draw, filled) {
        (Action::None, true, true) => Action::FillDraw,
        (Action::None, true, false) => Action::Draw,
        (Action::None, false, true) => Action::Fill,
        // `\draw[fill=orange]` fills as well as strokes: `\tikzoption{fill}`
        // ends in `\tikz@addmode{\tikz@mode@filltrue}` whatever else it does
        // (tikz.code.tex lines 507-519), so naming a fill colour turns filling
        // on. The pair above computed that correctly and the painting operator
        // was still chosen off the COMMAND, so the fill colour was written and
        // the path stroked with `S`: an outline where lualatex draws a solid.
        (Action::Draw, true, true) => Action::FillDraw,
        // A `\fill` with nothing to fill it with paints nothing at all.
        (Action::Fill | Action::FillDraw, true, false) => Action::Draw,
        (Action::Fill | Action::FillDraw, false, false) => Action::None,
        (other, _, _) => other,
    };
    let point = |p: Point| style.transform.apply(p);
    for sub in &built.subpaths {
        // A decoration REPLACES the path it is put on: `decorate` hands the
        // segments to the decoration's own state machine and what comes back
        // is what is drawn (§24). The path's own points are kept as its
        // extent, because that is what the picture is measured by.
        let sub = match (style.decorate, style.decoration) {
            (true, Some(decoration)) => decorate::apply(
                sub,
                decoration,
                style.segment_length,
                style.amplitude,
                style.decoration_aspect,
            ),
            _ => sub.clone(),
        };
        let sub = &sub;
        pic.paths.push(Path {
            points: sub.anchors().into_iter().map(point).collect(),
            start: point(sub.start),
            segments: sub
                .segments
                .iter()
                .map(|segment| match segment {
                    Segment::Line(to) => Segment::Line(point(*to)),
                    Segment::Curve(a, b, to) => Segment::Curve(point(*a), point(*b), point(*to)),
                })
                .collect(),
            closed: sub.closed,
            width: style.width,
            group: *group,
            action,
            stroke: style.stroke,
            fill: style.fill,
            draw,
            filled,
            even_odd: style.even_odd,
            dash: style.dash.clone(),
            cap: style.cap,
            join: style.join,
            draw_opacity: style.draw_opacity,
            fill_opacity: style.fill_opacity,
            arrow_start: style.arrow_start,
            arrow_end: style.arrow_end,
            rounded: style.rounded,
            // A `\shade` with no shading key still shades: `\tikz@shading`
            // starts as `axis` and the colours have defaults
            // (`tikz.code.tex` lines 625, 635-637).
            shade: match (action, style.shade) {
                (_, Some(shade)) => Some(shade),
                (Action::Shade | Action::ShadeDraw, None) => Some(Shade::default()),
                _ => None,
            },
        });
    }
    for pending in &built.nodes {
        // A node's own `[...]` overrides what the path handed it, which is how
        // `\draw[red] ... node[above,black] {x}` sets black text on a red line.
        let node_style = style.with(&pending.options, &Colours::new());
        let node_style = match pending.options.is_empty() {
            true => style.clone(),
            false => node_style,
        };
        let node = measure(
            &pending.text,
            pending.name.clone(),
            point(pending.at),
            &node_style,
            metrics,
        );
        // A node's size is in points and its place is in the picture's own
        // units, so every offset between the two is divided by the scale on
        // its way across -- the same trip a `1cm` in a coordinate makes.
        let (x_scale, y_scale) = (frame.x_scale, frame.y_scale);
        let unscale = |value: f64, scale: f64| match scale != 0.0 {
            true => value / scale,
            false => value,
        };
        let across = |(dx, dy): Point| (unscale(dx, x_scale), unscale(dy, y_scale));
        let (adx, ady) = across(node.border().anchor(node.anchor));
        let centre = (node.at.0 - adx, node.at.1 - ady);
        // The node's own name now stands for a SHAPE and not just a point, so
        // `(a.north)` in the next command has a border to land on.
        if let Some(name) = &node.name {
            frame.named.insert(
                name.clone(),
                coord::Placed {
                    at: centre,
                    border: node.border(),
                },
            );
        }
        // `label=` is a node of its own, put on the labelled node's border in
        // the direction the label names and anchored by the point 180 degrees
        // round from that -- which is what keeps it outside (§17.10.1).
        if let Some((degrees, text)) = &node_style.label {
            let mut label = measure(text, None, (0.0, 0.0), &node_style, metrics);
            // A label is a plain unpainted node: `every label` sets neither
            // `draw` nor `fill`, and it is not the labelled node's shape.
            label.draw = false;
            label.filled = false;
            label.shape = Shape::Rectangle;
            label.anchor = Anchor::Center;
            let (dx, dy) = node.border().border_at(*degrees);
            let reach = dx.hypot(dy) + node_style.label_distance;
            let (sin, cos) = degrees.to_radians().sin_cos();
            let (ox, oy) = label.border().border_at(degrees + 180.0);
            let (lx, ly) = across((reach * cos - ox, reach * sin - oy));
            label.at = (centre.0 + lx, centre.1 + ly);
            pic.nodes.push(label);
        }
        pic.nodes.push(node);
    }
    *group += 1;
}

/// One node, with its text measured and its geometry settled.
fn measure(
    text: &str,
    name: Option<String>,
    at: Point,
    style: &Style,
    metrics: &dyn Metrics,
) -> Node {
    let (height, depth) = metrics.height_of(text, style.font_size);
    Node {
        name,
        at,
        text: text.to_string(),
        anchor: style.anchor,
        shape: style.shape,
        measured: (metrics.width_of(text, style.font_size), height, depth),
        inner_sep: style.inner_sep,
        outer_sep: style.outer(),
        minimum: style.minimum,
        aspect: style.aspect,
        font_size: style.font_size,
        stroke: style.stroke,
        fill: style.fill,
        text_colour: style.text,
        draw: style.draw,
        filled: style.filled,
        width: style.width,
    }
}

/// The PDF operators that draw a picture, offset to `(ox, oy)`.
///
/// This is the paths and the node borders. Node TEXT needs a page to put
/// glyphs on and a font to set them in, which is what `draw_on` is for.
pub fn to_pdf_ops(pic: &Picture, ox: f64, oy: f64) -> String {
    render::to_pdf_ops(pic, ox, oy)
}

/// Draw a picture onto a page, node text and all.
///
/// The paths go into the page's content stream as operators; the text goes
/// through `Page::text_in`, which is the same call every other glyph on the
/// page is drawn by, so a node's text is set in the document's font at the
/// document's size and lands in the PDF as the same `BT ... ET` block.
pub fn draw_on(
    pic: &Picture,
    page: &mut crate::pdf::Page,
    ox: f64,
    oy: f64,
    font: crate::pdf::Font,
) {
    // A shading is painted by `sh`, which names a dictionary out of the
    // page's `/Shading` (§8.7.4.5) exactly as `gs` names one out of its
    // `/ExtGState`. Both are registered here, because a name that resolves to
    // nothing is worse than no operator: a reader answers a missing
    // `/ExtGState` by drawing the path opaque and a missing `/Shading` by
    // painting nothing at all.
    page.content
        .push_str(&render::to_pdf_ops_where(pic, ox, oy, true));
    for ramp in pic.shadings() {
        page.shading(&ramp.name, &ramp.dictionary());
    }
    // The operators just written may name a graphics state -- `/pgf@CA0.5 gs`
    // for `opacity=0.5` -- and a name resolves through the PAGE's `/ExtGState`
    // (§8.4.4). Emitting the operator without registering the dictionary is a
    // name pointing at nothing, which a reader answers by drawing the path
    // opaque: the picture is right and the transparency is gone.
    for (name, key, alpha) in pic.ext_gstates() {
        page.ext_gstate(&name, key, alpha);
    }
    for node in &pic.nodes {
        if node.text.trim().is_empty() {
            continue;
        }
        let (ax, ay) = (ox + node.at.0 * pic.x_scale, oy + node.at.1 * pic.y_scale);
        let (dx, dy) = node.border().anchor(node.anchor);
        let (cx, cy) = (ax - dx, ay - dy);
        page.text_in(
            font.clone(),
            node.font_size,
            cx - node.measured.0 / 2.0,
            cy - node.baseline_drop(),
            &node.text,
        );
    }
}
