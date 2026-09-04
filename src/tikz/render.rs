//! The picture as PDF content-stream operators.
//!
//! PDF's path model is the one PGF was written against, so most of this is a
//! transcription rather than a translation: `m` and `l` and `c` build the path,
//! `h` closes it, and ONE painting operator at the end says what to do with the
//! whole thing -- `S` to stroke, `f` to fill, `B` to do both, `W n` to clip,
//! `n` to do nothing (PDF 32000-1 S8.5.3, Table 60). A `\fill` over two
//! disjoint subpaths is one path object with one `f`, which is why the subpaths
//! of a command are emitted together and painted once.
//!
//! PDF's y axis points up from the bottom of the page, which is the way TikZ's
//! does too, so the coordinates go through unflipped.

use std::fmt::Write;

use super::arrows;
use super::options::{Shade, Shape, Tip};
use super::scan::Action;
use super::shading;
use super::{Node, Path, Picture, Segment};

/// Everything one path command draws, ready to be written out.
struct Group<'a> {
    paths: Vec<&'a Path>,
}

/// The operators that draw `pic`, with its origin at `(ox, oy)`.
pub fn to_pdf_ops(pic: &Picture, ox: f64, oy: f64) -> String {
    to_pdf_ops_where(pic, ox, oy, true)
}

/// The same, saying whether the caller can carry the `/Shading` entries the
/// shading operators name.
///
/// `sh` looks its ramp up BY NAME in the page's resource dictionary, so a page
/// that cannot carry the entry must not be given the operator: the name would
/// resolve to nothing, which is not "no shading" to a reader but a stream it
/// has to guess at. A caller that can register them passes `true` and reads
/// `Picture::shadings` for what to register.
pub fn to_pdf_ops_where(pic: &Picture, ox: f64, oy: f64, shadings: bool) -> String {
    let mut out = String::new();
    // The whole picture is one saved graphics state, so a `\clip` inside it
    // stops applying where the picture stops rather than at the page's end.
    out.push_str("q\n");
    for group in groups(pic) {
        write_group(&mut out, pic, &group, ox, oy, shadings);
    }
    for node in &pic.nodes {
        write_node(&mut out, pic, node, ox, oy);
    }
    out.push_str("Q\n");
    out
}

/// The paths of each command, in the order the commands came.
fn groups(pic: &Picture) -> Vec<Group<'_>> {
    let mut out: Vec<Group> = Vec::new();
    let mut current = usize::MAX;
    for path in &pic.paths {
        if path.group != current || out.is_empty() {
            out.push(Group { paths: Vec::new() });
            current = path.group;
        }
        if let Some(group) = out.last_mut() {
            group.paths.push(path);
        }
    }
    out
}

/// One path command: its state, its subpaths, one painting operator, and the
/// arrow tips that go on its ends.
fn write_group(out: &mut String, pic: &Picture, group: &Group, ox: f64, oy: f64, shadings: bool) {
    let Some(first) = group.paths.first() else {
        return;
    };
    if let (Some(shade), true) = (first.shade, shadings) {
        write_shading(out, pic, group, &shade, ox, oy);
    }
    // A clip must outlive its own `q ... Q` or it stops applying the moment it
    // is set; everything else is saved and restored so its state does not
    // follow the picture into the next command.
    let scoped = first.action != Action::Clip;
    if scoped {
        out.push_str("q\n");
    }
    write_state(out, first);
    let painter = paint(first, group);
    let mut tips: Vec<(Tip, (f64, f64), f64, f64)> = Vec::new();
    for path in &group.paths {
        let (start, segments) = shortened(path, pic, ox, oy, &mut tips);
        let _ = writeln!(out, "{:.2} {:.2} m", start.0, start.1);
        for segment in segments {
            match segment {
                Segment::Line((x, y)) => {
                    let _ = writeln!(out, "{x:.2} {y:.2} l");
                }
                Segment::Curve((ax, ay), (bx, by), (x, y)) => {
                    let _ = writeln!(out, "{ax:.2} {ay:.2} {bx:.2} {by:.2} {x:.2} {y:.2} c");
                }
            }
        }
        // `s` closes AND strokes in one operator (Table 60), so a path painted
        // by it must not be closed twice; every other painting operator needs
        // the `h` written out, because there is no combined operator for a
        // closed fill.
        if path.closed && painter != "s" {
            out.push_str("h\n");
        }
    }
    out.push_str(painter);
    out.push('\n');
    if scoped {
        out.push_str("Q\n");
    }
    for (tip, at, angle, width) in tips {
        write_tip(out, tip, at, angle, width, first);
    }
}

/// The shading a `\shade` paints, clipped to the path it was written on.
///
/// `\pgfshadepath` (`pgfcoreshade.code.tex` lines 929-950) does exactly this:
/// clip to the path, discard it, then shift to the bounding box's centre,
/// turn by the shading angle, scale, and paint. lualatex writes the three
/// matrices out one after another rather than multiplying them, so this does
/// too, and the operands can be compared with its own.
fn write_shading(out: &mut String, pic: &Picture, group: &Group, shade: &Shade, ox: f64, oy: f64) {
    let mut box_: Option<(f64, f64, f64, f64)> = None;
    for path in &group.paths {
        for (x, y) in path.anchor_points(pic, ox, oy) {
            box_ = Some(match box_ {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    // `\ifdim\pgf@pathminx=16000pt \pgfwarning{No path specified that can be
    // filled}` -- line 882. A shading with no path to clip it paints the page.
    let Some(box_) = box_ else { return };
    let name = pic
        .shades()
        .iter()
        .position(|known| known == shade)
        .unwrap_or(0);
    out.push_str("q\n");
    for path in &group.paths {
        write_outline(out, path, pic, ox, oy);
    }
    out.push_str("W\nn\n");
    let ((cx, cy), angle, (sx, sy)) = shading::placement(shade, box_);
    let _ = writeln!(out, "1 0 0 1 {} {} cm", number(cx), number(cy));
    let (sin, cos) = angle.to_radians().sin_cos();
    let _ = writeln!(
        out,
        "{} {} {} {} 0 0 cm",
        number(cos),
        number(sin),
        number(-sin),
        number(cos)
    );
    let _ = writeln!(out, "{} 0 0 {} 0 0 cm", number(sx), number(sy));
    let _ = writeln!(out, "/pgfsh{name} sh");
    out.push_str("Q\n");
}

/// One subpath's geometry, with no arrow shortening: what a clip wants.
fn write_outline(out: &mut String, path: &Path, pic: &Picture, ox: f64, oy: f64) {
    let place = |(x, y): (f64, f64)| (ox + x * pic.x_scale, oy + y * pic.y_scale);
    let mut start = place(path.start);
    let mut segments: Vec<Segment> = path
        .segments
        .iter()
        .map(|segment| match segment {
            Segment::Line(to) => Segment::Line(place(*to)),
            Segment::Curve(a, b, to) => Segment::Curve(place(*a), place(*b), place(*to)),
        })
        .collect();
    if path.rounded > 0.0 {
        let (head, rounded) = round_corners(start, &segments, path.closed, path.rounded);
        start = head;
        segments = rounded;
    }
    let _ = writeln!(out, "{:.2} {:.2} m", start.0, start.1);
    for segment in &segments {
        match segment {
            Segment::Line((x, y)) => {
                let _ = writeln!(out, "{x:.2} {y:.2} l");
            }
            Segment::Curve((ax, ay), (bx, by), (x, y)) => {
                let _ = writeln!(out, "{ax:.2} {ay:.2} {bx:.2} {by:.2} {x:.2} {y:.2} c");
            }
        }
    }
    if path.closed {
        out.push_str("h\n");
    }
}

/// The painting operator a whole path command ends with (S8.5.3, Table 60).
fn paint(style: &Path, group: &Group) -> &'static str {
    // One closed stroked subpath is what PDF's `s` is for; two of them share a
    // painting operator and have to close themselves.
    let closed_stroke = style.closed && style.action == Action::Draw && group.paths.len() == 1;
    match (style.action, style.even_odd) {
        (Action::Draw, _) => match closed_stroke {
            true => "s",
            false => "S",
        },
        (Action::Fill, false) => "f",
        (Action::Fill, true) => "f*",
        (Action::FillDraw, false) => "B",
        (Action::FillDraw, true) => "B*",
        (Action::Clip, false) => "W\nn",
        (Action::Clip, true) => "W*\nn",
        // `\shadedraw` strokes the border over the shading that has already
        // been painted through the path as a clip.
        (Action::ShadeDraw, _) => "S",
        // A `\path` builds and paints nothing, and a `\shade`'s own paint is
        // the shading written before this: `n` ends the path object without
        // drawing it a second time.
        (Action::None | Action::Shade | Action::Node, _) => "n",
    }
}

/// A number as a content stream wants it: five decimals at most, and no
/// trailing zeros.
///
/// Written with Rust's own float formatting instead, `0.8 * 0.4` comes out as
/// `0.32000000000000006` -- a valid PDF real and seventeen bytes of noise on
/// every stroked arrow tip in the file.
pub fn number(value: f64) -> String {
    let rounded = (value * 100_000.0).round() / 100_000.0;
    // `-0` is a real that no reader distinguishes from `0`, and writing it
    // makes two identical matrices compare unequal byte for byte.
    let rounded = match rounded == 0.0 {
        true => 0.0,
        false => rounded,
    };
    let mut text = format!("{rounded}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    text
}

/// The graphics state one command sets: colours, width, dash, caps, opacity.
fn write_state(out: &mut String, path: &Path) {
    if path.action == Action::Fill
        || path.action == Action::FillDraw
        || (path.filled && path.action != Action::Clip)
    {
        let (r, g, b) = path.fill;
        let _ = writeln!(out, "{} {} {} rg", number(r), number(g), number(b));
    }
    if path.action == Action::Draw || path.action == Action::FillDraw {
        let (r, g, b) = path.stroke;
        let _ = writeln!(out, "{} {} {} RG", number(r), number(g), number(b));
        let _ = writeln!(out, "{} w", number(path.width));
        if path.cap.code() != 0 {
            let _ = writeln!(out, "{} J", path.cap.code());
        }
        if path.join.code() != 0 {
            let _ = writeln!(out, "{} j", path.join.code());
        }
        let dash: Vec<String> = path.dash.iter().map(|d| number(*d)).collect();
        if !dash.is_empty() {
            let _ = writeln!(out, "[{}] 0 d", dash.join(" "));
        }
    }
    // Constant alpha lives in an ExtGState dictionary rather than in an
    // operator of its own (S11.6.4.4), so the name here has to be registered
    // in the page's `/ExtGState` -- `Picture::ext_gstates` lists the ones a
    // picture needs. PGF names them the same way.
    if path.draw_opacity != 1.0 {
        let _ = writeln!(out, "/pgf@CA{} gs", number(path.draw_opacity));
    }
    if path.fill_opacity != 1.0 {
        let _ = writeln!(out, "/pgf@ca{} gs", number(path.fill_opacity));
    }
}

/// A path in page points, with its ends pulled back to make room for arrow
/// tips, and the tips it needs recorded.
fn shortened(
    path: &Path,
    pic: &Picture,
    ox: f64,
    oy: f64,
    tips: &mut Vec<(Tip, (f64, f64), f64, f64)>,
) -> ((f64, f64), Vec<Segment>) {
    let place = |(x, y): (f64, f64)| (ox + x * pic.x_scale, oy + y * pic.y_scale);
    let mut start = place(path.start);
    let mut segments: Vec<Segment> = path
        .segments
        .iter()
        .map(|segment| match segment {
            Segment::Line(to) => Segment::Line(place(*to)),
            Segment::Curve(a, b, to) => Segment::Curve(place(*a), place(*b), place(*to)),
        })
        .collect();
    if path.rounded > 0.0 {
        let (rounded_start, rounded) = round_corners(start, &segments, path.closed, path.rounded);
        start = rounded_start;
        segments = rounded;
    }
    if segments.is_empty() {
        return (start, segments);
    }
    if let Some(tip) = path.arrow_start {
        let toward = match segments[0] {
            Segment::Line(to) => to,
            Segment::Curve(control, _, _) => control,
        };
        let angle = (toward.1 - start.1).atan2(toward.0 - start.0);
        let reach = arrows::head(tip, path.width).reach;
        tips.push((tip, start, angle + std::f64::consts::PI, path.width));
        start = (start.0 + reach * angle.cos(), start.1 + reach * angle.sin());
    }
    if let Some(tip) = path.arrow_end {
        let last = segments.len() - 1;
        let (from, end) = match (&segments[last], last) {
            (Segment::Line(to), 0) => (start, *to),
            (Segment::Line(to), _) => (end_of(&segments[last - 1]), *to),
            (Segment::Curve(_, control, to), _) => (*control, *to),
        };
        let angle = (end.1 - from.1).atan2(end.0 - from.0);
        let reach = arrows::head(tip, path.width).reach;
        let pulled = (end.0 - reach * angle.cos(), end.1 - reach * angle.sin());
        tips.push((tip, pulled, angle, path.width));
        segments[last] = match segments[last] {
            Segment::Line(_) => Segment::Line(pulled),
            Segment::Curve(a, b, _) => Segment::Curve(a, b, pulled),
        };
    }
    (start, segments)
}

/// `rounded corners=r`: every corner cut back by `r` along each leg and the
/// gap bridged by a quarter-circle cubic.
///
/// `pgfcorepathprocessing.code.tex` lines 371-427: the two new points are
/// `\pgfpointlineatdistance{r}` from the corner along each leg, and each
/// control point is 0.5522847 of the way from its end back to the corner --
/// the same constant a quarter ellipse is drawn with, because that is what
/// this is. A corner where a curve arrives keeps its shape, since the corner
/// PGF rounds is the one between two straight pieces.
fn round_corners(
    start: (f64, f64),
    segments: &[Segment],
    closed: bool,
    radius: f64,
) -> ((f64, f64), Vec<Segment>) {
    // The vertices, and whether the piece arriving at each is a straight
    // line. A closed path's first vertex is arrived at by the closing leg,
    // which is always straight.
    let mut points = vec![start];
    let mut straight = vec![closed];
    for segment in segments {
        points.push(end_of(segment));
        straight.push(matches!(segment, Segment::Line(_)));
    }
    let last = points.len() - 1;
    if last < 2 {
        return (start, segments.to_vec());
    }
    let corner = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        // Never cut back further than half a leg: a radius bigger than the
        // side it is on would put the arc past the next corner, which is a
        // path that crosses itself rather than a rounded one.
        let toward = |from: (f64, f64), to: (f64, f64)| {
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let length = dx.hypot(dy);
            let reach = radius.min(length / 2.0);
            match length == 0.0 {
                true => from,
                false => (from.0 + reach * dx / length, from.1 + reach * dy / length),
            }
        };
        let (p, q) = (toward(b, a), toward(b, c));
        let control = |at: (f64, f64)| (at.0 + KAPPA * (b.0 - at.0), at.1 + KAPPA * (b.1 - at.1));
        (p, Segment::Curve(control(p), control(q), q))
    };
    // A vertex is rounded when both the piece arriving at it and the piece
    // leaving it are straight; the two ends of an open path are not rounded
    // at all, because nothing turns there.
    let leaving = |at: usize| match at == last {
        true => closed && straight[0],
        false => straight[at + 1],
    };
    let before = |at: usize| match at == 0 {
        true => points[last],
        false => points[at - 1],
    };
    let after = |at: usize| match at == last {
        true => points[0],
        false => points[at + 1],
    };
    let turned = |at: usize| -> Option<((f64, f64), Segment)> {
        let ends = at == 0 || at == last;
        match straight[at] && leaving(at) && (closed || !ends) {
            true => Some(corner(before(at), points[at], after(at))),
            false => None,
        }
    };
    let mut out: Vec<Segment> = Vec::new();
    // A rounded closed path no longer starts at its first corner: it starts
    // where the arc round that corner leaves off.
    let head = match turned(0) {
        Some((_, Segment::Curve(_, _, to))) => to,
        _ => start,
    };
    for at in 1..=last {
        match turned(at) {
            Some((cut, arc)) => {
                out.push(Segment::Line(cut));
                out.push(arc);
            }
            None => out.push(segments[at - 1]),
        }
    }
    // And it comes back round that first corner at the end, which is the
    // piece the `h` would otherwise have drawn square.
    if let Some((cut, arc)) = turned(0) {
        out.push(Segment::Line(cut));
        out.push(arc);
    }
    (head, out)
}

/// PGF's quarter-circle constant, `pgfcorepathprocessing.code.tex` line 402.
const KAPPA: f64 = 0.5522847;

/// Where a segment ends.
fn end_of(segment: &Segment) -> (f64, f64) {
    match segment {
        Segment::Line(to) => *to,
        Segment::Curve(_, _, to) => *to,
    }
}

/// One arrow tip, placed and turned by a `cm` matrix the way PGF places one.
fn write_tip(out: &mut String, tip: Tip, at: (f64, f64), angle: f64, width: f64, style: &Path) {
    let head = arrows::head(tip, width);
    let (sin, cos) = angle.sin_cos();
    let _ = writeln!(
        out,
        "q {} {} {} {} {:.2} {:.2} cm",
        number(cos),
        number(sin),
        number(-sin),
        number(cos),
        at.0,
        at.1
    );
    let (r, g, b) = style.stroke;
    match head.filled {
        true => {
            let _ = writeln!(out, "{} {} {} rg", number(r), number(g), number(b));
        }
        false => {
            let _ = writeln!(
                out,
                "{} {} {} RG\n{} w\n1 J\n1 j",
                number(r),
                number(g),
                number(b),
                number(head.width)
            );
        }
    }
    // The tip is a solid shape whatever the line was dashed with: PGF resets
    // the pattern with `\pgfsetdash{}{+0pt}` before drawing one, and a dashed
    // arrowhead is a row of specks.
    out.push_str("[] 0 d\n");
    let _ = writeln!(out, "{:.5} {:.5} m", head.start.0, head.start.1);
    for segment in &head.segments {
        match segment {
            Segment::Line((x, y)) => {
                let _ = writeln!(out, "{x:.5} {y:.5} l");
            }
            Segment::Curve((ax, ay), (bx, by), (x, y)) => {
                let _ = writeln!(out, "{ax:.5} {ay:.5} {bx:.5} {by:.5} {x:.5} {y:.5} c");
            }
        }
    }
    out.push_str(match head.filled {
        true => "f\nQ\n",
        false => "S\nQ\n",
    });
}

/// A node's border and its fill. The text is drawn by `draw_on`, which has a
/// page to put glyphs on.
fn write_node(out: &mut String, pic: &Picture, node: &Node, ox: f64, oy: f64) {
    if !node.draw && !node.filled {
        return;
    }
    // The coordinate is where the node's ANCHOR sits, so the centre is that
    // point moved back by however far the anchor is from it: a node written
    // `above` a point has its south edge on the point and its middle a half
    // height further up.
    let (ax, ay) = (ox + node.at.0 * pic.x_scale, oy + node.at.1 * pic.y_scale);
    let border = node.border();
    // The coordinate is where the node's ANCHOR sits, and an anchor is a
    // point on the SHAPE: a circle's `north east` is on the circle and not on
    // the corner of the box around it, so where the centre goes depends on
    // which shape it is.
    let (adx, ady) = border.anchor(node.anchor);
    let (cx, cy) = (ax - adx, ay - ady);
    let (half_width, half_height) = border.drawn();
    out.push_str("q\n");
    if node.filled {
        let (r, g, b) = node.fill;
        let _ = writeln!(out, "{} {} {} rg", number(r), number(g), number(b));
    }
    if node.draw {
        let (r, g, b) = node.stroke;
        let _ = writeln!(
            out,
            "{} {} {} RG\n{} w",
            number(r),
            number(g),
            number(b),
            number(node.width)
        );
    }
    match node.shape {
        // Nothing to draw at all: a `\coordinate` is a name for a point.
        Shape::Coordinate => {
            out.push_str("Q\n");
            return;
        }
        Shape::Rectangle => {
            let _ = writeln!(
                out,
                "{:.2} {:.2} {:.2} {:.2} re",
                cx - half_width,
                cy - half_height,
                2.0 * half_width,
                2.0 * half_height
            );
        }
        // A circle and an ellipse are the same four cubics; the circle's two
        // radii happen to be equal. `\pgfpathellipse` starts at the east
        // point and goes anticlockwise -- `pgfcorepathconstruct` line 357.
        Shape::Circle | Shape::Ellipse => {
            let (rx, ry) = (half_width, half_height);
            let (kx, ky) = (0.552_284_75 * rx, 0.552_284_75 * ry);
            let _ = writeln!(out, "{:.2} {cy:.2} m", cx + rx);
            for (c1, c2, to) in [
                ((cx + rx, cy + ky), (cx + kx, cy + ry), (cx, cy + ry)),
                ((cx - kx, cy + ry), (cx - rx, cy + ky), (cx - rx, cy)),
                ((cx - rx, cy - ky), (cx - kx, cy - ry), (cx, cy - ry)),
                ((cx + kx, cy - ry), (cx + rx, cy - ky), (cx + rx, cy)),
            ] {
                let _ = writeln!(
                    out,
                    "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
                    c1.0, c1.1, c2.0, c2.1, to.0, to.1
                );
            }
            out.push_str("h\n");
        }
        // `pgflibraryshapes.geometric.code.tex` lines 336-340: east, north,
        // west, south, closed -- in that order.
        Shape::Diamond => {
            let _ = writeln!(out, "{:.2} {cy:.2} m", cx + half_width);
            for (x, y) in [
                (cx, cy + half_height),
                (cx - half_width, cy),
                (cx, cy - half_height),
            ] {
                let _ = writeln!(out, "{x:.2} {y:.2} l");
            }
            out.push_str("h\n");
        }
    }
    out.push_str(match (node.filled, node.draw) {
        (true, true) => "B\n",
        (true, false) => "f\n",
        _ => "S\n",
    });
    out.push_str("Q\n");
}
