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
use super::options::{Shape, Tip};
use super::scan::Action;
use super::{Node, Path, Picture, Segment};

/// Everything one path command draws, ready to be written out.
struct Group<'a> {
    paths: Vec<&'a Path>,
}

/// The operators that draw `pic`, with its origin at `(ox, oy)`.
pub fn to_pdf_ops(pic: &Picture, ox: f64, oy: f64) -> String {
    let mut out = String::new();
    // The whole picture is one saved graphics state, so a `\clip` inside it
    // stops applying where the picture stops rather than at the page's end.
    out.push_str("q\n");
    for group in groups(pic) {
        write_group(&mut out, pic, &group, ox, oy);
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
fn write_group(out: &mut String, pic: &Picture, group: &Group, ox: f64, oy: f64) {
    let Some(first) = group.paths.first() else {
        return;
    };
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
        // A `\path` builds and paints nothing, and a `\shade` would need a
        // shading dictionary this does not write -- `n` ends the path object
        // without drawing, which leaves the picture missing rather than wrong.
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
        start = (
            start.0 + reach * angle.cos(),
            start.1 + reach * angle.sin(),
        );
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

/// Where a segment ends.
fn end_of(segment: &Segment) -> (f64, f64) {
    match segment {
        Segment::Line(to) => *to,
        Segment::Curve(_, _, to) => *to,
    }
}

/// One arrow tip, placed and turned by a `cm` matrix the way PGF places one.
fn write_tip(
    out: &mut String,
    tip: Tip,
    at: (f64, f64),
    angle: f64,
    width: f64,
    style: &Path,
) {
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
    let (ax, ay) = (
        ox + node.at.0 * pic.x_scale,
        oy + node.at.1 * pic.y_scale,
    );
    let (half_width, half_height) = node.half_size();
    let (dx, dy) = node.anchor.offset();
    let (cx, cy) = (ax - dx * half_width, ay - dy * half_height);
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
        Shape::Circle => {
            // The circle shape's radius is the length of the vector whose
            // components are the half-width and half-height plus the inner
            // separation -- `pgfmoduleshapes.code.tex` lines 1198-1235.
            let radius = (half_width * half_width + half_height * half_height).sqrt();
            let k = 0.552_284_75 * radius;
            let _ = writeln!(out, "{:.2} {cy:.2} m", cx + radius);
            for (c1, c2, to) in [
                ((cx + radius, cy + k), (cx + k, cy + radius), (cx, cy + radius)),
                ((cx - k, cy + radius), (cx - radius, cy + k), (cx - radius, cy)),
                ((cx - radius, cy - k), (cx - k, cy - radius), (cx, cy - radius)),
                ((cx + k, cy - radius), (cx + radius, cy - k), (cx + radius, cy)),
            ] {
                let _ = writeln!(
                    out,
                    "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c",
                    c1.0, c1.1, c2.0, c2.1, to.0, to.1
                );
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
