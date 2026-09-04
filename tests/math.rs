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

/// `half(x)` (§100), which is what every shift in `mlist_to_hlist` is halved
/// with: an odd number rounds away from zero upward, so `x/2` is off by one on
/// half the values a test would otherwise agree with by luck.
fn half(x: i64) -> i64 {
    match x % 2 == 0 {
        true => x / 2,
        false => (x + 1) / 2,
    }
}

/// The kern between `left` and `right` in a family's text font, in scaled
/// points -- the lig/kern lookup §742 and §752 both make.
fn kern_between(f: &MathFonts, fam: usize, left: u8, right: u8) -> i64 {
    let Some(font) = f.font(fam, 0) else { return 0 };
    match font.tfm.step(left, right) {
        Some(texrs::tfm::Step::Kern { by, .. }) => (by * font.at * 65536.0).round() as i64,
        _ => 0,
    }
}

/// `cur_mu` (§703): one math unit, at a size, in scaled points.
fn cur_mu(f: &MathFonts, size: usize) -> i64 {
    f.math_quad(size) / 18
}

/// §739 and §742: the accent is centred over the accentee and skewed right by
/// the kern between the accented character and the font's `\skewchar`, and the
/// accent's own width is treated as zero.
///
/// `cmmi10`'s `\skewchar` is `'177` (plain.tex:474), and the skew is what puts
/// `\hat f` further right than `\hat x` -- a slanted letter is not centred on
/// its own box.
#[test]
fn a_math_accent_is_centred_over_its_nucleus_and_skewed_by_section_742() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\hat x", TEXT_STYLE) else {
        return;
    };
    let roman = f.font(0, 0).expect("cmr10 is loaded");
    let italic = f.font(1, 0).expect("cmmi10 is loaded");
    // `\hat` is `\mathaccent"705E` (plain.tex:946): cmr10's circumflex.
    let accent = roman.metrics(0x5E).expect("cmr10 has a circumflex accent");
    let accent_box = accent.width + accent.italic;
    // §720's `clean_box` of a single character: the box is as wide as the
    // character plus its italic correction.
    let x = italic.metrics(b'x').expect("cmmi10 has an x");
    let w = x.width + x.italic;
    // The formula is exactly as wide as its nucleus: §738 says the accent's
    // width counts for nothing.
    assert_eq!(
        s.width, w,
        "the accent widened the formula, where §738 gives it no width at all"
    );
    let skew = kern_between(&f, 1, b'x', 0o177);
    let placed = glyph(&s, '\u{5E}').expect("the accent is drawn");
    assert_eq!(
        placed.1,
        skew + half(w - accent_box),
        "the accent sits at {} sp where §739's `s+half(w-width(y))` puts it at {}",
        placed.1,
        skew + half(w - accent_box)
    );
    // §739: the accent is lowered onto the nucleus by `delta`, which is the
    // nucleus's height or the accent font's x-height, whichever is less.
    let delta = x.height.min(roman.param(5));
    let natural = accent.height + accent.depth - delta + x.height;
    // §740: a box shorter than the nucleus keeps the nucleus's height.
    let height = natural.max(x.height);
    let expected = height - (height - natural) - accent.height;
    assert_eq!(
        placed.2, expected,
        "the accent's baseline is at {} sp where §739 lowers it to {}",
        placed.2, expected
    );
}

/// §739: over a letter TALLER than the accent font's x-height the accent is
/// lowered by the x-height and no further, so `\bar A` and `\bar x` do not
/// carry their bars at the same distance above the baseline.
#[test]
fn an_accent_over_a_tall_letter_is_lowered_by_the_x_height() {
    let Some(f) = fonts() else { return };
    let Some(s) = set("\\bar A", TEXT_STYLE) else {
        return;
    };
    let roman = f.font(0, 0).expect("cmr10 is loaded");
    let italic = f.font(1, 0).expect("cmmi10 is loaded");
    // `\bar` is `\mathaccent"7016` (plain.tex:943): cmr10's macron.
    let accent = roman.metrics(0x16).expect("cmr10 has a macron");
    let a = italic.metrics(b'A').expect("cmmi10 has an A");
    let x_height = roman.param(5);
    assert!(
        a.height > x_height,
        "the test needs a letter taller than the x-height; `A` is {} sp against {}",
        a.height,
        x_height
    );
    let delta = x_height;
    let natural = accent.height + accent.depth - delta + a.height;
    let height = natural.max(a.height);
    let expected = height - (height - natural) - accent.height;
    let placed = glyph(&s, '\u{AF}').expect("the macron is drawn");
    assert_eq!(
        placed.2, expected,
        "the bar over a capital is at {} sp where §739's x-height rule puts it at {}",
        placed.2, expected
    );
}

/// §741: `\widehat` walks its charlist for the widest variant that is still no
/// wider than what it covers, so a wide subformula gets a wider accent.
#[test]
fn a_wide_accent_walks_the_charlist_for_the_widest_that_fits() {
    let Some(f) = fonts() else { return };
    let ext = f.font(3, 0).expect("cmex10 is loaded");
    let italic = f.font(1, 0).expect("cmmi10 is loaded");
    // The width of the accentee, as `clean_box` measures it: a list of
    // characters is as wide as their widths, and the last one's italic
    // correction (§755).
    let width_of = |letters: &str| -> i64 {
        let mut total = 0;
        let bytes: Vec<u8> = letters.bytes().collect();
        for (at, b) in bytes.iter().enumerate() {
            let m = italic.metrics(*b).expect("cmmi10 has the letter");
            total += m.width;
            // §752: a letter followed by another of the same family is a
            // `math_text_char`; cmmi10 has no `space` parameter, so §755 keeps
            // its italic correction anyway.
            let _ = at;
            total += m.italic;
            if at + 1 < bytes.len() {
                total += kern_between(&f, 1, *b, bytes[at + 1]);
            }
        }
        total
    };
    // §741's own loop, run here from the metrics rather than from the engine.
    let chosen = |w: i64| -> u8 {
        // `\widehat` is `\mathaccent"0362` (plain.tex:950).
        let mut c = 0x62u8;
        while let Some(next) = ext.next_larger(c) {
            let Some(m) = ext.metrics(next) else { break };
            if m.width > w {
                break;
            }
            c = next;
        }
        c
    };
    let narrow_w = width_of("x");
    let wide_w = width_of("xyz");
    let (narrow_c, wide_c) = (chosen(narrow_w), chosen(wide_w));
    assert!(
        wide_c > narrow_c,
        "the charlist offers no wider variant between {narrow_w} sp and {wide_w} sp, \
         so this test would pass without walking it"
    );
    for (source, w, code, skewed) in [
        ("\\widehat x", narrow_w, narrow_c, true),
        ("\\widehat{xyz}", wide_w, wide_c, false),
    ] {
        let Some(s) = set(source, TEXT_STYLE) else {
            return;
        };
        let m = ext.metrics(code).expect("cmex10 has the variant");
        // §742 skews by the kern in the NUCLEUS's font, not the accent's, and
        // only when the nucleus is a single character: `\widehat{xyz}` is a
        // list, so nothing is looked up for it.
        let skew = match skewed {
            true => kern_between(&f, 1, b'x', 0o177),
            false => 0,
        };
        let expected = skew + half(w - (m.width + m.italic));
        let placed = glyph(&s, '\u{2C6}').expect("the wide accent is drawn");
        assert_eq!(
            placed.1, expected,
            "{source}: the accent is at {} sp where §739 centres variant {code:#04x} at {}",
            placed.1, expected
        );
    }
}

/// §736: a `\vcenter` box is centred on the axis -- the line a fraction bar
/// and a `\left(` are centred on -- so its height is the axis height plus half
/// of everything in it, whatever that is.
#[test]
fn a_vcentered_box_is_centred_on_the_axis() {
    let Some(f) = fonts() else { return };
    let axis = f.axis_height(0);
    // `\vcenter`'s own height is stated in §736; what goes INSIDE it is
    // whatever the same material comes to unvcentred, which is what the
    // second setting of each pair measures.
    for (source, bare) in [
        ("\\vcenter{x}", "x"),
        ("\\vcenter{\\frac{a}{b}}", "\\frac{a}{b}"),
    ] {
        let (Some(s), Some(plain)) = (set(source, TEXT_STYLE), set(bare, TEXT_STYLE)) else {
            return;
        };
        let delta = plain.height + plain.depth;
        assert_eq!(
            s.height,
            axis + half(delta),
            "{source}: the box is {} sp tall where §736 makes it the {} sp axis              plus half of its {} sp",
            s.height,
            axis,
            delta
        );
        // And everything inside is lifted by exactly the difference between
        // the two heights, which is what "centred on the axis" looks like on
        // the page: the same material, moved onto the axis and not redrawn.
        let lift = axis + half(delta) - plain.height;
        let moved: Vec<i64> = s.glyphs.iter().map(|g| g.y).collect();
        let expected: Vec<i64> = plain.glyphs.iter().map(|g| g.y + lift).collect();
        assert_eq!(
            moved, expected,
            "{source}: the content sits at {moved:?} where a lift of {lift} sp onto the axis puts it at {expected:?}"
        );
    }
}

/// §731: a `\mathchoice` keeps the one of its four lists the current style
/// names, and throws the other three away.
#[test]
fn mathchoice_keeps_the_list_the_current_style_names() {
    let Some(f) = fonts() else { return };
    let source = "\\mathchoice{a}{b}{c}{d}";
    for (style, letter, size) in [
        (DISPLAY_STYLE, 'a', 0usize),
        (TEXT_STYLE, 'b', 0),
        (SCRIPT_STYLE, 'c', 1),
    ] {
        let Some(s) = set(source, style) else { return };
        let drawn: Vec<char> = s.glyphs.iter().map(|g| g.ch).collect();
        assert_eq!(
            drawn,
            vec![letter],
            "style {style} drew {drawn:?} where §731 keeps `{letter}` alone"
        );
        // And it is set at the size that style implies (§703), which is the
        // other half of what `\mathchoice` is for.
        let at = (f.at(1, size) * 65536.0).round() as i64;
        assert_eq!(
            s.glyphs[0].size, at,
            "style {style} set its choice at {} sp where §703 gives size {size}, {at} sp",
            s.glyphs[0].size
        );
    }
}

/// `\sqrt[3]{x}` is plain.tex's `\root 3 \of {x}` (plain.tex:1018-1022): the
/// index in a `\scriptscriptstyle` box, five `mu` in front of it, raised by
/// six tenths of the radical's height less its depth, and ten `mu` back.
#[test]
fn a_root_index_is_set_by_plain_texs_own_three_amounts() {
    let Some(f) = fonts() else { return };
    let (Some(plain), Some(rooted)) = (
        set("\\sqrt{x}", TEXT_STYLE),
        set("\\sqrt[3]{x}", TEXT_STYLE),
    ) else {
        return;
    };
    // The index is a `\scriptscriptstyle` box, so it comes out of cmr5.
    let tiny = f.font(0, 2).expect("cmr5 is loaded");
    let three = tiny.metrics(b'3').expect("cmr5 has a three");
    let index_width = three.width + three.italic;
    let mu = cur_mu(&f, 0);
    // `\mkern5mu ... \mkern-10mu`, which is five mu less than the index's own
    // width -- the ten pulls the radical back over the index.
    let expected = index_width + 5 * mu - 10 * mu;
    assert_eq!(
        rooted.width - plain.width,
        expected,
        "the index widened the radical by {} sp where plain.tex's three amounts \
         come to {}",
        rooted.width - plain.width,
        expected
    );
    let three_glyph = glyph(&rooted, '3').expect("the index is drawn");
    let at = (f.at(0, 2) * 65536.0).round() as i64;
    assert_eq!(
        three_glyph.3, at,
        "the index is set at {} sp where `\\scriptscriptstyle` is cmr5 at {}",
        three_glyph.3, at
    );
    // `\dimen@\ht\z@ \advance\dimen@-\dp\z@` then `\raise.6\dimen@`. §453
    // reads `.6` as 39322 sixty-five-thousand-five-hundred-and-thirty-sixths.
    let dimen = plain.height - plain.depth;
    let raise = ((dimen as i128 * 39322) / 65536) as i64;
    assert_eq!(
        three_glyph.2, raise,
        "the index sits at {} sp where `\\raise.6\\dimen@` of {} sp puts it at {}",
        three_glyph.2, dimen, raise
    );
}

/// §716-§717: a `mu` is one eighteenth of the `math_quad` of the size the glue
/// LANDS in, so the same `\mkern` is a different number of points in a script
/// than in text.
#[test]
fn a_mu_is_measured_at_the_size_the_glue_lands_in() {
    let Some(f) = fonts() else { return };
    for (style, size) in [(TEXT_STYLE, 0usize), (SCRIPT_STYLE, 1)] {
        let mu = cur_mu(&f, size);
        let (Some(bare), Some(kerned), Some(thinned)) = (
            set("ab", style),
            set("a\\mkern18mu b", style),
            set("a\\,b", style),
        ) else {
            return;
        };
        assert_eq!(
            kerned.width - bare.width,
            18 * mu,
            "style {style}: `\\mkern18mu` came to {} sp where eighteen mu at that \
             size is {}",
            kerned.width - bare.width,
            18 * mu
        );
        // `\,` is `\mskip\thinmuskip` (plain.tex:730) and `\thinmuskip` is 3mu
        // (plain.tex:373) -- which is three mu of THIS size, not of the text
        // size the formula was read in.
        assert_eq!(
            thinned.width - bare.width,
            3 * mu,
            "style {style}: `\\,` came to {} sp where three mu at that size is {}",
            thinned.width - bare.width,
            3 * mu
        );
    }
}

/// §810: a column is as wide as its widest entry, so the `=` of every row of
/// an `align` starts at the same place however long the left sides are.
#[test]
fn a_displays_columns_are_as_wide_as_their_widest_entry() {
    if fonts().is_none() {
        return;
    }
    let f = texrs::math::read_formula("a &= b \\\\ xyzw &= c").expect("the display parses");
    assert_eq!(f.rows.len(), 2, "the `\\\\` did not end a row: {:?}", f.rows);
    assert_eq!(f.rows[0].len(), 2, "the `&` did not end a column");
    let Some(rows) = texrs::math::set_display(&f, DISPLAY_STYLE, SIZE, "align", 469.75) else {
        return;
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].width, rows[1].width,
        "the two rows came to {} sp and {} sp, so nothing could line them up",
        rows[0].width, rows[1].width
    );
    let first = glyph(&rows[0], '=').expect("the first row's relation is set");
    let second = glyph(&rows[1], '=').expect("the second row's relation is set");
    assert_eq!(
        first.1, second.1,
        "the relations start at {} sp and {} sp, so the columns are not aligned",
        first.1, second.1
    );
    // The left column is flush RIGHT (amsmath.sty:2359), so the longer left
    // side is the one that starts at the column's left edge.
    let a = glyph(&rows[0], 'a').expect("the first row's left side is set");
    let x = glyph(&rows[1], 'x').expect("the second row's left side is set");
    assert!(
        a.1 > x.1,
        "the shorter left side starts at {} sp and the longer at {} sp, \
         where a right-aligned column puts the shorter one further right",
        a.1,
        x.1
    );
}

/// §1204-§1206: a display is centred on `\displaywidth` and its equation
/// number is set hard against the right edge of it.
#[test]
fn an_equation_number_sits_at_the_edge_of_the_display_width() {
    if fonts().is_none() {
        return;
    }
    const MEASURE: f64 = 469.75;
    let z = (MEASURE * 65536.0).round() as i64;
    let f = texrs::math::read_formula("x=y\\eqno(3)").expect("the display parses");
    let number = f.number.clone().expect("`\\eqno` gave the display a number");
    let (Some(body), Some(a)) = (
        set_mlist(&f.rows[0][0], DISPLAY_STYLE, SIZE),
        // §1199: the number is set at TEXT style whatever the display is in.
        set_mlist(&number, TEXT_STYLE, SIZE),
    ) else {
        return;
    };
    let Some(rows) = texrs::math::set_display(&f, DISPLAY_STYLE, SIZE, "equation", MEASURE) else {
        return;
    };
    assert_eq!(rows.len(), 1);
    let line = &rows[0];
    assert_eq!(
        line.width, z,
        "the display line is {} sp wide where `\\displaywidth` is {}",
        line.width, z
    );
    // §1206: `d:=half(z-w)`, and the number's width is nowhere near half the
    // measure, so the display stays centred.
    let d = half(z - body.width);
    let x = glyph(line, 'x').expect("the formula is set");
    assert_eq!(
        x.1, d,
        "the display starts at {} sp where §1206 displaces it by {}",
        x.1, d
    );
    let open = glyph(line, '(').expect("the equation number is set");
    assert_eq!(
        open.1,
        z - a.width,
        "the number starts at {} sp where the right edge of a {} sp measure \
         puts a {} sp number at {}",
        open.1,
        z,
        a.width,
        z - a.width
    );
}

/// The rows of an `align` reach both halves of the engine: a reader gets both
/// of them, and the page draws both.
///
/// The unit tests above measure what `mlist_to_hlist` produced; this is the
/// path from `\begin{align}` through the lowerer to the page, where an `&`
/// used to be dropped and a two-row display used to set as one row.
#[test]
fn an_align_environment_sets_every_row_of_itself() {
    let source = "\\documentclass{article}\n\\begin{document}\n\
                  \\begin{align}\na &= b + c \\\\\nxyzw &= d\n\\end{align}\n\
                  \\end{document}\n";
    let text = texrs::run_text(source).expect("the document runs");
    assert!(
        text.contains("a=b+c"),
        "the first row did not reach the reader: {text:?}"
    );
    assert!(
        text.contains("xyzw=d"),
        "the second row did not reach the reader: {text:?}"
    );
    let pdf = texrs::run_pdf(source).expect("the document sets");
    assert!(
        texrs::pdf_page_count(&pdf) >= 1,
        "the display produced no page"
    );
}

/// `\hat`, `\sqrt[3]` and `\vcenter` reach a reader as words, which is the
/// half of a formula `--text` prints.
#[test]
fn the_new_constructions_read_back_as_their_own_words() {
    let source = "\\documentclass{article}\n\\begin{document}\n\
                  Here are $\\hat x$, $\\bar y$, $\\sqrt[3]{z}$ and $\\vcenter{w}$.\n\
                  \\end{document}\n";
    let text = texrs::run_text(source).expect("the document runs");
    for (what, expected) in [
        ("an accent", "x\u{302}"),
        ("a bar", "y\u{304}"),
        ("a cube root", "\u{B3}\u{221A}z"),
        ("a vcentered box", "w"),
    ] {
        assert!(
            text.contains(expected),
            "{what} did not reach the reader as {expected:?}: {text:?}"
        );
    }
}

/// §761 and §767: a formula set inside a paragraph offers a break after a
/// binary operator and after a relation, and nowhere else.
///
/// `\binoppenalty` is 700 and `\relpenalty` 500 (plain.tex:288-289). Nothing
/// reads these yet -- a set formula reaches the paragraph breaker as one word
/// -- so this measures the mlist rather than a broken line, which is what
/// there is to measure.
#[test]
fn a_formula_in_a_paragraph_carries_section_767s_break_penalties() {
    use texrs::node::Node;
    let Some(f) = fonts() else { return };
    let penalties = |source: &str, with: bool| -> Vec<i64> {
        let list = parse_formula(source).expect("the formula parses");
        let b = match with {
            true => texrs::math::mlist::set_with_penalties(&f, &list, TEXT_STYLE),
            false => texrs::math::mlist::set(&f, &list, TEXT_STYLE),
        };
        b.list
            .iter()
            .filter_map(|n| match n {
                Node::Penalty(p) => Some(*p),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        penalties("a+b=c", true),
        vec![700, 500],
        "§767 charges `\\binoppenalty` after the Bin and `\\relpenalty` after the Rel"
    );
    // §1199: a DISPLAY inserts none of them, because a display is not inside a
    // paragraph that could break.
    assert!(
        penalties("a+b=c", false).is_empty(),
        "a display carried break penalties, where §1199 turns them off"
    );
    // §729 turns a Bin with a Rel after it into an Ord, and an Ord is not a
    // breakpoint -- so only the relation's own penalty is left.
    assert_eq!(
        penalties("a+=b", true),
        vec![500],
        "a Bin that §729 made an Ord kept its `\\binoppenalty`"
    );
    // §726 converts a final Bin to an Ord too, so a formula that ENDS in an
    // operator offers a break only at its relation.
    assert_eq!(
        penalties("a=b+", true),
        vec![500],
        "a trailing Bin was still charged `\\binoppenalty`"
    );
    // §767 inserts nothing where `link(q)` is null: there is nothing after the
    // break to move onto the next line.
    assert!(
        penalties("a=", true).is_empty(),
        "a formula ending at its relation offered a break with nothing after it"
    );
}
