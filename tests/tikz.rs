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
    // The anchor is half a line width OUTSIDE the drawn border -- `outer
    // xsep/.initial=.5\pgflinewidth`, pgfmoduleshapes lines 891-892 -- so the
    // box the coordinate carries is that much bigger than the box drawn.
    let (half_width, half_height) = node.border().drawn();
    let outer = node.outer_sep;
    assert_eq!(outer, 0.2, "half of the 0.4pt default line width");
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    // The rectangle's lower edge is a standoff ABOVE the coordinate: the node
    // is placed by its south anchor, and that anchor is outside the line.
    assert!(
        ops.contains(&format!(
            "{:.2} {:.2} {:.2} {:.2} re\n",
            -half_width,
            outer,
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
            -2.0 * half_height - outer,
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
    let (half_width, _) = node.border().drawn();
    assert_eq!(node.measured.0, 2.0 * 0.5 * 10.0, "two half-em characters");
    assert_eq!(half_width, 5.0 + 2.0, "half the text plus the inner sep");
    // and the ANCHOR box is that plus the outer separation, which is what
    // stands a line drawn to `(n.east)` off the border it points at.
    assert_eq!(node.half_size().0, 5.0 + 2.0 + 0.2);
    // `minimum size` widens a node that would come out smaller.
    let wide = parse("", r"\node[draw,minimum size=40pt] at (0,0) {A};");
    assert_eq!(wide.nodes[0].border().drawn().0, 20.0);
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
    let node = &pic.nodes[0];
    let (half_width, half_height) = node.text_half();
    let (radius, _) = node.border().drawn();
    let ops = to_pdf_ops(&pic, 0.0, 0.0);
    assert!(radius > half_width, "the circle encloses the box");
    // PGF computes that length in TeX's own dimensions, which round at every
    // step, and comes out a fraction SHORT of the square root: lualatex draws
    // a "Hi" node's circle at 10.75107 where the hypotenuse is 10.79139.
    let exact = half_width.hypot(half_height);
    assert!(radius < exact, "{radius} is PGF's answer, not {exact}");
    assert!(exact - radius < 0.01 * exact, "and within half a percent");
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
    // A pattern is a tiling pattern in the page's `/Pattern` resource, and
    // nothing here writes one. The area is left EMPTY rather than filled with
    // the fill colour, because a solid black box where the document asked for
    // hatching is a picture that is drawn and drawn wrong.
    let ops = emitted(r"\fill[pattern=north east lines] (0,0) rectangle (1,1);");
    assert!(
        !ops.contains("\nf\n"),
        "no solid fill was guessed at: {ops}"
    );
    // `matrix` and `\graph` are not read: their contents are not paths, and
    // half of one of them is not a picture.
    let matrix = parse("", r"\matrix { \node {a}; & \node {b}; \\ };");
    assert!(matrix.paths.is_empty(), "{:?}", matrix.paths);
    // A decoration nothing here knows leaves the path it was put on alone.
    let plain = emitted(r"\draw[decorate,decoration={footprints}] (0,0) -- (1,0);");
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

#[test]
fn an_anchor_is_on_the_nodes_border_and_not_at_its_middle() {
    // lualatex, for a node whose box is fixed by `minimum width=40pt,
    // minimum height=20pt, inner sep=0pt` so no font metric is in the answer:
    //   -19.92554 -9.96277 39.85107 19.92554 re      the border
    //   0.0 10.16202 m   20.12479 0.0 l              (a.north), (a.east)
    //   -20.12479 -10.16202 l   17.60118 10.16202 l  (a.south west), (a.30)
    // Every one of those is half a line width outside the drawn border, and
    // NONE of them is the centre -- which is what a name with no anchor is.
    let source = r"\node[draw,minimum width=40pt,minimum height=20pt,inner sep=0pt] (a) at (0,0) {};
                   \draw (a.north) -- (a.east) -- (a.south west) -- (a.30);";
    let pic = parse("", source);
    let points = &pic.paths[0].points;
    assert_eq!(points[0], (0.0, 10.2), "north: 10pt and the outer sep");
    assert_eq!(points[1], (20.2, 0.0), "east");
    assert_eq!(points[2], (-20.2, -10.2), "south west");
    // 30 degrees is steeper than the box's own corner, so the ray leaves by
    // the TOP: x is the half-height divided by tan 30, not the half-width.
    let (x, y) = points[3];
    assert!((y - 10.2).abs() < 1e-9, "{y}");
    assert!((x - 10.2 / 30f64.to_radians().tan()).abs() < 1e-9, "{x}");
    // And the bare name is still the centre, which is what it means.
    let centre = parse("", r"\node (a) at (3,4) {x}; \draw (a) -- (0,0);");
    assert_eq!(centre.paths[0].points[0], (3.0, 4.0));
}

#[test]
fn a_coordinates_anchors_are_the_coordinate() {
    // A `\coordinate` has no extent: `pgfcoreshapes.code.tex` gives it a
    // centre and nothing else, so `(a.north)` is the point itself. This is
    // the one case where the centre is the right answer, and it has to keep
    // working now that a node's is not.
    let pic = parse(
        "",
        r"\coordinate (a) at (2,3); \draw (a.north) -- (a.east);",
    );
    assert_eq!(pic.paths[0].points, vec![(2.0, 3.0), (2.0, 3.0)]);
}

#[test]
fn an_ellipse_node_is_root_two_bigger_than_its_text_box() {
    // `pgflibraryshapes.geometric.code.tex` lines 39-40 multiply both radii by
    // 1.4142136, so the text's corners land ON the ellipse. lualatex draws
    // the `minimum width=40pt,minimum height=20pt` ellipse from
    //   119.55322 0.0 m ... about a centre at 99.62769
    // which is 20pt and 10pt, the minimum sizes exactly.
    let ops = emitted(
        r"\node[draw,ellipse,minimum width=40pt,minimum height=20pt,inner sep=0pt] at (0,0) {};",
    );
    assert!(ops.contains("20.00 0.00 m\n"), "{ops}");
    assert_eq!(ops.matches(" c\n").count(), 4, "four quarters: {ops}");
    // With no minimum the radii are root two times the text half-box.
    let pic = parse("", r"\node[draw,ellipse,inner sep=2pt] at (0,0) {AB};");
    let (half_width, _) = pic.nodes[0].text_half();
    let (rx, _) = pic.nodes[0].border().drawn();
    assert!((rx - 1.414_213_6 * half_width).abs() < 1e-9, "{rx}");
}

#[test]
fn a_diamond_node_is_the_four_points_pgf_writes() {
    // lualatex, for `minimum width=40pt,minimum height=20pt,inner sep=0pt`:
    //   219.09837 0.0 m  199.25537 9.88023 l  179.41235 0.0 l
    //   199.25537 -9.88023 l  h
    // about a centre at 199.25537 -- east, north, west, south, closed. The
    // half-width drawn is 20pt less root two outer separations, because the
    // sloping side stands off by the outer sep and its CORNER by root two of
    // it (geometric library lines 328-340).
    let ops = emitted(
        r"\node[draw,diamond,minimum width=40pt,minimum height=20pt,inner sep=0pt] at (0,0) {};",
    );
    let half_width = 20.2 - 1.414_213 * 0.2;
    let half_height = 10.2 - 1.414_213 * 0.2;
    assert!(ops.contains(&format!("{half_width:.2} 0.00 m\n")), "{ops}");
    assert!(ops.contains(&format!("0.00 {half_height:.2} l\n")), "{ops}");
    assert!(
        ops.contains(&format!("{:.2} 0.00 l\n", -half_width)),
        "{ops}"
    );
    assert!(ops.contains("h\n"), "closed: {ops}");
    // And its east anchor is the full half-width, outer separation included:
    // lualatex draws to 219.38016, which is 199.25537 plus 20.12479.
    let pic = parse(
        "",
        r"\node[draw,diamond,minimum width=40pt,minimum height=20pt,inner sep=0pt] (d) at (0,0) {};
          \draw (d.east) -- (0,0);",
    );
    assert_eq!(pic.paths[0].points[0], (20.2, 0.0));
}

#[test]
fn rounded_corners_cut_the_corner_back_and_arc_across_it() {
    // lualatex, `\draw[rounded corners=4pt] (0,0) -- (0,20pt) -- (40pt,20pt)`:
    //   0.0 0.0 m
    //   0.0 15.94043 l
    //   0.0 18.14136 1.78416 19.92554 3.9851 19.92554 c
    //   39.85107 19.92554 l
    // The corner is cut back 4pt along each leg and the controls are
    // 0.5522847 of the way back to it -- pgfcorepathprocessing lines 371-427.
    let ops = emitted(r"\draw[rounded corners=4pt] (0,0) -- (0,20pt) -- (40pt,20pt);");
    let k = 0.5522847 * 4.0;
    assert!(ops.contains("0.00 16.00 l\n"), "cut back 4pt: {ops}");
    assert!(
        ops.contains(&format!(
            "0.00 {:.2} {:.2} 20.00 4.00 20.00 c\n",
            16.0 + k,
            4.0 - k
        )),
        "{ops}"
    );
    assert!(ops.contains("40.00 20.00 l\n"), "{ops}");
    // `sharp corners` puts it back, and the corner is a plain `l` again.
    let sharp =
        emitted(r"\draw[rounded corners=4pt,sharp corners] (0,0) -- (0,20pt) -- (40pt,20pt);");
    assert!(sharp.contains("0.00 20.00 l\n40.00 20.00 l\n"), "{sharp}");
    assert!(!sharp.contains(" c\n"), "no arc: {sharp}");
}

#[test]
fn a_snake_is_the_cubics_pgf_writes_for_one() {
    // lualatex, `\draw[decorate,decoration={snake}] (0,0) -- (40pt,0)` at the
    // default 10pt segment and 2.5pt amplitude:
    //   0.0 0.0 m
    //   1.24535 0.0 1.86801 2.49069 3.11336 2.49069 c     the rise
    //   4.01498 2.49069 4.79207 1.2752 5.60405 0.0 c      a cosine quarter
    //   6.41602 -1.2752 7.19312 -2.49069 8.09474 -2.49069 c   a sine quarter
    //   ...
    //   34.24701 2.49069 34.86969 0.0 36.11504 0.0 c      the fall
    //   39.85107 0.0 l
    // in big points; the same numbers in the picture's units are 10 and 2.5.
    let ops = emitted(r"\draw[decorate,decoration={snake}] (0,0) -- (40,0);");
    assert!(
        ops.contains("1.25 0.00 1.88 2.50 3.12 2.50 c\n"),
        "the rise: {ops}"
    );
    assert!(
        ops.contains("4.03 2.50 4.81 1.28 5.62 0.00 c\n"),
        "the cosine quarter: {ops}"
    );
    assert!(
        ops.contains("6.44 -1.28 7.22 -2.50 8.12 -2.50 c\n"),
        "the sine quarter: {ops}"
    );
    assert!(ops.contains("40.00 0.00 l\n"), "and back to the end: {ops}");
    // Without `decorate` the decoration is only named, and the line is a line.
    let named = emitted(r"\draw[decoration={snake}] (0,0) -- (40,0);");
    assert!(named.contains("0.00 0.00 m\n40.00 0.00 l\n"), "{named}");
}

#[test]
fn a_zigzag_and_a_brace_land_where_pgf_puts_them() {
    // lualatex, `decoration={zigzag}` on a 30pt line: apexes at 2.49069,
    // 7.47208, 12.45346 ... in big points, which is a quarter of a segment in
    // and then every half segment.
    let zigzag = emitted(r"\draw[decorate,decoration={zigzag}] (0,0) -- (30,0);");
    assert!(
        zigzag.contains("2.50 2.50 l\n7.50 -2.50 l\n12.50 2.50 l\n"),
        "{zigzag}"
    );
    assert!(!zigzag.contains(" c\n"), "straight limbs: {zigzag}");
    // and `decoration={brace}` on a 100pt line, whose shoulder curve lualatex
    // writes as `0.37358 0.74721 1.24535 1.24535 2.49069 1.24535 c` and whose
    // spike is at the aspect, `49.81384 2.49069`.
    let brace = emitted(r"\draw[decorate,decoration={brace}] (0,0) -- (100,0);");
    assert!(
        brace.contains("0.38 0.75 1.25 1.25 2.50 1.25 c\n"),
        "{brace}"
    );
    assert!(brace.contains("47.50 1.25 l\n"), "the shoulder: {brace}");
    assert!(
        brace.contains("48.75 1.25 49.62 1.75 50.00 2.50 c\n"),
        "the spike: {brace}"
    );
    assert!(
        brace.contains("98.75 1.25 99.62 0.75 100.00 0.00 c\n"),
        "{brace}"
    );
}

#[test]
fn a_shading_is_a_clip_a_matrix_and_the_sh_operator() {
    // lualatex, `\shade[left color=red,right color=blue] (0,0) rectangle (2,1)`
    // in centimetres, clips to the path and then writes
    //   1 0 0 1 28.3468 14.17339 cm      the box's centre
    //   0.0 1.0 -1.0 0.0 0.0 0.0 cm      `left color` turns it 90 degrees
    //   0.567 0 0 1.134 0.0 0.0 cm       the two scales
    // before painting the shading. It puts the shading in a form XObject and
    // this names it straight out of the page's `/Shading`, which is the same
    // paint through the same clip.
    let ops = emitted(r"\shade[left color=red,right color=blue] (0,0) rectangle (100,50);");
    assert!(ops.contains("\nW\nn\n"), "clipped to the path: {ops}");
    assert!(ops.contains("1 0 0 1 50 25 cm\n"), "the centre: {ops}");
    assert!(ops.contains("0 1 -1 0 0 0 cm\n"), "turned by 90: {ops}");
    assert!(
        ops.contains("1 0 0 2 0 0 cm\n"),
        "50 over 50 and 100 over 50: {ops}"
    );
    assert!(ops.contains("/pgfsh0 sh\n"), "{ops}");
    // A `\shade` paints nothing else: no fill colour was guessed at.
    assert!(!ops.contains("\nf\n"), "{ops}");
}

#[test]
fn a_shading_says_which_dictionary_the_page_has_to_carry() {
    // `sh` looks its shading up BY NAME in the page's `/Shading` resource
    // (PDF 32000-1 S8.7.4.5), so a page carrying a shaded picture has to
    // carry the dictionary too -- the same contract `/ExtGState` has for
    // opacity. The ramp is TikZ's own, tikz.code.tex lines 628-633: flat for
    // the first quarter, flat for the last, and changing over the middle.
    let pic = parse(
        "",
        r"\shade[left color=red,right color=blue] (0,0) rectangle (2,1);",
    );
    let shadings = pic.shadings();
    assert_eq!(shadings.len(), 1);
    assert_eq!(shadings[0].name, "pgfsh0");
    let dictionary = shadings[0].dictionary();
    assert!(dictionary.contains("/ShadingType 2"), "{dictionary}");
    assert!(dictionary.contains("/Coords [0 -50 0 50]"), "{dictionary}");
    assert!(
        dictionary.contains("/Bounds [ 0.25 0.5 0.75]"),
        "{dictionary}"
    );
    // The two ends of the ramp, as lualatex's own four functions have them.
    assert!(
        dictionary.contains("/C0 [0 0 1] /C1 [0 0 1]"),
        "{dictionary}"
    );
    assert!(
        dictionary.contains("/C0 [1 0 0] /C1 [1 0 0]"),
        "{dictionary}"
    );
    // A picture that shades nothing needs no dictionaries at all.
    assert!(parse("", r"\draw (0,0) -- (1,1);").shadings().is_empty());
    // `inner color`/`outer color` is the other type, and it extends inward so
    // that the middle of the circle is painted at all.
    let radial = parse("", r"\shade[inner color=red] (0,0) circle[radius=1];");
    let dictionary = radial.shadings()[0].dictionary();
    assert!(dictionary.contains("/ShadingType 3"), "{dictionary}");
    assert!(dictionary.contains("/Extend [true false]"), "{dictionary}");
}

#[test]
fn shadedraw_strokes_the_border_over_the_shading() {
    // `\shadedraw` is `\shade` with `\tikz@mode@drawtrue`: the shading is
    // painted through the path as a clip and then the path is stroked.
    let ops = emitted(r"\shadedraw[top color=red] (0,0) rectangle (100,50);");
    assert!(ops.contains("/pgfsh0 sh\n"), "{ops}");
    assert!(ops.contains("\nS\n"), "and stroked: {ops}");
    // `top color` shades along the picture's y axis, so it is NOT turned.
    assert!(ops.contains("1 0 0 1 0 0 cm\n"), "{ops}");
}

#[test]
fn a_coordinate_may_be_arithmetic() {
    // lualatex, `\draw (2*3,{sqrt(9)}) -- ({sin(30)},1) -- ({2*1cm},0)` in
    // centimetres: `170.08086 85.04042 m  14.17339 28.3468 l  56.69362 0.0 l`,
    // which is (6,3), (0.5,1) and (2cm,0). PGF hands every component to
    // `\pgfmathparse`, and reading only the literal numbers draws none of it.
    let pic = parse(
        "",
        r"\draw (2*3,{sqrt(9)}) -- ({sin(30)},1) -- ({2*1cm},0);",
    );
    let points = &pic.paths[0].points;
    assert_eq!(points[0], (6.0, 3.0));
    assert!((points[1].0 - 0.5).abs() < 1e-12, "{:?}", points[1]);
    assert_eq!(points[1].1, 1.0);
    // A unit multiplies through: `2*1cm` is two centimetres and not two of
    // the picture's own units.
    assert!(
        (points[2].0 - 2.0 * 72.27 / 2.54).abs() < 1e-9,
        "{:?}",
        points[2]
    );
    // An angle may be arithmetic too, which is what a `\foreach` writes.
    let polar = parse("", r"\draw (0,0) -- (2*15:2);");
    let (x, y) = polar.paths[0].points[1];
    assert!((x - 2.0 * 30f64.to_radians().cos()).abs() < 1e-12, "{x}");
    assert!((y - 2.0 * 30f64.to_radians().sin()).abs() < 1e-12, "{y}");
    // and an expression nothing can read leaves the coordinate unread rather
    // than putting the point at the origin.
    let unread = parse("", r"\draw (0,0) -- (nosuchfunction(2),1);");
    assert!(unread.paths.is_empty(), "{:?}", unread.paths);
}

#[test]
fn a_label_is_a_second_node_outside_the_border() {
    // `label=above:L` puts a node of its own on the labelled node's north
    // anchor, anchored by its own south, so the two do not overlap (S17.10.1).
    let pic = parse(
        "",
        r"\node[draw,label=above:L,minimum size=20pt,inner sep=0pt] at (0,0) {};",
    );
    assert_eq!(pic.nodes.len(), 2, "the node and its label");
    let label = pic
        .nodes
        .iter()
        .find(|node| node.text == "L")
        .expect("a label");
    let node = pic
        .nodes
        .iter()
        .find(|node| node.text.is_empty())
        .expect("the node");
    // The label sits entirely above the labelled node's own north anchor.
    let (_, half_height) = label.border().half;
    let (_, node_half) = node.border().half;
    assert!(
        label.at.1 - half_height >= node_half - 1e-9,
        "{:?}",
        label.at
    );
    // and it is not drawn: a label has no border of its own.
    assert!(!label.draw && !label.filled);
    // `label=below:` puts it on the other side.
    let below = parse(
        "",
        r"\node[draw,label=below:L,minimum size=20pt,inner sep=0pt] at (0,0) {};",
    );
    let under = below
        .nodes
        .iter()
        .find(|node| node.text == "L")
        .expect("a label");
    assert!(under.at.1 < 0.0, "{:?}", under.at);
}

#[test]
fn a_shaded_picture_puts_its_ramp_on_the_page_that_names_it() {
    // `sh` looks its ramp up by NAME in the page's `/Shading` resource, so the
    // operator and the resource have to arrive together: a name that resolves
    // to nothing is not "no shading" to a reader, it is a stream it has to
    // guess at. This pinned the absence of both while `pdf::Page` had no way to
    // carry a shading; it now pins the presence of both.
    let pic = parse("", r"\shade[top color=red] (0,0) rectangle (10,10);");
    let mut page = texrs::pdf::Page::letter();
    texrs::tikz::draw_on(
        &pic,
        &mut page,
        0.0,
        0.0,
        texrs::pdf::Font::Base14("Helvetica".to_string()),
    );
    assert!(page.content.contains("/pgfsh0 sh\n"), "{}", page.content);
    // And the page carries the dictionary that name resolves through.
    let (name, dictionary) = page.shadings.first().expect("a shading on the page");
    assert_eq!(name, "pgfsh0");
    assert!(dictionary.contains("/ShadingType"), "{dictionary}");
    assert_eq!(pic.shadings().len(), 1);
}

#[test]
fn an_anchor_does_not_shrink_with_the_pictures_scale() {
    // A node is sized by its TEXT, and `x=`/`y=` does not scale text: the
    // border of a `minimum size=20pt` node is 20pt across whatever the
    // picture's units are. So the anchor offset is in points and the node's
    // place is in picture units, and adding one to the other without dividing
    // puts the anchor in a different spot in every scaled picture.
    let source = r"\node[draw,minimum width=40pt,minimum height=20pt,inner sep=0pt] (a) at (1,0) {};
                   \draw (a.east) -- (0,0);";
    let plain = parse("", source);
    let half = parse("x=0.5pt,y=0.5pt", source);
    // At full scale the anchor is 20.2 points right of the centre; at half
    // scale it is the same 20.2 POINTS, which is 40.4 of the picture's units.
    assert_eq!(plain.paths[0].points[0], (1.0 + 20.2, 0.0));
    assert_eq!(half.paths[0].points[0], (1.0 + 40.4, 0.0));
    // and both land at the same distance from the node on the page.
    let ops = to_pdf_ops(&half, 0.0, 0.0);
    assert!(
        ops.contains(&format!("{:.2} 0.00 m\n", 0.5 + 20.2)),
        "{ops}"
    );
}

// ---- the bounding box, which is what a page reserves ---------------------

#[test]
fn the_bounding_box_grows_by_half_the_line_width_around_a_stroked_path() {
    // `\pgfusepath{stroke}` adds half the line width to the picture's box on
    // every side (pgfcorepathusage.code.tex lines 116-131), and it is exactly
    // why a picture sits ABOVE the baseline it is set on: lualatex places
    //
    //     \begin{tikzpicture}\draw[thick] (0,0) -- (3,0) -- (3,2) -- cycle;
    //
    // with `1 0 0 1 184.139 610.107 cm` on a baseline at 609.708 -- 0.399 big
    // points up, which is half of `thick`'s 0.79701. Read without it the
    // picture is drawn a hair into the line below.
    let pic = texrs::tikz::parse_document(
        "",
        r"\draw[thick] (0,0) -- (3,0) -- (3,2) -- cycle;",
        &texrs::colour::Colours::new(),
        &texrs::tikz::Estimate,
    );
    let (min_x, min_y, max_x, max_y) = pic.bounds();
    // `thick` is 0.8pt (pgfcore, `\pgfsetlinewidth{0.8pt}`), so half is 0.4.
    assert_eq!((min_x, min_y), (-0.4, -0.4));
    let cm = 72.27 / 2.54;
    assert!((max_x - (3.0 * cm + 0.4)).abs() < 1e-9, "got {max_x}");
    assert!((max_y - (2.0 * cm + 0.4)).abs() < 1e-9, "got {max_y}");
}

#[test]
fn a_curves_control_points_are_inside_the_bounding_box() {
    // `\pgf@lt@curveto` protocols all THREE of its points
    // (pgfcorepathconstruct.code.tex lines 92-97), so a curve reserves the hull
    // it is drawn inside. Measured on the endpoints alone, the S below would
    // claim a box one unit tall for a curve that reaches up to y=1 and down to
    // y=-1, and the page would draw it through the line above.
    let pic = parse("", r"\draw (0,0) .. controls (1,1) and (2,-1) .. (3,0);");
    let (_, min_y, _, max_y) = pic.bounds();
    assert_eq!(
        (min_y, max_y),
        (-1.2, 1.2),
        "the controls, and half of 0.4pt"
    );
}

#[test]
fn an_empty_picture_has_an_empty_box() {
    // A picture that draws nothing takes no room, and answering with an
    // infinite box -- which is what folding over no points leaves -- would make
    // every arithmetic downstream of it a NaN.
    assert_eq!(parse("", "").bounds(), (0.0, 0.0, 0.0, 0.0));
    assert_eq!(parse("", "").extent(), (0.0, 0.0));
}

#[test]
fn a_documents_bare_coordinate_is_a_centimetre() {
    // PGF's default unit vectors are `\pgfsetxvec{\pgfpoint{1cm}{0cm}}` and
    // `\pgfsetyvec{\pgfpoint{0cm}{1cm}}` (pgfcorepoints.code.tex:922-925).
    // `parse` answers in the picture's OWN units, one point to the unit, and
    // the caller multiplies by `x=`/`y=`; a DOCUMENT states neither, so its
    // unit is PGF's and `parse_document` is what supplies it.
    let source = r"\draw (0,0) -- (1,0);";
    let own = parse("", source);
    assert_eq!((own.x_scale, own.y_scale), (1.0, 1.0));
    let document = texrs::tikz::parse_document(
        "",
        source,
        &texrs::colour::Colours::new(),
        &texrs::tikz::Estimate,
    );
    assert_eq!(document.x_scale, 72.27 / 2.54);
    // and a picture that states its own unit keeps it.
    let stated = texrs::tikz::parse_document(
        "x=0.38pt,y=0.38pt",
        source,
        &texrs::colour::Colours::new(),
        &texrs::tikz::Estimate,
    );
    assert_eq!((stated.x_scale, stated.y_scale), (0.38, 0.38));
}

#[test]
fn a_draw_that_names_a_fill_colour_fills_as_well_as_strokes() {
    // `\tikzoption{fill}` ends in `\tikz@addmode{\tikz@mode@filltrue}`
    // (tikz.code.tex lines 507-519): naming a fill colour turns filling ON,
    // whichever command the path came from. lualatex fills the rectangle
    // below solid orange; this used to write the fill colour, work out that
    // the path was filled, and then paint it with a bare `S` -- an outline
    // where the document drew a solid.
    let ops = emitted(r"\draw[fill=orange] (0,0) rectangle (1,1);");
    assert!(ops.contains("1 0.5 0 rg\n"), "the fill colour: {ops}");
    assert!(
        ops.contains("\nb\n") || ops.contains("\nB\n"),
        "filled and stroked: {ops}"
    );
    assert!(
        !ops.contains("\nS\n") && !ops.contains("\ns\n"),
        "not stroked alone: {ops}"
    );
    // `fill=none` turns it off again, and the path is stroked and not filled.
    let none = emitted(r"\draw[fill=none] (0,0) rectangle (1,1);");
    assert!(none.contains("\ns\n") || none.contains("\nS\n"), "{none}");
    assert!(!none.contains("\nB"), "nothing was filled: {none}");
}
