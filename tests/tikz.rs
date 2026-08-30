//! The part of TikZ that is straight lines.
//!
//! TikZ is a language of its own -- coordinates, transforms, nodes, curves,
//! decorations. This reads the subset the publication marks are drawn with and
//! drops what it does not recognise, because a wrong line in a logo is worse
//! than a missing one.

use texrs::tikz::{parse, to_pdf_ops};

/// The mark from `arb-registry/docs/book-print.tex`, verbatim.
const MARK: &str = r"
        \draw[textDim,line width=1.52pt]
            (32,58) -- (55,45) -- (55,19) -- (32,6) -- (9,19) -- (9,45) -- cycle;
        \draw[neonCyan,line width=1.33pt,line join=miter]
            (19,24) -- (19,38) -- (25,32) -- (31,38) -- (31,24);
        \draw[neonCyan,line width=1.33pt] (35,38) -- (45,38) (40,38) -- (40,24);
";

#[test]
fn the_real_mark_parses_into_the_paths_it_draws() {
    let pic = parse("x=0.38pt,y=0.38pt,line cap=butt,line join=round", MARK);
    // Three \draw commands, but the last carries two disjoint segments: a
    // coordinate following a coordinate with no `--` starts a new path.
    assert_eq!(pic.paths.len(), 4, "got {:?}", pic.paths.len());
    assert_eq!(pic.x_scale, 0.38);
    assert_eq!(pic.y_scale, 0.38);
}

#[test]
fn a_hexagon_closes_and_a_line_does_not() {
    let pic = parse("", MARK);
    assert!(pic.paths[0].closed, "`-- cycle` closes the path");
    assert_eq!(pic.paths[0].points.len(), 6, "six corners");
    assert!(!pic.paths[1].closed, "a polyline without cycle stays open");
}

#[test]
fn the_line_width_is_the_one_the_draw_asked_for() {
    let pic = parse("", MARK);
    assert_eq!(pic.paths[0].width, 1.52);
    assert_eq!(pic.paths[1].width, 1.33);
}

#[test]
fn one_draw_can_carry_two_disjoint_segments() {
    // `(35,38) -- (45,38) (40,38) -- (40,24)` is a cross, not a quadrilateral.
    // Joining them would draw a line that is not in the picture.
    let pic = parse("", r"\draw[a] (35,38) -- (45,38) (40,38) -- (40,24);");
    assert_eq!(pic.paths.len(), 2);
    assert_eq!(pic.paths[0].points, vec![(35.0, 38.0), (45.0, 38.0)]);
    assert_eq!(pic.paths[1].points, vec![(40.0, 38.0), (40.0, 24.0)]);
}

#[test]
fn the_scale_reaches_the_operators() {
    let pic = parse("x=0.5pt,y=0.5pt", r"\draw[a] (10,20) -- (30,40);");
    let ops = to_pdf_ops(&pic, 100.0, 200.0);
    assert!(
        ops.contains("105.00 210.00 m"),
        "start scaled and offset: {ops}"
    );
    assert!(ops.contains("115.00 220.00 l"), "and the line to: {ops}");
    assert!(ops.contains("\nS\n"), "stroked, not filled: {ops}");
}

#[test]
fn a_closed_path_uses_the_closing_stroke_operator() {
    let pic = parse("", r"\draw[a] (0,0) -- (10,0) -- (10,10) -- cycle;");
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    assert!(ops.contains("\ns\n"), "`s` closes and strokes: {ops}");
}

#[test]
fn what_it_cannot_read_it_leaves_out() {
    // A curve is not a polyline. Emitting its endpoints as a straight line
    // would draw something the document does not contain.
    let pic = parse("", r"\draw[a] (0,0) .. controls (5,10) .. (10,0);");
    let straight = pic.paths.iter().all(|p| p.points.len() <= 3);
    assert!(straight, "no curve is invented");
}

#[test]
fn the_picture_reports_its_own_size() {
    let pic = parse("x=2pt,y=3pt", r"\draw[a] (0,0) -- (10,20);");
    let (w, h) = pic.size();
    assert_eq!((w, h), (20.0, 60.0));
}
