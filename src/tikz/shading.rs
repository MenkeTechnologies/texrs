//! `\shade` and `\shadedraw`: the colour ramps PGF declares, and the PDF
//! shading dictionaries they come to (§15.5, PDF 32000-1 §8.7.4.5).
//!
//! A shading is not an operator with operands. PDF puts the whole ramp in a
//! dictionary -- what type it is, where its axis or its circles are, and a
//! FUNCTION from the parameter to a colour -- and `sh` names that dictionary
//! out of the page's `/Shading` resource. So the geometry and the colours are
//! settled here and the page has to carry the entry; `Picture::shadings` is
//! the list of what it has to carry.
//!
//! The ramps are TikZ's own, stop by stop, out of `tikz.code.tex`:
//!
//! ```text
//! \pgfdeclareverticalshading{axis}{100bp}{
//!   color(0bp)=(bottom); color(25bp)=(bottom); color(50bp)=(middle);
//!   color(75bp)=(top);   color(100bp)=(top)}                    lines 628-633
//! \pgfdeclareradialshading{ball}{(-10bp,10bp)}{
//!   color(0bp)=(ball!15!white);  color(9bp)=(ball!75!white);
//!   color(18bp)=(ball!70!black); color(25bp)=(ball!50!black);
//!   color(50bp)=(black)}                                        lines 639-644
//! \pgfdeclareradialshading{radial}{origin}{
//!   color(0bp)=(inner); color(25bp)=(outer); color(50bp)=(outer)} 648-651
//! ```
//!
//! The two flat stops at each end of `axis` are why a `top color`/`bottom
//! color` box has a band of even colour at top and bottom and only changes
//! over the middle half: a straight two-stop ramp is a different picture.
//!
//! Where the ramp goes is `\pgfshadepath`, `pgfcoreshade.code.tex` lines
//! 881-954: the path's bounding box, its centre, the shading angle, and
//!
//! ```text
//! xscale = 1pt/50bp * (w|cos a| + h|sin a|) / (|cos a| + |sin a|)
//! yscale = 1pt/50bp * (w|sin a| + h|cos a|) / (|cos a| + |sin a|)
//! ```
//!
//! from lines 904-906. lualatex writes `0.567 0 0 1.134` for a 56.69 by 28.35
//! box shaded at 90 degrees, which is those two divided by 50.

use crate::colour::Rgb;

use super::options::{Shade, Shading};

/// Which of PDF's two shading types a ramp is (Table 78).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Type 2: colour along a line.
    Axial,
    /// Type 3: colour between two circles.
    Radial,
}

impl Kind {
    /// The number `/ShadingType` takes.
    pub fn code(self) -> u8 {
        match self {
            Kind::Axial => 2,
            Kind::Radial => 3,
        }
    }
}

/// One shading, ready to be written into a page's `/Shading` resource.
///
/// Everything is in the shading's OWN space, which is the 100 by 100 box PGF
/// declares its shadings in, centred on the origin: the matrix the content
/// stream sets before `sh` is what puts that box over the path.
#[derive(Debug, Clone, PartialEq)]
pub struct Ramp {
    /// The name the content stream calls it by -- `/pgfsh0`.
    pub name: String,
    pub kind: Kind,
    /// `/Coords`: two points for an axial shading, two circles for a radial.
    pub coords: Vec<f64>,
    /// The colour at each point of `0..=1` along it, in order.
    pub stops: Vec<(f64, Rgb)>,
    /// `/Extend`: whether the colour carries on past each end. A radial
    /// shading extends INWARD, because the inner circle has no area and
    /// leaving it unextended puts an unpainted point at the middle.
    pub extend: (bool, bool),
}

impl Ramp {
    /// The ramp a TikZ shading asks for, under the given name.
    pub fn of(shade: &Shade, name: &str) -> Ramp {
        match shade.kind {
            // `tikz.code.tex` lines 628-633. The shading runs along y from the
            // bottom of the 100bp box to its top, and the box is centred on
            // the origin here, so the axis is -50 to 50.
            Shading::Axis => Ramp {
                name: name.to_string(),
                kind: Kind::Axial,
                coords: vec![0.0, -50.0, 0.0, 50.0],
                stops: vec![
                    (0.0, shade.bottom),
                    (0.25, shade.bottom),
                    (0.5, shade.middle),
                    (0.75, shade.top),
                    (1.0, shade.top),
                ],
                extend: (false, false),
            },
            // Lines 648-651: inner at the centre, outer from a quarter of the
            // way out. lualatex writes `/Coords [50.00064 50.00064 0.0
            // 50.00064 50.00064 50.00064] ... /Extend [true false]`, which is
            // the same two circles about the box's middle.
            Shading::Radial => Ramp {
                name: name.to_string(),
                kind: Kind::Radial,
                coords: vec![0.0, 0.0, 0.0, 0.0, 0.0, 50.0],
                stops: vec![(0.0, shade.inner), (0.5, shade.outer), (1.0, shade.outer)],
                extend: (true, false),
            },
            // Lines 639-644. The highlight is up and to the left of the
            // middle -- `\pgfqpoint{-10bp}{10bp}` -- which is what makes the
            // sphere look lit rather than merely round.
            Shading::Ball => Ramp {
                name: name.to_string(),
                kind: Kind::Radial,
                coords: vec![-10.0, 10.0, 0.0, -10.0, 10.0, 50.0],
                stops: vec![
                    (0.0, mix(shade.ball, WHITE, 0.15)),
                    (0.18, mix(shade.ball, WHITE, 0.75)),
                    (0.36, mix(shade.ball, BLACK, 0.70)),
                    (0.5, mix(shade.ball, BLACK, 0.50)),
                    (1.0, BLACK),
                ],
                extend: (true, false),
            },
        }
    }

    /// The `/Shading` dictionary this ramp is, as PDF source.
    ///
    /// The colour is a stitching function (§7.10.4) over one exponential
    /// function per gap between stops, which is what a piecewise-linear ramp
    /// has to be written as: PDF has no "list of stops".
    pub fn dictionary(&self) -> String {
        let number = super::render::number;
        let mut functions = String::new();
        let mut bounds = String::new();
        let mut encode = String::new();
        for pair in self.stops.windows(2) {
            let ((_, from), (at, to)) = (pair[0], pair[1]);
            functions.push_str(&format!(
                "<< /FunctionType 2 /Domain [0 1] /C0 [{} {} {}] /C1 [{} {} {}] /N 1 >> ",
                number(from.0),
                number(from.1),
                number(from.2),
                number(to.0),
                number(to.1),
                number(to.2),
            ));
            encode.push_str("0 1 ");
            if at < 1.0 {
                bounds.push_str(&format!("{} ", number(at)));
            }
        }
        let coords: Vec<String> = self.coords.iter().map(|c| number(*c)).collect();
        let extend = |on: bool| match on {
            true => "true",
            false => "false",
        };
        format!(
            "<< /ShadingType {} /ColorSpace /DeviceRGB /Coords [{}] \
             /Function << /FunctionType 3 /Domain [0 1] /Functions [ {}] \
             /Bounds [ {}] /Encode [ {}] >> /Extend [{} {}] >>",
            self.kind.code(),
            coords.join(" "),
            functions,
            bounds.trim_end(),
            encode.trim_end(),
            extend(self.extend.0),
            extend(self.extend.1),
        )
    }
}

/// Where the shading's own 100 by 100 box goes: the centre of the path's
/// bounding box, the angle, and the two scales.
///
/// `pgfcoreshade.code.tex` lines 887-918.
pub fn placement(shade: &Shade, box_: (f64, f64, f64, f64)) -> ((f64, f64), f64, (f64, f64)) {
    let (min_x, min_y, max_x, max_y) = box_;
    let centre = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
    let (width, height) = (max_x - min_x, max_y - min_y);
    let (sin, cos) = shade.angle.to_radians().sin_cos();
    let (abs_sin, abs_cos) = (sin.abs(), cos.abs());
    let total = abs_cos + abs_sin;
    // Half of the shading's box is 50 of the units its ramp is declared in,
    // so the scale is the reach it has to cover over that 50.
    let scale = match total == 0.0 {
        true => (0.0, 0.0),
        false => (
            (abs_cos * width + abs_sin * height) / (50.0 * total),
            (abs_sin * width + abs_cos * height) / (50.0 * total),
        ),
    };
    (centre, shade.angle, scale)
}

const WHITE: Rgb = (1.0, 1.0, 1.0);
const BLACK: Rgb = (0.0, 0.0, 0.0);

/// xcolor's `a!p!b`: `p` percent of `a` mixed into `b`.
fn mix(a: Rgb, b: Rgb, fraction: f64) -> Rgb {
    (
        a.0 * fraction + b.0 * (1.0 - fraction),
        a.1 * fraction + b.1 * (1.0 - fraction),
        a.2 * fraction + b.2 * (1.0 - fraction),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axis_ramp_is_flat_at_both_ends() {
        // lualatex's own stitching function for `left color=red,right
        // color=blue` is four pieces with bounds at 25, 50 and 75 of 100, the
        // first and last of which have the same colour at both ends.
        let shade = Shade {
            top: (1.0, 0.0, 0.0),
            bottom: (0.0, 0.0, 1.0),
            middle: (0.5, 0.0, 0.5),
            angle: 90.0,
            ..Shade::default()
        };
        let ramp = Ramp::of(&shade, "pgfsh0");
        assert_eq!(ramp.kind, Kind::Axial);
        assert_eq!(
            ramp.stops,
            vec![
                (0.0, (0.0, 0.0, 1.0)),
                (0.25, (0.0, 0.0, 1.0)),
                (0.5, (0.5, 0.0, 0.5)),
                (0.75, (1.0, 0.0, 0.0)),
                (1.0, (1.0, 0.0, 0.0)),
            ]
        );
        let dictionary = ramp.dictionary();
        assert!(dictionary.contains("/ShadingType 2"), "{dictionary}");
        // The bounds lualatex writes, over its 0..100 domain, are a quarter,
        // a half and three quarters of the way along.
        assert!(
            dictionary.contains("/Bounds [ 0.25 0.5 0.75]"),
            "{dictionary}"
        );
        assert!(
            dictionary.contains("/C0 [0 0 1] /C1 [0 0 1]"),
            "{dictionary}"
        );
        assert!(dictionary.contains("/Extend [false false]"), "{dictionary}");
    }

    #[test]
    fn the_radial_ramp_is_two_circles_about_the_middle() {
        let shade = Shade {
            kind: Shading::Radial,
            inner: (1.0, 0.0, 0.0),
            outer: (0.0, 0.0, 1.0),
            ..Shade::default()
        };
        let ramp = Ramp::of(&shade, "pgfsh0");
        // lualatex: `/Coords [50.00064 50.00064 0.0 50.00064 50.00064
        // 50.00064]` about the centre of the box, and `/Extend [true false]`.
        assert_eq!(ramp.coords, vec![0.0, 0.0, 0.0, 0.0, 0.0, 50.0]);
        assert_eq!(ramp.extend, (true, false));
        let dictionary = ramp.dictionary();
        assert!(dictionary.contains("/ShadingType 3"), "{dictionary}");
        assert!(dictionary.contains("/Bounds [ 0.5]"), "{dictionary}");
    }

    #[test]
    fn the_placement_is_pgfs_own_two_scales() {
        // The picture lualatex was given: `\shade[left color=red,right
        // color=blue] (0,0) rectangle (2,1)`, whose box is 56.69362 by
        // 28.3468. It wrote `1 0 0 1 28.3468 14.17339 cm`, the rotation for
        // 90 degrees, and `0.567 0 0 1.134 0.0 0.0 cm`.
        let shade = Shade {
            angle: 90.0,
            ..Shade::default()
        };
        let (centre, angle, (sx, sy)) = placement(&shade, (0.0, 0.0, 56.69362, 28.3468));
        assert!((centre.0 - 28.3468).abs() < 1e-4, "{centre:?}");
        assert!((centre.1 - 14.17339).abs() < 1e-4, "{centre:?}");
        assert_eq!(angle, 90.0);
        assert!((sx - 0.56694).abs() < 1e-3, "{sx}");
        assert!((sy - 1.13387).abs() < 1e-3, "{sy}");
        // At no angle at all the box goes the other way round, which is what
        // a `top color` shading does.
        let (_, _, (sx, sy)) = placement(&Shade::default(), (0.0, 0.0, 56.69362, 28.3468));
        assert!((sx - 1.13387).abs() < 1e-3, "{sx}");
        assert!((sy - 0.56694).abs() < 1e-3, "{sy}");
    }

    #[test]
    fn a_balls_highlight_is_up_and_to_the_left() {
        let shade = Shade {
            kind: Shading::Ball,
            ball: (0.0, 0.0, 1.0),
            ..Shade::default()
        };
        let ramp = Ramp::of(&shade, "pgfsh0");
        // `\pgfdeclareradialshading[tikz@ball]{ball}{\pgfqpoint{-10bp}{10bp}}`
        assert_eq!(ramp.coords[0], -10.0);
        assert_eq!(ramp.coords[1], 10.0);
        // `color(0bp)=(tikz@ball!15!white)` -- mostly white at the highlight.
        assert_eq!(ramp.stops[0].1, (0.85, 0.85, 1.0));
        // and black at the rim, `color(50bp)=(black)`.
        assert_eq!(ramp.stops.last().unwrap().1, (0.0, 0.0, 0.0));
    }
}
