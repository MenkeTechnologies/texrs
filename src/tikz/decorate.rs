//! Decorations: a path replaced by a wigglier path along the same line (§24).
//!
//! PGF's decorations are state machines. `\pgfdeclaredecoration` gives a list
//! of states, each with a WIDTH it advances the moving frame by and a body
//! that draws in that frame -- x along the path, y across it -- and each with
//! rules for switching state when the distance left runs short. The result
//! replaces the path, which is why a decorated `\draw` strokes the wiggle and
//! not the line under it.
//!
//! The four here are transcribed from the two libraries, state by state:
//!
//! - `snake` -- `pgflibrarydecorations.pathmorphing.code.tex` lines 162-207,
//!   whose middle is `\pgfpathcosine` and `\pgfpathsine`, themselves the
//!   cubics on `pgfcorepathconstruct.code.tex` lines 1331-1368 with the
//!   control fractions .3260/.5120 and .3620/.6740/.4880 written out;
//! - `zigzag` -- lines 31-55 of the same file;
//! - `saw` -- lines 64-75;
//! - `brace` -- `pgflibrarydecorations.pathreplacing.code.tex` lines 140-185.
//!
//! The lengths are `\pgfdecorationsegmentlength` (10pt) and
//! `\pgfdecorationsegmentamplitude` (2.5pt), `pgfmoduledecorations.code.tex`
//! lines 41-42, and `aspect` (0.5) on line 44.
//!
//! What is NOT here: PGF runs one state machine along the WHOLE path, so a
//! decorated polyline's wiggle carries on round its corners. This decorates
//! each straight segment on its own, which is the same path for the single
//! segment a decoration is usually put on and restarts the pattern at a
//! corner where PGF would not. A segment that is a curve is left alone rather
//! than straightened, because a decoration laid along a chord is not the
//! decoration the document asked for.

use super::options::Decoration;
use super::path::Sub;
use super::{Point, Segment};

/// The path a decoration draws in place of `sub`.
pub fn apply(
    sub: &Sub,
    decoration: Decoration,
    segment_length: f64,
    amplitude: f64,
    aspect: f64,
) -> Sub {
    if segment_length <= 0.0 {
        return sub.clone();
    }
    let mut out = Sub {
        start: sub.start,
        segments: Vec::new(),
        // A decorated path is a new path and does not inherit the `cycle`:
        // the wiggle's own last point is where it ends.
        closed: false,
    };
    let mut from = sub.start;
    for segment in &sub.segments {
        match segment {
            Segment::Line(to) => {
                let run = Run::new(from, *to);
                match run.length > 0.0 {
                    true => run.draw(decoration, segment_length, amplitude, aspect, &mut out),
                    false => out.segments.push(Segment::Line(*to)),
                }
                from = *to;
            }
            // A curve keeps its own shape. Decorating it would need the arc
            // length of a cubic, which PGF gets by flattening; until that is
            // here, the curve the document wrote is the honest answer.
            other => {
                out.segments.push(*other);
                from = match other {
                    Segment::Curve(_, _, to) => *to,
                    Segment::Line(to) => *to,
                };
            }
        }
    }
    // A decoration that drew nothing at all -- a brace with no room, say --
    // leaves the segment it was given rather than an empty path.
    match out.segments.is_empty() {
        true => sub.clone(),
        false => out,
    }
}

/// One straight run of the path, and the moving frame along it.
struct Run {
    at: Point,
    to: Point,
    /// The unit vector along the run and the one across it.
    along: Point,
    across: Point,
    length: f64,
}

impl Run {
    fn new(from: Point, to: Point) -> Run {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length = dx.hypot(dy);
        let along = match length == 0.0 {
            true => (1.0, 0.0),
            false => (dx / length, dy / length),
        };
        Run {
            at: from,
            to,
            along,
            across: (-along.1, along.0),
            length,
        }
    }

    /// A point in the moving frame at `base` along the run: `x` further along
    /// and `y` across.
    fn point(&self, base: f64, x: f64, y: f64) -> Point {
        (
            self.at.0 + (base + x) * self.along.0 + y * self.across.0,
            self.at.1 + (base + x) * self.along.1 + y * self.across.1,
        )
    }

    fn draw(
        &self,
        decoration: Decoration,
        length: f64,
        amplitude: f64,
        aspect: f64,
        out: &mut Sub,
    ) {
        match decoration {
            Decoration::Snake => self.snake(length, amplitude, out),
            Decoration::Zigzag => self.zigzag(length, amplitude, out),
            Decoration::Saw => self.saw(length, amplitude, out),
            Decoration::Brace => self.brace(amplitude, aspect, out),
        }
    }

    /// `snake`, lines 162-207: a rise, a run of full waves, a fall.
    fn snake(&self, length: f64, amplitude: f64, out: &mut Sub) {
        let mut at = 0.0;
        // The `initial` state, which switches straight to `final` if there is
        // not .625 of a segment to work in (line 164).
        if self.length - at < 0.625 * length {
            out.segments.push(Segment::Line(self.to));
            return;
        }
        out.segments.push(Segment::Curve(
            self.point(at, 0.125 * length, 0.0),
            self.point(at, 0.1875 * length, amplitude),
            self.point(at, 0.3125 * length, amplitude),
        ));
        at += 0.3125 * length;
        // `down` and `up`, each half a segment of cosine and sine, until
        // there is less than .8125 of one left (lines 173-186).
        let mut down = true;
        while self.length - at >= 0.8125 * length {
            let (high, low) = match down {
                true => (amplitude, -amplitude),
                false => (-amplitude, amplitude),
            };
            // `\pgfpathcosine{(.25l, -a)}` then `\pgfpathsine{(.25l, -a)}`,
            // whose cubics are `pgfcorepathconstruct` lines 1356-1368 and
            // 1331-1343: a quarter period each, meeting at the zero.
            let quarter = 0.25 * length;
            // Each of the two is a quarter period, and they meet on the line:
            // the cosine leaves the crest flat and the sine arrives at the
            // trough flat, which is what makes the join smooth.
            let middle = (high + low) / 2.0;
            out.segments.push(Segment::Curve(
                self.point(at, 0.3620 * quarter, high),
                self.point(at, 0.6740 * quarter, high + 0.4880 * (middle - high)),
                self.point(at, quarter, middle),
            ));
            out.segments.push(Segment::Curve(
                self.point(
                    at,
                    quarter + 0.3260 * quarter,
                    middle + 0.5120 * (low - middle),
                ),
                self.point(at, quarter + 0.6380 * quarter, low),
                self.point(at, 2.0 * quarter, low),
            ));
            at += 0.5 * length;
            down = !down;
        }
        // `end down` and `end up`, lines 187-202: back to the line.
        let side = match down {
            true => amplitude,
            false => -amplitude,
        };
        out.segments.push(Segment::Curve(
            self.point(at, 0.125 * length, side),
            self.point(at, 0.1875 * length, 0.0),
            self.point(at, 0.3125 * length, 0.0),
        ));
        // `final`: `\pgfpathlineto{\pgfpointdecoratedpathlast}`, line 205.
        out.segments.push(Segment::Line(self.to));
    }

    /// `zigzag`, lines 31-55: half a segment per limb, the apex at a quarter.
    fn zigzag(&self, length: f64, amplitude: f64, out: &mut Sub) {
        let half = 0.5 * length;
        let mut at = 0.0;
        if self.length < half {
            out.segments.push(Segment::Line(self.to));
            return;
        }
        // `up from center`, line 32.
        out.segments
            .push(Segment::Line(self.point(at, 0.25 * length, amplitude)));
        at += half;
        // `big down` and `big up` alternate until under half a segment is
        // left, when `center finish` puts the path back on the line.
        let mut up = false;
        while self.length - at >= half {
            let side = match up {
                true => amplitude,
                false => -amplitude,
            };
            out.segments
                .push(Segment::Line(self.point(at, 0.25 * length, side)));
            at += half;
            up = !up;
        }
        // `center finish`, line 48: `\pgfpathlineto{\pgfpointorigin}` in the
        // frame, which is the point on the line itself. It is the same point
        // as the path's end whenever the last limb fell on it, and lualatex
        // writes one `l` there rather than two.
        let finish = self.point(at, 0.0, 0.0);
        out.segments.push(Segment::Line(finish));
        if (finish.0 - self.to.0).hypot(finish.1 - self.to.1) > 1e-6 {
            out.segments.push(Segment::Line(self.to));
        }
    }

    /// `saw`, lines 64-75: a ramp up and a drop straight back down.
    fn saw(&self, length: f64, amplitude: f64, out: &mut Sub) {
        let mut at = 0.0;
        while self.length - at >= length {
            out.segments
                .push(Segment::Line(self.point(at, length, amplitude)));
            out.segments
                .push(Segment::Line(self.point(at, length, 0.0)));
            at += length;
        }
        out.segments.push(Segment::Line(self.to));
    }

    /// `brace`, `pathreplacing` lines 140-185: two shoulders and a middle
    /// spike, the whole of it across the run in one state.
    fn brace(&self, amplitude: f64, aspect: f64, out: &mut Sub) {
        let total = self.length;
        // Lines 144-149: the shoulder shortens rather than overshooting when
        // the brace is not long enough for a full one.
        let mut yc = aspect * total;
        yc = match 2.0 * amplitude > yc {
            true => 0.5 * yc,
            false => amplitude,
        };
        // Lines 150-156, where the second shoulder is measured back from the
        // far end and so starts out negative.
        let mut xc = aspect * total - total;
        xc = match -2.0 * amplitude < xc {
            true => -0.5 * xc,
            false => amplitude,
        };
        let middle = aspect * total;
        out.segments.push(Segment::Curve(
            self.point(0.0, 0.15 * yc, 0.3 * amplitude),
            self.point(0.0, 0.5 * yc, 0.5 * amplitude),
            self.point(0.0, yc, 0.5 * amplitude),
        ));
        out.segments
            .push(Segment::Line(self.point(middle, -yc, 0.5 * amplitude)));
        out.segments.push(Segment::Curve(
            self.point(middle, -0.5 * yc, 0.5 * amplitude),
            self.point(middle, -0.15 * yc, 0.7 * amplitude),
            self.point(middle, 0.0, amplitude),
        ));
        out.segments.push(Segment::Curve(
            self.point(middle, 0.15 * xc, 0.7 * amplitude),
            self.point(middle, 0.5 * xc, 0.5 * amplitude),
            self.point(middle, xc, 0.5 * amplitude),
        ));
        out.segments
            .push(Segment::Line(self.point(total, -xc, 0.5 * amplitude)));
        out.segments.push(Segment::Curve(
            self.point(total, -0.5 * xc, 0.5 * amplitude),
            self.point(total, -0.15 * xc, 0.3 * amplitude),
            self.point(total, 0.0, 0.0),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(length: f64) -> Sub {
        Sub {
            start: (0.0, 0.0),
            segments: vec![Segment::Line((length, 0.0))],
            closed: false,
        }
    }

    #[test]
    fn a_snake_rises_waves_and_falls_back_to_the_line() {
        // Forty points of line at PGF's own ten-point segment: a rise over
        // 3.125, three half-waves of five, and a fall over 3.125.
        let out = apply(&line(40.0), Decoration::Snake, 10.0, 2.5, 0.5);
        assert!(matches!(out.segments[0], Segment::Curve(..)));
        // The rise ends at the amplitude, a third of a segment along.
        if let Segment::Curve(_, _, (x, y)) = out.segments[0] {
            assert!(
                (x - 3.125).abs() < 1e-9 && (y - 2.5).abs() < 1e-9,
                "{x} {y}"
            );
        }
        // And every one of them ends back on the line.
        let last = out.segments.last().unwrap();
        assert_eq!(*last, Segment::Line((40.0, 0.0)));
        // The waves swing to both sides, which a decoration drawn as a row of
        // bumps on one side would not.
        let lowest = out
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Line((_, y)) => *y,
                Segment::Curve(_, _, (_, y)) => *y,
            })
            .fold(f64::MAX, f64::min);
        assert!((lowest + 2.5).abs() < 1e-9, "{lowest}");
    }

    #[test]
    fn a_line_too_short_for_a_snake_stays_a_line() {
        // `switch if less than=+.625\pgfdecorationsegmentlength to final`,
        // line 164: under 6.25 points there is no room for the rise.
        let out = apply(&line(4.0), Decoration::Snake, 10.0, 2.5, 0.5);
        assert_eq!(out.segments, vec![Segment::Line((4.0, 0.0))]);
    }

    #[test]
    fn a_zigzag_puts_its_apexes_a_quarter_segment_in() {
        let out = apply(&line(30.0), Decoration::Zigzag, 10.0, 2.5, 0.5);
        // `up from center` draws to (.25l, a) and advances .5l, so the next
        // apex is at .75l on the other side -- lines 32-47.
        assert_eq!(out.segments[0], Segment::Line((2.5, 2.5)));
        assert_eq!(out.segments[1], Segment::Line((7.5, -2.5)));
        assert_eq!(out.segments[2], Segment::Line((12.5, 2.5)));
        // and every limb is a straight line, not a curve.
        assert!(out
            .segments
            .iter()
            .all(|segment| matches!(segment, Segment::Line(_))));
    }

    #[test]
    fn a_saw_rises_and_drops_at_the_same_place() {
        let out = apply(&line(20.0), Decoration::Saw, 10.0, 2.5, 0.5);
        // Lines 70-71: `(l, a)` then `(l, 0)` -- the drop is vertical.
        assert_eq!(out.segments[0], Segment::Line((10.0, 2.5)));
        assert_eq!(out.segments[1], Segment::Line((10.0, 0.0)));
        assert_eq!(out.segments[2], Segment::Line((20.0, 2.5)));
    }

    #[test]
    fn a_brace_spikes_at_the_aspect_and_returns_to_the_ends() {
        let out = apply(&line(100.0), Decoration::Brace, 10.0, 2.5, 0.5);
        // The spike is at `aspect * length` along and a full amplitude out --
        // lines 163-168.
        let spike = match out.segments[2] {
            Segment::Curve(_, _, at) => at,
            other => panic!("a curve, not {other:?}"),
        };
        assert!((spike.0 - 50.0).abs() < 1e-9, "{spike:?}");
        assert!((spike.1 - 2.5).abs() < 1e-9, "{spike:?}");
        // and it comes back to the far end of the run.
        let end = match out.segments.last().unwrap() {
            Segment::Curve(_, _, at) => *at,
            other => panic!("a curve, not {other:?}"),
        };
        assert!(
            (end.0 - 100.0).abs() < 1e-9 && end.1.abs() < 1e-9,
            "{end:?}"
        );
    }

    #[test]
    fn a_decoration_turns_with_the_line_it_is_on() {
        // Straight up: the amplitude goes to the LEFT, because the frame's y
        // axis is the path's normal and not the page's.
        let up = Sub {
            start: (0.0, 0.0),
            segments: vec![Segment::Line((0.0, 30.0))],
            closed: false,
        };
        let out = apply(&up, Decoration::Zigzag, 10.0, 2.5, 0.5);
        assert_eq!(out.segments[0], Segment::Line((-2.5, 2.5)));
    }
}
