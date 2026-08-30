//! Reading a `\special`, ported from the `spc_*.c` family in tectonic's
//! `xdvipdfmx`.
//!
//! A `\special` is TeX's escape hatch: whatever is inside it goes into the DVI
//! untouched, and what it means is between the document and the driver. TeX
//! itself has no opinion, which is how colour, graphics, hyperlinks and page
//! size all reach a page in an engine that knows about none of them.
//!
//! That freedom is why a driver needs a parser rather than a lookup: the same
//! file may carry dvips's spelling, dvipdfmx's, HTML links from a converter
//! nobody remembers, and a raw PostScript fragment. `xdvipdfmx` sorts them by
//! recognising prefixes and reading each family's own grammar; this does the
//! same, and hands back the ones it does not know as they were, because a
//! driver that dropped an unknown special would silently lose a document's
//! links or its colours.
//!
//! What is read here is what texrs will emit and what a document is likely to
//! carry: colour, page size, an included figure, PDF destinations and
//! annotations, and HTML links.

/// A colour, in the space it was written in.
#[derive(Debug, Clone, PartialEq)]
pub enum Colour {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
    /// One of the names `color.pro` defines, which a document may use without
    /// saying what it means.
    Named(String),
}

impl Colour {
    /// The colour as red, green and blue, which is what a page is drawn in.
    ///
    /// The conversion from CMYK is the one every driver uses, subtracting the
    /// black from each of the others, and is not colour management: it is what
    /// dvips and dvipdfmx do, so a document looks the same through this as
    /// through them.
    pub fn rgb(&self) -> (f64, f64, f64) {
        match self {
            Colour::Gray(value) => (*value, *value, *value),
            Colour::Rgb(r, g, b) => (*r, *g, *b),
            Colour::Cmyk(c, m, y, k) => (
                1.0 - (c + k).min(1.0),
                1.0 - (m + k).min(1.0),
                1.0 - (y + k).min(1.0),
            ),
            Colour::Named(name) => named_colour(name).unwrap_or((0.0, 0.0, 0.0)),
        }
    }
}

/// What a `\special` says.
#[derive(Debug, Clone, PartialEq)]
pub enum Special {
    /// `color push rgb 1 0 0`: the colour everything takes until it is popped.
    ColourPush(Colour),
    ColourPop,
    /// `background gray 0.9`, which colours the page rather than the ink.
    Background(Colour),
    /// `papersize=210mm,297mm`, in scaled points.
    PaperSize {
        width: i64,
        height: i64,
    },
    /// `PSfile="fig.eps" llx=0 lly=0 urx=100 ury=50 rwi=1000`: a figure, its
    /// bounding box in PostScript points, and how wide to draw it in tenths of
    /// a point.
    Figure {
        name: String,
        bbox: [f64; 4],
        width: Option<f64>,
        height: Option<f64>,
        angle: Option<f64>,
    },
    /// `pdf:dest (name) [ … ]`: somewhere a link can point at.
    PdfDest {
        name: String,
        rest: String,
    },
    /// `pdf:` anything else, kept whole: a driver that understands it will,
    /// and one that does not must not mangle it.
    Pdf {
        operator: String,
        rest: String,
    },
    /// `html:<a href="…">` and its closing `</a>`.
    HtmlAnchor {
        href: String,
    },
    HtmlEnd,
    /// Anything else, as it was.
    Unknown(String),
}

/// Read one `\special`'s text.
pub fn parse(text: &str) -> Special {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();

    // dvips's colour stack, which is what every colour package writes.
    if let Some(rest) = lower.strip_prefix("color ") {
        let rest = trimmed[trimmed.len() - rest.len()..].trim();
        if rest.eq_ignore_ascii_case("pop") {
            return Special::ColourPop;
        }
        if let Some(pushed) = rest
            .strip_prefix("push ")
            .or_else(|| rest.strip_prefix("Push "))
        {
            if let Some(colour) = colour(pushed) {
                return Special::ColourPush(colour);
            }
        }
        // `color rgb 1 0 0` with no push sets the colour outright, which older
        // documents do.
        if let Some(colour) = colour(rest) {
            return Special::ColourPush(colour);
        }
    }
    if let Some(rest) = lower.strip_prefix("background ") {
        let rest = &trimmed[trimmed.len() - rest.len()..];
        if let Some(colour) = colour(rest) {
            return Special::Background(colour);
        }
    }

    if let Some(rest) = lower.strip_prefix("papersize=") {
        let rest = &trimmed[trimmed.len() - rest.len()..];
        if let Some((width, height)) = rest.split_once(',') {
            if let (Some(width), Some(height)) = (dimension(width), dimension(height)) {
                return Special::PaperSize { width, height };
            }
        }
    }

    // A figure, which dvips spells `PSfile=` and other tools spell `psfile=`.
    if lower.starts_with("psfile=") || lower.starts_with("psfile ") {
        return figure(trimmed);
    }

    if let Some(rest) = trimmed.strip_prefix("pdf:") {
        let rest = rest.trim();
        let (operator, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if operator == "dest" {
            let tail = tail.trim();
            if let Some(name) = between(tail, '(', ')') {
                return Special::PdfDest {
                    name,
                    rest: tail.to_string(),
                };
            }
        }
        return Special::Pdf {
            operator: operator.to_string(),
            rest: tail.trim().to_string(),
        };
    }

    if let Some(rest) = trimmed.strip_prefix("html:") {
        let rest = rest.trim();
        if rest.eq_ignore_ascii_case("</a>") {
            return Special::HtmlEnd;
        }
        if rest.to_ascii_lowercase().starts_with("<a ") {
            if let Some(href) = between(rest, '"', '"') {
                return Special::HtmlAnchor { href };
            }
        }
    }

    Special::Unknown(trimmed.to_string())
}

/// What is between the first `open` and the next `close`.
fn between(text: &str, open: char, close: char) -> Option<String> {
    let from = text.find(open)? + open.len_utf8();
    let to = text[from..].find(close)? + from;
    Some(text[from..to].to_string())
}

/// `rgb 1 0 0`, `gray 0.5`, `cmyk 0 1 1 0`, or a name.
fn colour(text: &str) -> Option<Colour> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let number = |word: &str| word.parse::<f64>().ok();
    match words.as_slice() {
        [space, values @ ..] if space.eq_ignore_ascii_case("rgb") => {
            let values: Vec<f64> = values.iter().filter_map(|w| number(w)).collect();
            match values.as_slice() {
                [r, g, b, ..] => Some(Colour::Rgb(*r, *g, *b)),
                _ => None,
            }
        }
        [space, values @ ..] if space.eq_ignore_ascii_case("cmyk") => {
            let values: Vec<f64> = values.iter().filter_map(|w| number(w)).collect();
            match values.as_slice() {
                [c, m, y, k, ..] => Some(Colour::Cmyk(*c, *m, *y, *k)),
                _ => None,
            }
        }
        [space, value, ..] if space.eq_ignore_ascii_case("gray") => number(value).map(Colour::Gray),
        [name] => Some(Colour::Named(name.to_string())),
        _ => None,
    }
}
/// A TeX dimension, in scaled points -- and in the same scaled points TeX
/// itself would compute, which is not the same as the arithmetic gives.
///
/// `tex.web` §453-§458: a dimension is scanned as an integer and a fraction,
/// turned into scaled points, and only then multiplied by the unit's ratio,
/// with the division truncated. Doing it the other way round -- converting the
/// number to points and scaling at the end -- is out by one for `1cc` and by
/// five for `210mm`, which was worth finding out from tex rather than
/// reasoning about: a driver that disagreed with the engine about the size of a
/// page would put the page in the wrong place.
pub fn dimension(text: &str) -> Option<i64> {
    let text = text.trim();
    let at = text.find(|c: char| c.is_ascii_alphabetic())?;
    let (number, unit) = text.split_at(at);
    let number = number.trim();
    let unit = unit.trim().to_ascii_lowercase();

    let (sign, number) = match number.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, number.trim_start_matches('+')),
    };
    let (whole, fraction) = match number.split_once(['.', ',']) {
        Some((whole, fraction)) => (whole, fraction),
        None => (number, ""),
    };
    let whole: i64 = match whole.is_empty() {
        true => 0,
        false => whole.parse().ok()?,
    };
    if !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    // §452 `round_decimals`: the digits are folded in from the right, so the
    // fraction is the nearest sixteen-bit one rather than a truncation.
    let f = crate::dimen::round_decimals(fraction);
    let scaled = whole.checked_mul(65536)?.checked_add(f)?;

    // §458: the unit's ratio, applied to the scaled value, truncating. The
    // table lives in `dimen.rs` with the rest of TeX's dimension arithmetic --
    // the engine reads the same units from tokens, and two copies of a
    // conversion table are two chances to fix only one of them.
    let (num, denom) = crate::dimen::unit_ratio(&unit)?;
    if unit == "sp" {
        // A dimension in scaled points is already one.
        return Some(sign * scaled / 65536);
    }
    // §460: a dimension may not exceed 16383.99998pt. tex clamps and
    // complains; a driver reading a paper size has nothing to complain to, so
    // it clamps quietly -- but it must clamp, or a page would come out with a
    // negative width when the number wrapped.
    let value = sign * scaled.checked_mul(num)? / denom;
    Some(value.clamp(-MAX_DIMEN, MAX_DIMEN))
}

/// The largest dimension TeX can hold: 2^30 - 1 scaled points, which is
/// 16383.99998pt. Defined once, in `dimen.rs`.
pub use crate::dimen::MAX_DIMEN;

/// `PSfile="fig.eps" llx=0 lly=0 urx=100 ury=50 rwi=1000`.
fn figure(text: &str) -> Special {
    let mut name = String::new();
    let mut bbox = [0.0f64; 4];
    let (mut width, mut height, mut angle) = (None, None, None);
    for (key, value) in keys(text) {
        let number = value.trim_matches('"').parse::<f64>().ok();
        match key.to_ascii_lowercase().as_str() {
            "psfile" => name = value.trim_matches('"').to_string(),
            "llx" => bbox[0] = number.unwrap_or(0.0),
            "lly" => bbox[1] = number.unwrap_or(0.0),
            "urx" => bbox[2] = number.unwrap_or(0.0),
            "ury" => bbox[3] = number.unwrap_or(0.0),
            // dvips counts these in tenths of a point.
            "rwi" => width = number.map(|n| n / 10.0),
            "rhi" => height = number.map(|n| n / 10.0),
            "angle" => angle = number,
            _ => {}
        }
    }
    Special::Figure {
        name,
        bbox,
        width,
        height,
        angle,
    }
}

/// The `key=value` pairs of a special, where a value may be quoted and hold
/// spaces.
fn keys(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < chars.len() {
        while at < chars.len() && (chars[at].is_whitespace() || chars[at] == ',') {
            at += 1;
        }
        let start = at;
        while at < chars.len() && chars[at] != '=' && !chars[at].is_whitespace() {
            at += 1;
        }
        let key: String = chars[start..at].iter().collect();
        if chars.get(at) != Some(&'=') {
            if !key.is_empty() {
                out.push((key, String::new()));
            }
            continue;
        }
        at += 1;
        let value: String = match chars.get(at) {
            Some('"') => {
                at += 1;
                let start = at;
                while at < chars.len() && chars[at] != '"' {
                    at += 1;
                }
                let value = chars[start..at].iter().collect();
                at += 1;
                value
            }
            _ => {
                let start = at;
                while at < chars.len() && !chars[at].is_whitespace() {
                    at += 1;
                }
                chars[start..at].iter().collect()
            }
        };
        out.push((key, value));
    }
    out
}

/// The colours `color.pro` names, which a document may use without saying what
/// they mean. These are the ones dvips defines; a name this does not know is
/// black, as it is there.
fn named_colour(name: &str) -> Option<(f64, f64, f64)> {
    let cmyk = |c: f64, m: f64, y: f64, k: f64| {
        Some((
            1.0 - (c + k).min(1.0),
            1.0 - (m + k).min(1.0),
            1.0 - (y + k).min(1.0),
        ))
    };
    match name {
        "Black" => cmyk(0.0, 0.0, 0.0, 1.0),
        "White" => cmyk(0.0, 0.0, 0.0, 0.0),
        "Red" => cmyk(0.0, 1.0, 1.0, 0.0),
        "Green" => cmyk(1.0, 0.0, 1.0, 0.0),
        "Blue" => cmyk(1.0, 1.0, 0.0, 0.0),
        "Cyan" => cmyk(1.0, 0.0, 0.0, 0.0),
        "Magenta" => cmyk(0.0, 1.0, 0.0, 0.0),
        "Yellow" => cmyk(0.0, 0.0, 1.0, 0.0),
        "Gray" => cmyk(0.0, 0.0, 0.0, 0.5),
        "Orange" => cmyk(0.0, 0.61, 0.87, 0.0),
        "Purple" => cmyk(0.45, 0.86, 0.0, 0.0),
        "Brown" => cmyk(0.0, 0.81, 1.0, 0.6),
        _ => None,
    }
}

impl std::fmt::Display for Special {
    /// What a special means, in words -- which is what `-X special` prints and
    /// what a person wants when a page comes out the wrong colour.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Special::ColourPush(colour) => {
                let (r, g, b) = colour.rgb();
                write!(f, "colour push   {colour:?}  = rgb {r} {g} {b}")
            }
            Special::ColourPop => write!(f, "colour pop"),
            Special::Background(colour) => write!(f, "background    {colour:?}"),
            Special::PaperSize { width, height } => write!(
                f,
                "paper size    {width} by {height} sp ({:.2} by {:.2} pt)",
                *width as f64 / 65536.0,
                *height as f64 / 65536.0
            ),
            Special::Figure {
                name,
                bbox,
                width,
                height,
                angle,
            } => {
                write!(f, "figure        {name}  box {bbox:?}")?;
                if let Some(width) = width {
                    write!(f, "  width {width}pt")?;
                }
                if let Some(height) = height {
                    write!(f, "  height {height}pt")?;
                }
                match angle {
                    Some(angle) => write!(f, "  angle {angle}"),
                    None => Ok(()),
                }
            }
            Special::PdfDest { name, .. } => write!(f, "destination   {name}"),
            Special::Pdf { operator, rest } => write!(f, "pdf {operator}  {rest}"),
            Special::HtmlAnchor { href } => write!(f, "link          {href}"),
            Special::HtmlEnd => write!(f, "link end"),
            Special::Unknown(text) => write!(f, "not read      {text}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The colour stack, which is what every colour package writes.
    #[test]
    fn a_colour_special_says_which_colour_and_in_which_space() {
        assert_eq!(
            parse("color push rgb 1 0 0"),
            Special::ColourPush(Colour::Rgb(1.0, 0.0, 0.0))
        );
        assert_eq!(parse("color pop"), Special::ColourPop);
        assert_eq!(
            parse("color push gray 0.5"),
            Special::ColourPush(Colour::Gray(0.5))
        );
        assert_eq!(
            parse("color push cmyk 0 1 1 0"),
            Special::ColourPush(Colour::Cmyk(0.0, 1.0, 1.0, 0.0))
        );
        // A name, which a document may use without saying what it means.
        assert_eq!(
            parse("color push Blue"),
            Special::ColourPush(Colour::Named("Blue".into()))
        );
        // And the page's own colour, which is not the ink's.
        assert_eq!(
            parse("background gray 0.9"),
            Special::Background(Colour::Gray(0.9))
        );

        // What a page is drawn in. CMYK red is RGB red, and the conversion is
        // the one dvips uses rather than colour management.
        assert_eq!(Colour::Cmyk(0.0, 1.0, 1.0, 0.0).rgb(), (1.0, 0.0, 0.0));
        assert_eq!(Colour::Gray(0.25).rgb(), (0.25, 0.25, 0.25));
        assert_eq!(Colour::Named("Red".into()).rgb(), (1.0, 0.0, 0.0));
        assert_eq!(Colour::Named("Black".into()).rgb(), (0.0, 0.0, 0.0));
        // A name nobody defines is black, which is what a driver does.
        assert_eq!(Colour::Named("Chartreuse".into()).rgb(), (0.0, 0.0, 0.0));
    }

    /// A dimension is in scaled points, and TeX's inch is not PostScript's.
    #[test]
    fn a_dimension_is_read_in_scaled_points() {
        // Every one of these came from tex itself, by assigning the dimension
        // to a \\count, which prints it in scaled points exactly.
        for (written, sp) in [
            ("1pt", 65536),
            ("1in", 4736286),
            ("10mm", 1864679),
            ("2.5cm", 4661699),
            ("72bp", 4736286),
            ("1dd", 70124),
            ("1cc", 841489),
            ("1pc", 786432),
            ("210mm", 39158276),
            ("0.5in", 2368143),
            ("100sp", 100),
        ] {
            assert_eq!(dimension(written), Some(sp), "{written}");
        }
        // A big point is PostScript's, and 72 of them make an inch where 72.27
        // TeX points do.
        assert_eq!(dimension("72bp"), dimension("1in"));
        assert_eq!(dimension("-1in"), Some(-4736286));
        assert_eq!(dimension("2.5 cm"), dimension("2.5cm"));
        // A dimension too big for TeX is the biggest TeX has, which is what
        // tex itself stores after complaining.
        assert_eq!(dimension("297in"), Some(MAX_DIMEN));
        assert_eq!(dimension("-297in"), Some(-MAX_DIMEN));
        assert_eq!(MAX_DIMEN, 1073741823);
        // Scaled points are whole: tex truncates the fraction.
        assert_eq!(dimension("123.456sp"), Some(123));
        assert_eq!(dimension("2.5sp"), Some(2));

        assert_eq!(dimension("1furlong"), None);
        assert_eq!(dimension("nonsense"), None);

        assert_eq!(
            parse("papersize=210mm,297mm"),
            Special::PaperSize {
                width: dimension("210mm").unwrap(),
                height: dimension("297mm").unwrap()
            }
        );
    }

    /// A figure, as dvips spells one.
    #[test]
    fn a_figure_carries_its_name_and_its_box() {
        let figure = parse("PSfile=\"fig.eps\" llx=0 lly=0 urx=100 ury=50 rwi=1000");
        assert_eq!(
            figure,
            Special::Figure {
                name: "fig.eps".into(),
                bbox: [0.0, 0.0, 100.0, 50.0],
                // dvips counts the width in tenths of a point.
                width: Some(100.0),
                height: None,
                angle: None,
            }
        );

        // A name with a space in it, which is why the value may be quoted, and
        // a rotation.
        let figure = parse("psfile=\"my figure.eps\" llx=-10 lly=-10 urx=10 ury=10 angle=90");
        match figure {
            Special::Figure {
                name, bbox, angle, ..
            } => {
                assert_eq!(name, "my figure.eps");
                assert_eq!(bbox, [-10.0, -10.0, 10.0, 10.0]);
                assert_eq!(angle, Some(90.0));
            }
            other => panic!("{other:?}"),
        }
    }

    /// The PDF and HTML families, and everything else.
    #[test]
    fn the_other_families_are_recognised_or_kept_whole() {
        assert_eq!(
            parse("pdf:dest (section.1) [ @thispage /XYZ @xpos @ypos null ]"),
            Special::PdfDest {
                name: "section.1".into(),
                rest: "(section.1) [ @thispage /XYZ @xpos @ypos null ]".into()
            }
        );
        assert_eq!(
            parse("pdf:literal q 1 0 0 1 0 0 cm"),
            Special::Pdf {
                operator: "literal".into(),
                rest: "q 1 0 0 1 0 0 cm".into()
            }
        );
        assert_eq!(
            parse("html:<a href=\"http://example.com\">"),
            Special::HtmlAnchor {
                href: "http://example.com".into()
            }
        );
        assert_eq!(parse("html:</a>"), Special::HtmlEnd);

        // Anything else comes back as it was: a driver that dropped an unknown
        // special would silently lose whatever it carried.
        assert_eq!(
            parse("ps: gsave 0 0 moveto"),
            Special::Unknown("ps: gsave 0 0 moveto".into())
        );
        assert_eq!(parse(""), Special::Unknown(String::new()));
        // Something that begins like a colour and is not one.
        assert_eq!(
            parse("color wobble"),
            Special::ColourPush(Colour::Named("wobble".into()))
        );
    }
}
