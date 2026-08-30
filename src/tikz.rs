//! The part of TikZ that is straight lines.
//!
//! TikZ is a language: coordinates, transforms, styles, node placement, curves,
//! decorations, a whole layer of its own on top of PGF. This reads the subset
//! these documents draw their marks with -- polylines with `--`, an optional
//! `cycle`, a line width, and the picture's x/y scale -- and turns it into PDF
//! path operators. Anything it does not recognise it drops rather than guesses
//! at, because a wrong line in a logo is worse than a missing one.
//!
//! What is NOT here: curves (`..controls`), nodes, arrows, patterns, shadings,
//! coordinate arithmetic, named coordinates, loops. A picture using them comes
//! out with whatever straight segments it also had, and no more.

/// A stroked polyline, in the picture's own units.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub points: Vec<(f64, f64)>,
    /// Whether the path closes back to its first point (`-- cycle`).
    pub closed: bool,
    /// Stroke width in points, as `line width=` gave it.
    pub width: f64,
}

/// A whole `tikzpicture`: its paths and the scale its options set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picture {
    pub paths: Vec<Path>,
    /// `x=0.38pt` and `y=0.38pt` scale every coordinate.
    pub x_scale: f64,
    pub y_scale: f64,
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
}

/// Read a `tikzpicture` body, with the options from its `[...]`.
pub fn parse(options: &str, body: &str) -> Picture {
    let mut pic = Picture {
        x_scale: dimension(options, "x=").unwrap_or(1.0),
        y_scale: dimension(options, "y=").unwrap_or(1.0),
        ..Picture::default()
    };
    // Each `\draw ... ;` is one command however many lines it spans, which is
    // the same rule the prelude's delimited macro uses.
    for cmd in body.split('\\') {
        let Some(rest) = cmd.strip_prefix("draw") else {
            continue;
        };
        let Some(end) = rest.find(';') else { continue };
        let (opts, coords) = split_options(&rest[..end]);
        let width = dimension(&opts, "line width=").unwrap_or(0.4);
        pic.paths.extend(paths_in(coords, width));
    }
    pic
}

/// `[a,b=c]` at the start, and everything after it.
fn split_options(text: &str) -> (String, &str) {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('[') {
        return (String::new(), trimmed);
    }
    match trimmed.find(']') {
        Some(i) => (trimmed[1..i].to_string(), &trimmed[i + 1..]),
        None => (String::new(), trimmed),
    }
}

/// A `key=<number>pt` out of an option list, in points.
fn dimension(options: &str, key: &str) -> Option<f64> {
    let at = options.find(key)? + key.len();
    let rest = &options[at..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// The polylines in one `\draw`'s coordinate text.
///
/// A single `\draw` may carry several disjoint paths -- `(a) -- (b) (c) -- (d)`
/// is two segments, not one -- so a coordinate that follows a coordinate rather
/// than a `--` starts a new path.
fn paths_in(coords: &str, width: f64) -> Vec<Path> {
    let mut out = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut closed = false;
    let mut connected = false;
    let mut rest = coords;

    while let Some(open) = rest.find('(') {
        let before = &rest[..open];
        if before.contains("cycle") {
            closed = true;
        }
        // A gap with no `--` in it ends the path that was being drawn.
        if !current.is_empty() && !before.contains("--") {
            out.push(Path {
                points: std::mem::take(&mut current),
                closed,
                width,
            });
            closed = false;
        }
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let inside = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let mut parts = inside.split(',');
        let (Some(x), Some(y)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(x), Ok(y)) = (x.trim().parse::<f64>(), y.trim().parse::<f64>()) else {
            continue;
        };
        current.push((x, y));
        connected = true;
    }
    if rest.contains("cycle") {
        closed = true;
    }
    if connected && !current.is_empty() {
        out.push(Path {
            points: current,
            closed,
            width,
        });
    }
    out
}

/// The PDF operators that stroke a picture, offset to `(ox, oy)`.
///
/// PDF's y axis points up from the bottom of the page, which is the same way
/// TikZ's does, so the coordinates go through unflipped.
pub fn to_pdf_ops(pic: &Picture, ox: f64, oy: f64) -> String {
    let mut out = String::new();
    for path in &pic.paths {
        if path.points.len() < 2 {
            continue;
        }
        out.push_str(&format!("{} w\n", path.width));
        for (i, (x, y)) in path.points.iter().enumerate() {
            let px = ox + x * pic.x_scale;
            let py = oy + y * pic.y_scale;
            let op = match i {
                0 => "m",
                _ => "l",
            };
            out.push_str(&format!("{px:.2} {py:.2} {op}\n"));
        }
        out.push_str(match path.closed {
            true => "s\n",
            false => "S\n",
        });
    }
    out
}
