//! TikZ: the picture language, as far as a document draws with it.
//!
//! TikZ is a language in its own right -- coordinates, transformations, path
//! operators, styles, nodes, arrow tips, loops -- sitting on top of PGF, which
//! is itself a portable path model over half a dozen drivers. What is here is
//! the part of it that reaches the page as PDF operators:
//!
//! - **Path operators** (§14): `--`, `-|` and `|-`, `..controls..`, `rectangle`,
//!   `circle`, `ellipse`, `arc`, `grid`, `parabola`, `to` and `cycle`.
//! - **Actions** (§15): `\draw`, `\fill`, `\filldraw`, `\path`, `\clip`, and
//!   `\shade` as a path that paints nothing.
//! - **Options**: colour by name or by `draw=`/`fill=`, `line width=` and the
//!   seven named widths, the dash patterns, caps and joins, opacity, the
//!   even-odd rule, and the canvas transformations `rotate`, `scale`, `shift`,
//!   `xshift` and `yshift`.
//! - **Arrows** (§16): `->`, `<-`, `<->`, `-stealth` and `-latex`, drawn as the
//!   paths PGF draws them as, on a line shortened to make room for them.
//! - **Nodes** (§17): `\node`, a `node` on a path, the nine anchors, the
//!   `rectangle` and `circle` shapes, and text set through the engine's own
//!   typesetter.
//! - **Coordinates** (§13): cartesian, polar, named, `+`/`++` relative, and
//!   enough of `calc` for `($(a)+(1,0)$)` and `($(a)!.5!(b)$)`.
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
//! What is NOT here: shadings and patterns, decorations, the `matrix` and
//! `graph` libraries, `pic`s, node shapes beyond rectangle and circle, edges
//! with `bend`, and PGF's mathematical engine. A path built out of those comes
//! out with whatever else it also had, and no more.

pub mod arrows;
pub mod coord;
pub mod options;
pub mod path;
pub mod render;
pub mod scan;
pub mod units;

use crate::colour::{Colours, Rgb};

pub use options::{Anchor, Cap, Join, Shape, Style, Tip, Transform};
pub use scan::Action;

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
    /// The space between the text and the border (§17.2.2).
    pub inner_sep: f64,
    /// `minimum width` and `minimum height`, in points.
    pub minimum: (f64, f64),
    pub font_size: f64,
    pub stroke: Rgb,
    pub fill: Rgb,
    pub text_colour: Rgb,
    pub draw: bool,
    pub filled: bool,
    pub width: f64,
}

impl Node {
    /// Half the node's border box, in points.
    pub fn half_size(&self) -> (f64, f64) {
        let (width, height, depth) = self.measured;
        (
            (width / 2.0 + self.inner_sep).max(self.minimum.0 / 2.0),
            ((height + depth) / 2.0 + self.inner_sep).max(self.minimum.1 / 2.0),
        )
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

/// The same, against a document's own palette and font metrics.
pub fn parse_with(
    options: &str,
    body: &str,
    colours: &Colours,
    metrics: &dyn Metrics,
) -> Picture {
    let mut pic = Picture {
        x_scale: option_length(options, "x").unwrap_or(1.0),
        y_scale: option_length(options, "y").unwrap_or(1.0),
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
                emit(&built, *action, &style, metrics, pic, group);
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
    pic: &mut Picture,
    group: &mut usize,
) {
    let (draw, filled) = match action {
        Action::Draw => (true, style.filled),
        Action::Fill => (style.draw, true),
        Action::FillDraw => (true, true),
        // `\path[draw]` and `\path[fill]` are what `\draw` and `\fill` stand
        // for, so a bare `\path` paints exactly what its options asked for.
        Action::None | Action::Shade | Action::Clip | Action::Node => {
            (style.draw, style.filled)
        }
    };
    let action = match (action, draw, filled) {
        (Action::None, true, true) => Action::FillDraw,
        (Action::None, true, false) => Action::Draw,
        (Action::None, false, true) => Action::Fill,
        (other, _, _) => other,
    };
    let point = |p: Point| style.transform.apply(p);
    for sub in &built.subpaths {
        pic.paths.push(Path {
            points: sub.anchors().into_iter().map(point).collect(),
            start: point(sub.start),
            segments: sub
                .segments
                .iter()
                .map(|segment| match segment {
                    Segment::Line(to) => Segment::Line(point(*to)),
                    Segment::Curve(a, b, to) => {
                        Segment::Curve(point(*a), point(*b), point(*to))
                    }
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
        let (height, depth) = metrics.height_of(&pending.text, node_style.font_size);
        pic.nodes.push(Node {
            name: pending.name.clone(),
            at: point(pending.at),
            text: pending.text.clone(),
            anchor: node_style.anchor,
            shape: node_style.shape,
            measured: (
                metrics.width_of(&pending.text, node_style.font_size),
                height,
                depth,
            ),
            inner_sep: node_style.inner_sep,
            minimum: node_style.minimum,
            font_size: node_style.font_size,
            stroke: node_style.stroke,
            fill: node_style.fill,
            text_colour: node_style.text,
            draw: node_style.draw,
            filled: node_style.filled,
            width: node_style.width,
        });
    }
    *group += 1;
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
    page.content.push_str(&to_pdf_ops(pic, ox, oy));
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
        let (ax, ay) = (
            ox + node.at.0 * pic.x_scale,
            oy + node.at.1 * pic.y_scale,
        );
        let (half_width, half_height) = node.half_size();
        let (dx, dy) = node.anchor.offset();
        let (cx, cy) = (ax - dx * half_width, ay - dy * half_height);
        page.text_in(
            font.clone(),
            node.font_size,
            cx - node.measured.0 / 2.0,
            cy - node.baseline_drop(),
            &node.text,
        );
    }
}
