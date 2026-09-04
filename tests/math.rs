//! What `mlist_to_hlist` produces, measured against `tex.web`'s own rules.
//!
//! Every expectation here is COMPUTED from the same font parameters TeX reads
//! -- `cmsy10`'s and `cmex10`'s `fontdimen`s -- by the rule the relevant
//! section states, and compared against what the engine placed. A test that
//! hard-coded "the superscript sits 4.5pt up" would pin this machine's
//! Computer Modern rather than §758's rule, and would say nothing at all if
//! the rule were wrong.
//!
//! The metrics belong to a TeX INSTALLATION, so a machine without one skips
//! rather than fails, exactly as `tests/typeset.rs` does.

use texrs::math::font::MathFonts;
use texrs::math::noad::{Class, Space, DISPLAY_STYLE, SCRIPT_STYLE, TEXT_STYLE};
use texrs::math::set::Setting;
use texrs::math::{parse_formula, set_mlist};

/// One point, in scaled points.
const PT: i64 = 65536;

/// The document size every test sets at: plain TeX's own, so `cmr10` is at ten
/// points and the script sizes are `cmr7` and `cmr5`.
const SIZE: f64 = 10.0;

/// The families, or `None` on a machine with no TeX installation.
fn fonts() -> Option<MathFonts> {
    let f = MathFonts::load(SIZE);
    match f.usable() {
        true => Some(f),
        false => {
            eprintln!("skipping: no TeX installation, so there are no math metrics to measure in");
            None
        }
    }
}

/// Set a formula written the way a document writes it.
fn set(source: &str, style: i64) -> Option<Setting> {
    let list = parse_formula(source).expect("the formula parses");
    set_mlist(&list, style, SIZE)
}

/// Every glyph of a setting, as `(character, x, y, size)`.
fn glyphs(s: &Setting) -> Vec<(char, i64, i64, i64)> {
    s.glyphs.iter().map(|g| (g.ch, g.x, g.y, g.size)).collect()
}

fn glyph(s: &Setting, ch: char) -> Option<(char, i64, i64, i64)> {
    glyphs(s).into_iter().find(|g| g.0 == ch)
}

/// §764's table is the specification, so the port of it is checked against
/// The TeXbook's reading of the same table rather than against itself.
#[test]
fn the_inter_element_spacing_table_is_the_one_tex_web_preloads() {
    use texrs::math::noad::spacing;
    // The row for Ord, which is the one every formula meets first.
    assert_eq!(spacing(Class::Ord, Class::Ord), Space::None);
    assert_eq!(spacing(Class::Ord, Class::Bin), Space::ConditionalMedium);
    assert_eq!(spacing(Class::Ord, Class::Rel), Space::ConditionalThick);
    // An Op takes a thin space either side, unconditionally on the left.
    assert_eq!(spacing(Class::Ord, Class::Op), Space::Thin);
    assert_eq!(spacing(Class::Op, Class::Ord), Space::Thin);
    // A Punct is followed by a conditional thin space and preceded by none.
    assert_eq!(spacing(Class::Ord, Class::Punct), Space::None);
    assert_eq!(spacing(Class::Punct, Class::Ord), Space::ConditionalThin);
    // An Open takes nothing on either side.
    assert_eq!(spacing(Class::Open, Class::Ord), Space::None);
    assert_eq!(spacing(Class::Ord, Class::Close), Space::None);
}

/// §766: a "conditional" space is `\nonscript`, and vanishes at script size.
///
/// This is the check that the table is being READ as §766 reads it rather
/// than merely stored: `$a+b$` is wider than `$ab$` in text style by two
/// medium spaces, and exactly as wide as `$ab$` in script style, where the
/// same two spaces are conditional and so are not inserted.
#[test]
fn a_conditional_space_is_inserted_in_text_style_and_not_in_script_style() {
    let Some(f) = fonts() else { return };
    let (Some(text_plus), Some(text_bare)) = (set("a+b", TEXT_STYLE), set("ab", TEXT_STYLE)) else {
        return;
    };
    let (Some(script_plus), Some(script_bare)) =
        (set("a+b", SCRIPT_STYLE), set("ab", SCRIPT_STYLE))
    else {
        return;
    };
    // `\medmuskip` is 4mu (plain.tex:374), and one mu is `math_quad/18`
    // (§703). Two of them, one either side of the `+`.
    let cur_mu = f.math_quad(0) / 18;
    let medium = 4 * cur_mu;
    let plus_width = f
        .font(0, 0)
        .and_then(|r| r.metrics(b'+'))
        .expect("cmr10 has a plus sign")
        .width;
    let widened = text_plus.width - text_bare.width;
    assert!(
        (widened - (plus_width + 2 * medium)).abs() <= 2,
        "text style put {widened} sp around a {plus_width} sp plus sign, \
         where §764's two medium spaces are {}",
        plus_width + 2 * medium
    );
    let script_widened = script_plus.width - script_bare.width;
    let script_plus_width = f
        .font(0, 1)
        .and_then(|r| r.metrics(b'+'))
        .expect("cmr7 has a plus sign")
        .width;
    assert_eq!(
        script_widened, script_plus_width,
        "a conditional space was inserted at script size, where §766 inserts none"
    );
}

/// §728: a Bin with nothing in front of it to bind is an Ord, and an Ord
/// takes no space at all.
#[test]
fn a_binary_operator_with_nothing_to_bind_becomes_an_ordinary_one() {
    let Some(_) = fonts() else { return };
    let (Some(leading), Some(bare)) = (set("-x", TEXT_STYLE), set("x", TEXT_STYLE)) else {
        return;
    };
    let (Some(binary), Some(pair)) = (set("a-x", TEXT_STYLE), set("ax", TEXT_STYLE)) else {
        return;
    };
    let minus = MathFonts::load(SIZE)
        .font(2, 0)
        .and_then(|s| s.metrics(0x00))
        .expect("cmsy10 has a minus sign")
        .width;
    assert_eq!(
        leading.width - bare.width,
        minus,
        "a leading `-` was given inter-element spacing, where §728 makes it an Ord"
    );
    assert!(
        binary.width - pair.width > minus,
        "a `-` between two letters was given no spacing, where §764 gives it a medium space either side"
    );
}

/// §758: a superscript's baseline rises by `sup2` in uncramped text style,
/// and it is set in the SCRIPT font -- which is the whole visible difference
/// between `$x^2$` and `$x2$`.
#[test]
fn a_superscript_rises_by_sup2_and_is_set_at_script_size() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("x^2", TEXT_STYLE) else {
        return;
    };
    let two = glyph(&s, '2').expect("the superscript is set");
    let x = glyph(&s, 'x').expect("the nucleus is set");
    assert_eq!(x.2, 0, "the nucleus is on the formula's own baseline");
    // §756: the nucleus is a character node, so `shift_up` starts at zero;
    // §758 then raises it to `sup2`, and to a quarter of the x-height above
    // the script's depth if that is more. A digit has no depth.
    let sup2 = f.sup2(0);
    let quarter = f.math_x_height(0).abs() / 4;
    let expected = sup2.max(quarter);
    assert_eq!(
        two.2, expected,
        "the superscript sits at {} sp where §758 puts it at {expected} sp",
        two.2
    );
    assert_eq!(
        two.3,
        7 * PT,
        "the superscript is set at {} pt, not at `\\scriptfont`'s seven",
        two.3 as f64 / PT as f64
    );
    // §757/§759 reserve `\scriptspace` after a script, so the formula is
    // wider than the two glyphs come to.
    let plain_width: i64 = f
        .font(1, 0)
        .and_then(|m| m.metrics(b'x'))
        .map(|m| m.width + m.italic)
        .unwrap_or(0)
        + f.font(0, 1)
            .and_then(|m| m.metrics(b'2'))
            .map(|m| m.width)
            .unwrap_or(0);
    assert_eq!(
        s.width,
        plain_width + PT / 2,
        "the formula does not carry plain.tex's 0.5pt \\scriptspace"
    );
}

/// §757: a subscript on its own drops by `sub1`, or further if its top would
/// otherwise rise above four fifths of the x-height.
#[test]
fn a_subscript_drops_by_section_757s_rule() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("x_1", TEXT_STYLE) else {
        return;
    };
    let one = glyph(&s, '1').expect("the subscript is set");
    let height = f
        .font(0, 1)
        .and_then(|m| m.metrics(b'1'))
        .expect("cmr7 has a one")
        .height;
    let expected = f.sub1(0).max(height - (f.math_x_height(0).abs() * 4) / 5);
    assert_eq!(
        one.2, -expected,
        "the subscript sits at {} sp where §757 puts it at {} sp below the baseline",
        one.2, expected
    );
}

/// §759: with both scripts present they are held four rule thicknesses apart,
/// and the superscript is offset to the right of the subscript by the
/// nucleus's italic correction.
#[test]
fn a_sub_and_a_superscript_together_are_four_rule_thicknesses_apart() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("x^a_b", TEXT_STYLE) else {
        return;
    };
    let sup = glyph(&s, 'a').expect("the superscript is set");
    let sub = glyph(&s, 'b').expect("the subscript is set");
    let gap = (sup.2
        - f.font(1, 1)
            .and_then(|m| m.metrics(b'a'))
            .expect("cmmi7 has an a")
            .depth)
        - (sub.2
            + f.font(1, 1)
                .and_then(|m| m.metrics(b'b'))
                .expect("cmmi7 has a b")
                .height);
    assert!(
        gap >= 4 * f.default_rule_thickness(0),
        "the scripts are {gap} sp apart, where §759 keeps them at least {} sp apart",
        4 * f.default_rule_thickness(0)
    );
    // The superscript is `delta` -- the italic correction of `x` in cmmi10 --
    // to the right of the subscript.
    let delta = f
        .font(1, 0)
        .and_then(|m| m.metrics(b'x'))
        .expect("cmmi10 has an x")
        .italic;
    assert_eq!(
        sup.1 - sub.1,
        delta,
        "the superscript is not offset by the nucleus's italic correction"
    );
}

/// §746 and §747: the bar of a fraction is `default_rule_thickness` thick and
/// is centred on the axis; §748 puts a `\nulldelimiterspace` on each side.
#[test]
fn a_fraction_bar_is_centred_on_the_axis_and_is_one_rule_thick() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\frac{1}{2}", TEXT_STYLE) else {
        return;
    };
    assert_eq!(s.rules.len(), 1, "a fraction draws exactly one bar");
    let bar = s.rules[0];
    let thickness = f.default_rule_thickness(0);
    assert_eq!(bar.height, thickness, "the bar is not one rule thick");
    // §747: the rule's centre is the axis, so its bottom edge is half a
    // thickness below it. `half` rounds an odd number up (§100).
    let half = match thickness % 2 == 0 {
        true => thickness / 2,
        false => (thickness + 1) / 2,
    };
    assert_eq!(
        bar.y,
        f.axis_height(0) - half,
        "the bar sits at {} sp where §747 centres it on the {} sp axis",
        bar.y,
        f.axis_height(0)
    );
    // §748: two null delimiters, each `\nulldelimiterspace` wide.
    let null_space = 78643;
    assert_eq!(
        s.width,
        bar.width + 2 * null_space,
        "the fraction is not flanked by plain.tex's 1.2pt \\nulldelimiterspace"
    );
    assert_eq!(
        bar.x, null_space,
        "the bar starts at the left null delimiter"
    );
    // The numerator is above the bar and the denominator below it, both in
    // the smaller style -- §744's `num_style` and `denom_style`.
    let numerator = glyph(&s, '1').expect("the numerator is set");
    let denominator = glyph(&s, '2').expect("the denominator is set");
    assert!(numerator.2 > bar.y, "the numerator is not above the bar");
    assert!(
        denominator.2 < bar.y,
        "the denominator is not below the bar"
    );
    assert_eq!(
        numerator.3,
        7 * PT,
        "a text-style fraction sets its numerator at script size (§702)"
    );
}

/// §745: `\atop` has no bar at all, and holds the two apart by three rule
/// thicknesses in text style.
#[test]
fn atop_draws_no_bar() {
    let Some(_) = fonts() else { return };
    let Some(s) = set("1\\atop 2", TEXT_STYLE) else {
        return;
    };
    assert!(s.rules.is_empty(), "\\atop drew a fraction bar");
    assert_eq!(s.glyphs.len(), 2, "\\atop set {} glyphs", s.glyphs.len());
}

/// §737: a radical is a sign with a rule over its nucleus, and the rule is
/// exactly as wide as what it covers.
#[test]
fn a_radical_puts_a_rule_over_exactly_its_nucleus() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\sqrt{2}", TEXT_STYLE) else {
        return;
    };
    assert_eq!(s.rules.len(), 1, "a radical draws exactly one bar");
    let two = f
        .font(0, 0)
        .and_then(|m| m.metrics(b'2'))
        .expect("cmr10 has a two")
        .width;
    assert_eq!(
        s.rules[0].width, two,
        "the bar is {} sp over a {two} sp nucleus",
        s.rules[0].width
    );
    assert_eq!(
        s.rules[0].height,
        f.default_rule_thickness(0),
        "the bar of a radical is one rule thick (§737)"
    );
    // The sign itself is drawn, and it starts at the left of the formula.
    let sign = glyph(&s, '√').expect("the radical sign is set");
    assert_eq!(sign.1, 0);
    assert!(
        s.rules[0].x >= sign.1,
        "the bar starts left of the radical sign"
    );
}

/// §734: `\overline` is a rule three thicknesses above its nucleus.
#[test]
fn an_overline_is_three_rule_thicknesses_above_what_it_covers() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\overline{x}", TEXT_STYLE) else {
        return;
    };
    assert_eq!(s.rules.len(), 1);
    let t = f.default_rule_thickness(0);
    let x_height = f
        .font(1, 0)
        .and_then(|m| m.metrics(b'x'))
        .expect("cmmi10 has an x")
        .height;
    assert_eq!(
        s.rules[0].y,
        x_height + 3 * t,
        "the bar sits at {} sp where §705's kern puts it at {} sp",
        s.rules[0].y,
        x_height + 3 * t
    );
    assert_eq!(s.rules[0].height, t);
}

/// §749: in display style an operator is replaced by the next larger variant
/// in its character list, which is why `\sum` is bigger in a display.
#[test]
fn a_large_operator_grows_in_display_style() {
    let Some(_) = fonts() else { return };
    let (Some(text), Some(display)) = (set("\\sum", TEXT_STYLE), set("\\sum", DISPLAY_STYLE))
    else {
        return;
    };
    assert!(
        display.height + display.depth > text.height + text.depth,
        "the display-style sum is {} sp tall and the text-style one {} sp",
        display.height + display.depth,
        text.height + text.depth
    );
    assert!(
        display.width > text.width,
        "the display-style sum is no wider than the text-style one"
    );
}

/// §750-§751: `\limits` puts the scripts above and below rather than beside,
/// so the box grows vertically and not horizontally.
#[test]
fn limits_go_above_and_below_an_operator() {
    let Some(_) = fonts() else { return };
    let (Some(beside), Some(above)) = (
        set("\\sum\\nolimits_1^2", TEXT_STYLE),
        set("\\sum\\limits_1^2", TEXT_STYLE),
    ) else {
        return;
    };
    assert!(
        above.height > beside.height && above.depth > beside.depth,
        "\\limits did not put the scripts above and below: {} / {} against {} / {}",
        above.height,
        above.depth,
        beside.height,
        beside.depth
    );
    assert!(
        above.width < beside.width,
        "\\limits set the scripts beside the operator after all"
    );
    // The limits are centred over the operator, which is what §750's `rebox`
    // is for: both scripts sit inside the operator's own width.
    let one = glyph(&above, '1').expect("the lower limit is set");
    let two = glyph(&above, '2').expect("the upper limit is set");
    assert!(one.2 < 0 && two.2 > 0, "the limits are on the baseline");
}

/// §762: a `\left` delimiter is grown to the formula it encloses, so the same
/// parenthesis around a fraction is bigger than around a letter.
#[test]
fn a_left_delimiter_grows_to_the_formula_it_encloses() {
    let Some(_) = fonts() else { return };
    let (Some(small), Some(large)) = (
        set("\\left(x\\right)", TEXT_STYLE),
        set("\\left(\\frac{1}{2}\\right)", TEXT_STYLE),
    ) else {
        return;
    };
    assert!(
        glyph(&small, '(').is_some() && glyph(&small, ')').is_some(),
        "the delimiters are not set at all"
    );
    assert!(
        large.height + large.depth > small.height + small.depth,
        "the delimiter did not grow: {} sp against {} sp",
        large.height + large.depth,
        small.height + small.depth
    );
    // §762 says exactly how big: `\delimiterfactor/500` of the greatest
    // distance from the axis, or twice that distance less
    // `\delimitershortfall`, whichever is more.
    let Some(inner) = set("\\frac{1}{2}", TEXT_STYLE) else {
        return;
    };
    let f = MathFonts::load(SIZE);
    let delta2 = inner.depth + f.axis_height(0);
    let delta1 = (inner.height + inner.depth - delta2).max(delta2);
    let wanted = ((delta1 / 500) * 901).max(delta1 + delta1 - 5 * PT);
    assert!(
        large.height + large.depth >= wanted,
        "the delimiter came out {} sp tall where §762 asks for {wanted} sp",
        large.height + large.depth
    );
    // §706 then centres it on the axis, which for a `cmex10` delimiter --
    // whose body hangs below its own baseline -- lifts that baseline well
    // above the formula's. The small one is `cmr10`'s and sits on it.
    let big = glyph(&large, '(').expect("the left parenthesis is set");
    let small_paren = glyph(&small, '(').expect("the left parenthesis is set");
    assert_eq!(
        small_paren.2, 0,
        "a text-size parenthesis is on the baseline"
    );
    assert!(
        big.2 > 0,
        "the grown delimiter was not re-centred on the axis"
    );
}

/// §702 and §703: a script is one style smaller, and a script of a script is
/// smaller again but no smaller than that.
#[test]
fn the_styles_step_down_and_stop_at_scriptscript() {
    let Some(_) = fonts() else { return };
    let Some(s) = set("x^{y^{z^{w}}}", TEXT_STYLE) else {
        return;
    };
    let size = |c: char| glyph(&s, c).map(|g| g.3);
    assert_eq!(size('x'), Some(10 * PT));
    assert_eq!(size('y'), Some(7 * PT));
    assert_eq!(size('z'), Some(5 * PT));
    assert_eq!(
        size('w'),
        Some(5 * PT),
        "a fourth level went below \\scriptscriptfont, which §702 does not"
    );
}

/// A formula still says what it says to a reader who asked for the words
/// rather than for a page.
#[test]
fn a_formula_reads_as_its_own_words_in_the_text_a_reader_gets() {
    use texrs::math::set::plain;
    assert_eq!(plain(&parse_formula("x^2+1").unwrap()), "x²+1");
    assert_eq!(plain(&parse_formula("a_1").unwrap()), "a₁");
    assert_eq!(plain(&parse_formula("\\alpha\\beta").unwrap()), "αβ");
    assert_eq!(plain(&parse_formula("\\frac{a}{b}").unwrap()), "a/b");
    assert_eq!(plain(&parse_formula("\\sqrt{2}").unwrap()), "√2");
    assert_eq!(plain(&parse_formula("\\sum_{i=1}^{n}").unwrap()), "∑^n_i=1");
    // Not one character of it is a space, because the line breaker splits a
    // paragraph on spaces and a formula is one word.
    let set = plain(&parse_formula("a + b \\leq c").unwrap());
    assert_eq!(set, "a+b≤c");
}

/// The Greek, the operators and the relations a real document writes all
/// resolve to a family and a slot rather than being dropped.
#[test]
fn the_symbols_a_document_writes_all_reach_a_font() {
    let Some(_) = fonts() else { return };
    for (source, expected) in [
        ("\\alpha", 'α'),
        ("\\Omega", 'Ω'),
        ("\\sum", '∑'),
        ("\\prod", '∏'),
        ("\\int", '∫'),
        ("\\infty", '∞'),
        ("\\partial", '∂'),
        ("\\leq", '≤'),
        ("\\geq", '≥'),
        ("\\times", '×'),
        // `cmsy10`'s slot 1 is the centred dot, and U+00B7 is the one every
        // text face has a glyph for; U+22C5 is the same shape and is in
        // almost none of them.
        ("\\cdot", '·'),
        ("\\rightarrow", '→'),
        ("\\in", '∈'),
        ("\\nabla", '∇'),
    ] {
        let Some(s) = set(source, TEXT_STYLE) else {
            return;
        };
        assert!(
            glyph(&s, expected).is_some(),
            "{source} set {:?} rather than {expected:?}",
            glyphs(&s)
        );
    }
}

/// A formula written the way a document writes it reaches the reader as its
/// own words, and reaches the page as boxes.
///
/// The two halves travel the same marker -- the setting in its spec, the words
/// between it and its close -- so this is the check that neither half leaks
/// into the other's path.
#[test]
fn a_formula_in_a_document_reaches_both_the_reader_and_the_page() {
    let Some(_) = fonts() else { return };
    let source = "\\documentclass{article}\n\\begin{document}\n\
                  Euler wrote \\(e^{i\\pi}+1=0\\) and the mean is \
                  \\(\\frac{a+b}{2}\\).\n\\end{document}\n";
    let text = texrs::run_text(source).expect("the document runs");
    assert!(
        text.contains("e^iπ+1=0"),
        "the reader did not get the formula's words: {text:?}"
    );
    assert!(
        text.contains("a+b/2"),
        "the reader did not get the fraction's words: {text:?}"
    );
    let leaked: Vec<char> = text
        .chars()
        .filter(|c| c.is_control() && *c != '\n')
        .collect();
    assert!(
        leaked.is_empty(),
        "the formula's marker left {leaked:?} in the text a reader gets"
    );
    // The page draws the fraction's BAR, which is a rule and not a character:
    // no run of text can produce one, so its presence in the content stream
    // is the setting having reached the page.
    let pdf = texrs::run_pdf(source).expect("the document sets");
    let plain = String::from_utf8_lossy(&texrs::pdf::inflate_streams(&pdf)).to_string();
    assert!(
        plain.contains(" re f"),
        "the fraction bar was not drawn on the page"
    );
    assert!(
        texrs::pdf_page_count(&pdf) >= 1,
        "the document produced no page at all"
    );
}

/// `\$` is a dollar sign and `$…$` is a formula, in the same document.
///
/// `src/latex/prelude.tex` defines `\$` as the character while it is still an
/// ordinary one, and `\begin{document}` is where the character becomes a math
/// shift -- so the definition keeps the dollar sign and the document's own
/// `$` opens maths. Getting this wrong turns every price in every book into a
/// runaway formula.
#[test]
fn an_escaped_dollar_is_a_dollar_and_a_bare_one_opens_a_formula() {
    let source = "\\documentclass{article}\n\\begin{document}\n\
                  It costs \\$5 and $a+b$ holds.\n\\end{document}\n";
    let text = texrs::run_text(source).expect("the document runs");
    assert!(text.contains("$5"), "the escaped dollar was lost: {text:?}");
    assert!(
        text.contains("a+b") && !text.contains("$a"),
        "the bare dollar did not open a formula: {text:?}"
    );
}

/// §1046: a `\par` in math mode closes the formula, so a stray `$` costs one
/// paragraph rather than the rest of the document.
#[test]
fn a_stray_dollar_stops_at_the_end_of_its_paragraph() {
    let source = "\\documentclass{article}\n\\begin{document}\n\
                  A stray $ dollar here.\n\n\
                  The paragraph after it is unharmed.\n\\end{document}\n";
    let text = texrs::run_text(source).expect("the document runs");
    assert!(
        text.contains("The paragraph after it is unharmed."),
        "a stray dollar swallowed the rest of the document: {text:?}"
    );
}

/// A named operator is upright roman set as one Op noad, which is what
/// plain.tex:1054 makes it -- so `\log x` is not three italic variables.
#[test]
fn an_operator_name_is_upright_roman_and_takes_a_thin_space() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\log x", TEXT_STYLE) else {
        return;
    };
    let letters: String = s.glyphs.iter().map(|g| g.ch).collect();
    assert_eq!(letters, "logx");
    // §764 puts a thin space between an Op and the Ord after it. `\thinmuskip`
    // is 3mu (plain.tex:373).
    let thin = 3 * (f.math_quad(0) / 18);
    let log_width: i64 = "log"
        .bytes()
        .filter_map(|b| f.font(0, 0).and_then(|r| r.metrics(b)).map(|m| m.width))
        .sum();
    // §755 puts the LAST letter's italic correction after it, because nothing
    // follows it inside the operator's own list -- the three letters before
    // it are `math_text_char`s and get none (§752).
    let g_italic = f
        .font(0, 0)
        .and_then(|r| r.metrics(b'g'))
        .expect("cmr10 has a g")
        .italic;
    let x = glyph(&s, 'x').expect("the variable is set");
    assert!(
        (x.1 - (log_width + g_italic + thin)).abs() <= 2,
        "the variable starts at {} sp where a thin space after `log` puts it at {}",
        x.1,
        log_width + g_italic + thin
    );
    // The `l` and the `o` get NO italic correction: each is followed by
    // another cmr10 character, so §752 makes it a `math_text_char` and §755
    // drops the correction for a font that has a space.
    let o = glyph(&s, 'o').expect("the second letter is set");
    let l_width = f
        .font(0, 0)
        .and_then(|r| r.metrics(b'l'))
        .expect("cmr10 has an l")
        .width;
    assert_eq!(
        o.1, l_width,
        "the `o` of `log` was pushed right by an italic correction §752 suppresses"
    );
}
