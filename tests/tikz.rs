//! What a TikZ picture comes out of this engine as.
//!
//! TikZ is a language of its own -- coordinates, transforms, nodes, curves,
//! arrows, decorations. This reads the part of it that reaches the page and
//! drops what it does not recognise, because a wrong line in a logo is worse
//! than a missing one.
//!
//! These tests assert the PDF OPERATORS, not that something was drawn. The
//! numbers come from what lualatex writes for the same picture, and the
//! comments say which picture and which bytes -- a curve that comes out at the
//! right endpoints and the wrong curvature passes a test that only counts
//! segments, and is still the wrong shape on the page.

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

// The rest of this file pins the OPERATORS a picture comes out as, not that
// something was drawn. Every expected number was read off what lualatex writes
// for the same picture, converted from PDF big points to TeX points -- lualatex
// puts one centimetre at 56.69362 big points where these numbers say 56.91
// points, and 56.69362 * 72.27/72 is 56.906. The command that produced them:
//
//     lualatex probe.tex && python3 -c 'import zlib,re,sys;
//       d=open("probe.pdf","rb").read();
//       print(zlib.decompress(d[re.search(rb"stream\r?\n",d).end():
//                              d.find(b"endstream")]).decode())'

/// The operators for one command, with the whitespace normalised away.
fn emitted(source: &str) -> String {
    let pic = parse("", source);
    to_pdf_ops(&pic, 0.0, 0.0)
}

#[test]
fn a_rectangle_is_the_four_corners_lualatex_writes() {
    // lualatex, `\draw (0,0) rectangle (2,1)`:
    //   0.0 0.0 m  0.0 28.3468 l  56.69362 28.3468 l  56.69362 0.0 l  h  S
    // Up the left side, across the top, down the right, and close -- not the
    // `re` operator, which PGF never emits for a TikZ rectangle.
    let ops = emitted(r"\draw (0,0) rectangle (2cm,1cm);");
    assert!(ops.contains("0.00 0.00 m\n"), "{ops}");
    assert!(ops.contains("0.00 28.45 l\n"), "{ops}");
    assert!(ops.contains("56.91 28.45 l\n"), "{ops}");
    assert!(ops.contains("56.91 0.00 l\n"), "{ops}");
    assert!(ops.contains("\ns\n"), "closed and stroked: {ops}");
}

#[test]
fn a_circle_is_four_cubics_at_pgfs_own_constant() {
    // lualatex, `\draw (0,0) circle [radius=1cm]`, writes the east point and
    // four `c` operators whose controls are at 0.55228475 of the radius:
    //   28.3468 0.0 m  28.3468 15.6557 15.6557 28.3468 0.0 28.3468 c ...
    // 15.6557 big points is 15.71 points, which is 0.55228475 * 28.45.
    let ops = emitted(r"\draw (0,0) circle [radius=1cm];");
    assert!(ops.contains("28.45 0.00 m\n"), "starts due east: {ops}");
    assert!(
        ops.contains("28.45 15.71 15.71 28.45 0.00 28.45 c\n"),
        "the first quarter: {ops}"
    );
    assert_eq!(ops.matches(" c\n").count(), 4, "four quarters: {ops}");
    // A radius given for one axis only still draws a circle, because `circle`
    // is `ellipse` with both radii the same.
    let round = emitted(r"\draw (0,0) circle [x radius=1cm];");
    assert!(
        round.contains("28.45 15.71 15.71 28.45 0.00 28.45 c\n"),
        "{round}"
    );
}

#[test]
fn an_arc_starts_at_the_current_point_and_not_at_the_centre() {
    // lualatex, `\draw (2,0) arc [start angle=0, end angle=90, radius=2cm]`:
    //   56.69362 0.0 m
    //   56.69362 31.31142 31.3114 56.69363 0.0 56.69363 c
    // The arc's centre is the ORIGIN -- the current point is where the arc
    // begins, on the circle. Reading it as the centre puts the whole quarter
    // two centimetres to the right of where the document drew it.
    let ops = emitted(r"\draw (2cm,0) arc [start angle=0, end angle=90, radius=2cm];");
    assert!(ops.contains("56.91 0.00 m\n"), "{ops}");
    assert!(
        ops.contains("56.91 31.43 31.43 56.91 0.00 56.91 c\n"),
        "one cubic ending at the top of the circle: {ops}"
    );
    assert_eq!(
        ops.matches(" c\n").count(),
        1,
        "90 degrees is one piece: {ops}"
    );
}

#[test]
fn a_wide_arc_is_cut_into_the_pieces_pgf_cuts_it_into() {
    // `\pgfpatharc` splits a sweep of more than 90 degrees, by 90 when the
    // whole sweep is over 115 degrees and by 60 when it is not
    // (pgfcorepathconstruct.code.tex lines 307-314). A single cubic over 180
    // degrees is not a semicircle -- it bulges out by several per cent.
    let half = emitted(r"\draw (1cm,0) arc [start angle=0, end angle=180, radius=1cm];");
    assert_eq!(half.matches(" c\n").count(), 2, "two 90s: {half}");
    let wide = emitted(r"\draw (1cm,0) arc [start angle=0, end angle=100, radius=1cm];");
    assert_eq!(wide.matches(" c\n").count(), 2, "60 then 40: {wide}");
    // `delta angle` says the same thing the other way round.
    let delta = emitted(r"\draw (1cm,0) arc [start angle=0, delta angle=90, radius=1cm];");
    assert_eq!(delta.matches(" c\n").count(), 1, "{delta}");
}

#[test]
fn a_curve_keeps_both_of_its_control_points() {
    // lualatex, `\draw (0,0) .. controls (1,1) and (2,-1) .. (3,0)`:
    //   0.0 0.0 m  28.3468 28.3468 56.69362 -28.3468 85.04042 0.0 c
    // Emitting the endpoints as a straight line -- which is what this module
    // used to do -- draws a chord where the document drew an S.
    let ops = emitted(r"\draw (0,0) .. controls (1,1) and (2,-1) .. (3,0);");
    assert!(ops.contains("1.00 1.00 2.00 -1.00 3.00 0.00 c\n"), "{ops}");
    assert!(
        !ops.contains(" l\n"),
        "no straight line was invented: {ops}"
    );
}

#[test]
fn a_parabola_uses_pgfs_two_control_fractions() {
    // lualatex, `\draw (0,0) parabola bend (1,1) (2,0)`:
    //   3.18909 6.37819 14.17339 28.3468 28.3468 28.3468 c
    //   42.5202 28.3468 53.5045 6.3782 56.69362 0.0 c
    // 3.18909/28.3468 is .1125 and 6.37819/28.3468 is .225 -- the fractions
    // `\pgfpathparabola` calls "found by trial and error".
    let ops = emitted(r"\draw (0,0) parabola bend (1,1) (2,0);");
    assert!(
        ops.contains("0.11 0.23 0.50 1.00 1.00 1.00 c\n"),
        "up: {ops}"
    );
    assert!(
        ops.contains("1.50 1.00 1.89 0.22 2.00 0.00 c\n"),
        "down: {ops}"
    );
}

#[test]
fn the_two_right_angle_connectors_go_the_two_ways_round() {
    // lualatex writes `-|` as `56.69362 0.0 l  56.69362 56.69362 l` and `|-`
    // as `0.0 56.69362 l  56.69362 56.69362 l`: across then up, or up then
    // across. Getting them the wrong way round draws the other corner.
    let horizontal = emitted(r"\draw (0,0) -| (2,2);");
    assert!(
        horizontal.contains("2.00 0.00 l\n2.00 2.00 l\n"),
        "{horizontal}"
    );
    let vertical = emitted(r"\draw (0,0) |- (2,2);");
    assert!(
        vertical.contains("0.00 2.00 l\n2.00 2.00 l\n"),
        "{vertical}"
    );
}

#[test]
fn a_grid_is_the_lines_pgf_draws_in_the_order_it_draws_them() {
    // `\pgf@pathgrid` writes every horizontal line and then every vertical
    // one, and all of them are one path object with one `S`.
    let ops = emitted(r"\draw (0,0) grid (2,2);");
    assert!(ops.contains("0.00 0.00 m\n2.00 0.00 l\n"), "y=0: {ops}");
    assert!(ops.contains("0.00 2.00 m\n2.00 2.00 l\n"), "y=2: {ops}");
    assert!(ops.contains("1.00 0.00 m\n1.00 2.00 l\n"), "x=1: {ops}");
    assert_eq!(
        ops.matches("\nS\n").count(),
        1,
        "one stroke for all six: {ops}"
    );
    // A step of a half doubles the count on each axis.
    let fine = emitted(r"\draw (0,0) grid [step=0.5] (1,1);");
    assert_eq!(fine.matches(" m\n").count(), 6, "three each way: {fine}");
}

#[test]
fn each_action_ends_with_its_own_painting_operator() {
    // PDF 32000-1 Table 60. `\fill` is `f`, `\filldraw` is `B`, `\clip` is
    // `W` then `n`, and a bare `\path` paints nothing at all.
    assert!(emitted(r"\fill (0,0) -- (1,0) -- (1,1) -- cycle;").contains("\nh\nf\n"));
    assert!(emitted(r"\filldraw (0,0) rectangle (1,1);").contains("\nh\nB\n"));
    assert!(emitted(r"\clip (0,0) rectangle (1,1);").contains("\nh\nW\nn\n"));
    assert!(emitted(r"\path (0,0) -- (1,1);").contains("\nn\n"));
    // The even-odd rule is a different operator and not a different path.
    assert!(emitted(r"\fill[even odd rule] (0,0) rectangle (1,1);").contains("\nf*\n"));
    assert!(emitted(r"\filldraw[even odd rule] (0,0) rectangle (1,1);").contains("\nB*\n"));
    // `\path[draw]` is what `\draw` abbreviates, so it strokes.
    assert!(emitted(r"\path[draw] (0,0) -- (1,1);").contains("\nS\n"));
}

#[test]
fn a_clip_is_not_wrapped_in_a_save_and_restore() {
    // A clipping path inside its own `q ... Q` stops clipping at the `Q`,
    // which is to say it does nothing. It has to outlive its command and be
    // released with the picture.
    let ops = emitted(r"\clip (0,0) rectangle (1,1); \draw (0,0) -- (2,2);");
    let clip = ops.find("W\nn\n").expect("a clip: {ops}");
    let restore = ops.rfind("Q\n").expect("the picture closes");
    assert!(
        clip < restore,
        "the clip is inside the picture's own q/Q: {ops}"
    );
    assert!(
        !ops[..clip].ends_with("q\n"),
        "and not inside one of its own: {ops}"
    );
}

#[test]
fn two_subpaths_of_one_command_are_painted_once() {
    // lualatex writes a two-subpath `\draw` as two runs of `m`/`l` and a
    // single `S`. Painting each separately is a different picture as soon as
    // the even-odd rule or a fill is involved -- a donut becomes two discs.
    let ops = emitted(r"\fill (0,0) rectangle (3,3) (1,1) rectangle (2,2);");
    assert_eq!(ops.matches(" m\n").count(), 2, "two subpaths: {ops}");
    assert_eq!(ops.matches("\nf\n").count(), 1, "one fill: {ops}");
}

#[test]
fn a_colour_reaches_the_stroke_and_the_fill_separately() {
    // `\filldraw[fill=blue,draw=green]` is `0 0 1 rg` and `0 1 0 RG`, which is
    // what lualatex writes -- two operators, not one.
    let ops = emitted(r"\filldraw[fill=blue,draw=green] (0,0) rectangle (1,1);");
    assert!(ops.contains("0 0 1 rg\n"), "{ops}");
    assert!(ops.contains("0 1 0 RG\n"), "{ops}");
    // A bare colour name sets both, which is how `\draw[red]` comes out.
    let red = emitted(r"\draw[red] (0,0) -- (1,1);");
    assert!(red.contains("1 0 0 RG\n"), "{red}");
}

#[test]
fn a_dash_pattern_reaches_the_d_operator() {
    // lualatex writes `dashed` as `[ 2.98883 2.98883 ] 0.0 d`, which is 3pt
    // on and 3pt off in big points.
    assert!(emitted(r"\draw[dashed] (0,0) -- (3,0);").contains("[3 3] 0 d\n"));
    assert!(emitted(r"\draw[densely dashed] (0,0) -- (3,0);").contains("[3 2] 0 d\n"));
    // `dotted` takes its `on` length from the line width in force, so the
    // order the options are written in changes the bytes.
    let thick = emitted(r"\draw[line width=1pt,dotted] (0,0) -- (3,0);");
    assert!(thick.contains("[1 2] 0 d\n"), "{thick}");
    assert!(thick.contains("1 w\n"), "{thick}");
    // A solid line writes no `d` at all, which is the state a page starts in.
    assert!(!emitted(r"\draw (0,0) -- (1,0);").contains(" d\n"));
}

#[test]
fn caps_and_joins_are_written_only_when_they_are_not_the_default() {
    // PDF starts a page at butt caps and mitre joins, so writing `0 J 0 j` on
    // every path is bytes that say nothing.
    assert!(!emitted(r"\draw (0,0) -- (1,0);").contains(" J\n"));
    assert!(emitted(r"\draw[line cap=round] (0,0) -- (1,0);").contains("1 J\n"));
    assert!(emitted(r"\draw[line join=bevel] (0,0) -- (1,0);").contains("2 j\n"));
}

#[test]
fn an_arrow_shortens_the_line_and_draws_a_tip_at_the_new_end() {
    // lualatex, `\draw[-stealth] (0,0) -- (2,0)`, stops the line at 54.7011
    // where the full length is 56.69362, and puts the tip's own path at that
    // point -- 54.7011 big points is 54.91 points.
    let ops = emitted(r"\draw[-stealth] (0,0) -- (2cm,0);");
    assert!(
        ops.contains("54.91 0.00 l\n"),
        "the line stops short: {ops}"
    );
    assert!(ops.contains("cm\n"), "the tip is placed by a matrix: {ops}");
    assert!(ops.contains("2.00000 0.00000 m\n"), "5u forward: {ops}");
    assert!(ops.contains("\nf\n"), "stealth is filled: {ops}");
    // The default `->` is a stroked tip at four fifths the line's width.
    let to = emitted(r"\draw[->] (0,0) -- (2cm,0);");
    assert!(to.contains("56.45 0.00 l\n"), "shortened by 0.458pt: {to}");
    assert!(to.contains("0.32 w\n"), "0.8 of 0.4pt: {to}");
    assert!(
        to.contains("\nS\n1 J\n1 j\n") || to.contains("1 J\n1 j\n"),
        "{to}"
    );
}

#[test]
fn an_arrow_at_the_start_is_turned_to_face_back_along_the_line() {
    // lualatex places a start tip with `-1.0 0.0 0.0 -1.0 ... cm` on a line
    // running to the right: the same tip, turned through half a circle.
    let ops = emitted(r"\draw[<-] (0,0) -- (2cm,0);");
    assert!(ops.contains("q -1 0 0 -1 "), "turned round: {ops}");
    // And the line starts past the origin by the tip's reach.
    assert!(ops.contains("0.46 0.00 m\n"), "{ops}");
    // `<->` is both, so there are two matrices and two tips.
    let both = emitted(r"\draw[<->] (0,0) -- (2cm,0);");
    assert_eq!(both.matches(" cm\n").count(), 2, "{both}");
}

#[test]
fn a_transform_is_baked_into_the_coordinates() {
    // lualatex writes `\draw[rotate=30,scale=2] (0,0) -- (2,0)` as
    // `98.19649 56.69362 l` -- the point itself, moved. It does not wrap the
    // path in a `cm`, so neither does this.
    let ops = emitted(r"\draw[rotate=30,scale=2] (0,0) -- (2,0);");
    let x = 4.0 * 30f64.to_radians().cos();
    let y = 4.0 * 30f64.to_radians().sin();
    assert!(ops.contains(&format!("{x:.2} {y:.2} l\n")), "{ops}");
    assert!(!ops.contains(" cm\n"), "no matrix was emitted: {ops}");
}

#[test]
fn a_scope_hands_its_options_down_and_a_path_overrides_them() {
    let pic = parse(
        "",
        r"\begin{scope}[red,xshift=3pt]\draw (0,0) -- (1,0);\draw[blue] (0,0) -- (0,1);\end{scope}
          \draw (0,0) -- (1,1);",
    );
    assert_eq!(pic.paths.len(), 3);
    assert_eq!(pic.paths[0].stroke, (1.0, 0.0, 0.0), "the scope's red");
    assert_eq!(pic.paths[1].stroke, (0.0, 0.0, 1.0), "the path's own blue");
    assert_eq!(pic.paths[2].stroke, (0.0, 0.0, 0.0), "outside, untouched");
    // The scope's shift moved both of its paths and neither of the others.
    assert_eq!(pic.paths[0].points[0], (3.0, 0.0));
    assert_eq!(pic.paths[2].points[0], (0.0, 0.0));
}

#[test]
fn a_loop_draws_one_path_per_value() {
    let pic = parse("", r"\foreach \x in {0,1,2} { \draw (\x,0) -- (\x,1); }");
    assert_eq!(pic.paths.len(), 3);
    assert_eq!(pic.paths[2].points, vec![(2.0, 0.0), (2.0, 1.0)]);
    // Each iteration is its own path command, so each gets its own `S`.
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    assert_eq!(ops.matches("\nS\n").count(), 3, "{ops}");
    // A range counts, and `\x/\y` takes two values from one entry.
    let pairs = parse(
        "",
        r"\foreach \a/\b in {1/2, 3/4} { \draw (\a,0) -- (\b,0); }",
    );
    assert_eq!(pairs.paths.len(), 2);
    assert_eq!(pairs.paths[1].points, vec![(3.0, 0.0), (4.0, 0.0)]);
}

#[test]
fn relative_and_named_coordinates_land_where_lualatex_puts_them() {
    // lualatex, for `(30:2cm) -- ++(1,0) -- +(0,1)` in centimetres, writes
    //   49.09825 28.3468 m  77.44505 28.3468 l  77.44505 56.69362 l
    // -- so `++` moved the point that `+` is then measured from, and `+` did
    // not. Treating them alike draws the second segment from the wrong place.
    let pic = parse("", r"\draw (30:2cm) -- ++(1cm,0) -- +(0,1cm);");
    let cm = 72.27 / 2.54;
    let points = &pic.paths[0].points;
    assert!((points[0].0 - 2.0 * cm * 30f64.to_radians().cos()).abs() < 1e-6);
    assert!(
        (points[1].0 - (points[0].0 + cm)).abs() < 1e-6,
        "{points:?}"
    );
    assert!(
        (points[2].0 - points[1].0).abs() < 1e-6,
        "+ does not move x"
    );
    assert!(
        (points[2].1 - (points[1].1 + cm)).abs() < 1e-6,
        "{points:?}"
    );
    // A named coordinate is the point it was given.
    let named = parse("", r"\coordinate (a) at (1,1); \draw (a) -- (2,2);");
    assert_eq!(named.paths[0].points, vec![(1.0, 1.0), (2.0, 2.0)]);
    // And `calc` walks between two of them.
    let mid = parse(
        "",
        r"\coordinate (a) at (0,0); \coordinate (b) at (4,2); \draw ($(a)!.5!(b)$) -- (9,9);",
    );
    assert_eq!(mid.paths[0].points[0], (2.0, 1.0));
}

#[test]
fn a_node_is_placed_by_the_anchor_its_options_name() {
    // `above` is `anchor=south` (tikz.code.tex line 1008), so the node's
    // BOTTOM sits on the coordinate and its border is drawn above it.
    let pic = parse("", r"\node[draw,above] at (0,0) {A};");
    let node = &pic.nodes[0];
    assert_eq!(node.anchor, texrs::tikz::Anchor::South);
    let (half_width, half_height) = node.half_size();
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    // The rectangle's lower edge is ON the coordinate: the node is placed by
    // its south anchor, so the whole box is above y=0.
    assert!(
        ops.contains(&format!(
            "{:.2} {:.2} {:.2} {:.2} re\n",
            -half_width,
            0.0,
            2.0 * half_width,
            2.0 * half_height
        )),
        "{ops}"
    );
    // `below` puts the same box entirely under the coordinate.
    let under = to_pdf_ops(&parse("", r"\node[draw,below] at (0,0) {A};"), 0.0, 0.0);
    assert!(
        under.contains(&format!(
            "{:.2} {:.2} {:.2} {:.2} re\n",
            -half_width,
            -2.0 * half_height,
            2.0 * half_width,
            2.0 * half_height
        )),
        "{under}"
    );
    // With no anchor named the coordinate is the node's middle.
    let centred = to_pdf_ops(&parse("", r"\node[draw] at (0,0) {A};"), 0.0, 0.0);
    assert!(
        centred.contains(&format!("{:.2} {:.2} ", -half_width, -half_height)),
        "{centred}"
    );
    // `\node` without `draw` or `fill` writes no border at all.
    let plain = to_pdf_ops(&parse("", r"\node at (0,0) {A};"), 0.0, 0.0);
    assert!(!plain.contains(" re"), "{plain}");
}

#[test]
fn a_node_border_is_the_text_box_plus_the_inner_separation() {
    // PGF's default `inner sep` is .3333em, which is 3.333pt at 10pt
    // (pgfmoduleshapes.code.tex line 888). The half-width is half the text
    // plus that, which is exactly what lualatex's `-7.0566 -6.88724 14.1132
    // 13.77448 re` is for a one-letter node.
    let pic = parse(
        "",
        r"\node[draw,inner sep=2pt,minimum size=0pt] at (0,0) {AB};",
    );
    let node = &pic.nodes[0];
    let (half_width, _) = node.half_size();
    assert_eq!(node.measured.0, 2.0 * 0.5 * 10.0, "two half-em characters");
    assert_eq!(half_width, 5.0 + 2.0, "half the text plus the inner sep");
    // `minimum size` widens a node that would come out smaller.
    let wide = parse("", r"\node[draw,minimum size=40pt] at (0,0) {A};");
    assert_eq!(wide.nodes[0].half_size().0, 20.0);
}

#[test]
fn a_node_on_a_path_sits_where_the_path_had_got_to() {
    // `\draw (0,0) -- (2,2) node[above] {C}` puts the node at (2,2), which is
    // where the line ended -- not at the origin and not at the midpoint.
    let pic = parse("", r"\draw (0,0) -- (2,2) node[above] {C};");
    assert_eq!(pic.paths.len(), 1);
    assert_eq!(pic.nodes.len(), 1);
    assert_eq!(pic.nodes[0].at, (2.0, 2.0));
    assert_eq!(pic.nodes[0].text, "C");
    // And a named node can be referred to afterwards.
    let named = parse("", r"\node (n) at (1,3) {x}; \draw (n) -- (0,0);");
    assert_eq!(named.paths[0].points[0], (1.0, 3.0));
}

#[test]
fn a_circle_node_takes_pgfs_radius_and_not_the_half_width() {
    // The circle shape's radius is the LENGTH of the vector made of the
    // half-width and half-height (pgfmoduleshapes.code.tex lines 1198-1235),
    // so a wide node gets a circle that encloses it rather than one that cuts
    // its corners off.
    let pic = parse("", r"\node[draw,circle,inner sep=3pt] at (0,0) {AB};");
    let (half_width, half_height) = pic.nodes[0].half_size();
    let radius = (half_width * half_width + half_height * half_height).sqrt();
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    assert!(radius > half_width, "the circle encloses the box");
    assert!(ops.contains(&format!("{radius:.2} 0.00 m\n")), "{ops}");
    assert_eq!(ops.matches(" c\n").count(), 4, "four quarters: {ops}");
}

#[test]
fn opacity_names_a_graphics_state_the_page_has_to_carry() {
    // Constant alpha is not an operator: `/pgf@ca0.5 gs` looks up a dictionary
    // in the page's `/ExtGState`, so a picture that uses it has to say which
    // dictionaries it needs or the name resolves to nothing.
    let pic = parse("", r"\draw[opacity=0.5] (0,0) -- (1,1);");
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    assert!(ops.contains("/pgf@CA0.5 gs\n"), "{ops}");
    assert!(ops.contains("/pgf@ca0.5 gs\n"), "{ops}");
    let states = pic.ext_gstates();
    assert!(
        states.contains(&("pgf@CA0.5".to_string(), "CA", 0.5)),
        "{states:?}"
    );
    assert!(
        states.contains(&("pgf@ca0.5".to_string(), "ca", 0.5)),
        "{states:?}"
    );
    // A picture that never sets opacity needs no dictionaries at all.
    assert!(parse("", r"\draw (0,0) -- (1,1);").ext_gstates().is_empty());
}

#[test]
fn what_it_still_cannot_read_it_still_leaves_out() {
    // A shading needs a shading dictionary that nothing here writes. The path
    // is read so that a node hanging off it is still placed, and the path
    // object is ended with `n` -- drawn nowhere rather than drawn wrong.
    let ops = emitted(r"\shade[left color=red,right color=blue] (0,0) rectangle (1,1);");
    assert!(ops.contains("\nn\n"), "{ops}");
    assert!(!ops.contains("\nf\n"), "no fill was guessed at: {ops}");
    // A decoration is not read, and what it decorates comes out plain.
    let plain = emitted(r"\draw[decorate,decoration={snake}] (0,0) -- (1,0);");
    assert!(plain.contains("0.00 0.00 m\n1.00 0.00 l\n"), "{plain}");
}

#[test]
fn a_comment_does_not_draw() {
    // A `%` takes the rest of its line with it, so a commented-out `\draw` is
    // not a path.
    let pic = parse("", "% \\draw (0,0) -- (9,9);\n\\draw (0,0) -- (1,1);");
    assert_eq!(pic.paths.len(), 1);
    assert_eq!(pic.paths[0].points, vec![(0.0, 0.0), (1.0, 1.0)]);
}

#[test]
fn node_text_is_drawn_by_the_same_call_every_other_glyph_is() {
    // A node's text is not a path: it goes through `Page::text_in`, so it
    // lands in the file as a `BT ... Tj ... ET` block in a real font and comes
    // back out of a text extractor. Drawing it as outlines would put a picture
    // of the word on the page and nothing a reader could search for.
    let pic = parse("", r"\node[draw] at (10,20) {Hi};");
    let mut page = texrs::pdf::Page::letter();
    texrs::tikz::draw_on(
        &pic,
        &mut page,
        100.0,
        200.0,
        texrs::pdf::Font::Base14("Helvetica".to_string()),
    );
    let node = &pic.nodes[0];
    // The text sits centred on the node, which is centred on the coordinate:
    // half its own width to the left, and its baseline below the middle.
    let x = 110.0 - node.measured.0 / 2.0;
    let y = 220.0 - node.baseline_drop();
    assert!(
        page.content
            .contains(&format!("1 0 0 1 {x} {y} Tm (Hi) Tj")),
        "{}",
        page.content
    );
    assert_eq!(page.fonts.len(), 1, "one font, named once");
    // And the border is in the same content stream, before the text.
    let border = page.content.find(" re").expect("a border");
    let text = page.content.find("BT ").expect("the text");
    assert!(border < text, "{}", page.content);
}

#[test]
fn to_bends_when_its_options_bend_it_and_not_otherwise() {
    // lualatex, `\draw (0,0) to[out=90,in=180] (2,2)` in centimetres:
    //   0.0 0.0 m  0.0 31.39217 25.30144 56.69362 56.69362 56.69362 c
    // The controls stand off each end by 0.3915 of the distance between them
    // (tikzlibrarytopaths.code.tex lines 203-230): the points are 80.176
    // points apart, and 0.3915 of that is 31.39.
    let cm = 72.27 / 2.54;
    let ops = emitted(r"\draw (0,0) to[out=90,in=180] (2cm,2cm);");
    let reach = 0.3915 * (2.0 * cm) * 2f64.sqrt();
    assert!(
        ops.contains(&format!(
            "0.00 {reach:.2} {:.2} {:.2} {:.2} {:.2} c\n",
            2.0 * cm - reach,
            2.0 * cm,
            2.0 * cm,
            2.0 * cm
        )),
        "{ops}"
    );
    // A bare `to` is a straight line: `to path/.initial` is `-- (target)`.
    let straight = emitted(r"\draw (0,0) to (3,0);");
    assert!(
        straight.contains("0.00 0.00 m\n3.00 0.00 l\n"),
        "{straight}"
    );
    assert!(!straight.contains(" c\n"), "no curve invented: {straight}");
    // `bend left` measures its angle from the line, so a bend on a vertical
    // line is not the same curve as the same bend on a horizontal one.
    let across = emitted(r"\draw (0,0) to[bend left=30] (4,0);");
    let up = emitted(r"\draw (0,0) to[bend left=30] (0,4);");
    assert_ne!(across, up, "the angle is relative to the line");
}
