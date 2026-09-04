//! The mlist: what a formula is, before it is a list of boxes.
//!
//! `tex.web` §680-§698. An mlist is a linear sequence of NOADS -- "no-adds",
//! to keep them apart from the nodes of an hlist -- and it is a tree because a
//! noad's nucleus, subscript and superscript can each be an mlist of their own.
//! The classification into Ord, Op, Bin, Rel, Open, Close, Punct and Inner
//! (§682) is what the spacing table of §764 is indexed by, so it is carried on
//! the noad rather than worked out again later.

use crate::node::Node;

/// A dimension in scaled points.
pub type Scaled = i64;

/// `ord_noad` … `inner_noad` (§682), and the four that are not spacing classes.
///
/// The eight spacing classes are numbered exactly as `tex.web` numbers them
/// relative to `ord_noad`, because §766 indexes `math_spacing` with
/// `r_type*8+t` -- arithmetic on the number, not a case analysis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Ord = 0,
    Op = 1,
    Bin = 2,
    Rel = 3,
    Open = 4,
    Close = 5,
    Punct = 6,
    Inner = 7,
}

impl Class {
    /// The class a math code's class field names (§1155): `type(p) :=
    /// ord_noad + (c div "1000)`.
    pub fn from_code(c: i64) -> Class {
        match c {
            1 => Class::Op,
            2 => Class::Bin,
            3 => Class::Rel,
            4 => Class::Open,
            5 => Class::Close,
            6 => Class::Punct,
            7 => Class::Inner,
            _ => Class::Ord,
        }
    }
}

/// `subtype(p)` of an `op_noad` (§682).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Limits {
    /// `\displaylimits`: above and below in display style, beside otherwise.
    #[default]
    Normal,
    /// `\limits`: always above and below.
    Above,
    /// `\nolimits`: always beside.
    Beside,
}

/// A `math_char` (§681): a family and a character within it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MathChar {
    pub fam: usize,
    pub character: u8,
}

/// A delimiter field (§683): the small and large variants, each a family and a
/// character.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Delimiter {
    pub small_fam: usize,
    pub small_char: u8,
    pub large_fam: usize,
    pub large_char: u8,
}

impl Delimiter {
    /// `null_delimiter` (§685): the delimiter that is not there.
    pub fn null() -> Delimiter {
        Delimiter::default()
    }

    /// Whether this is the null delimiter, which §697 tests field by field.
    pub fn is_null(&self) -> bool {
        self.small_fam == 0 && self.small_char == 0 && self.large_fam == 0 && self.large_char == 0
    }

    /// A 24-bit `\delimiter` code's delimiter half (§1160): the low six hex
    /// digits, small variant first.
    pub fn from_code(code: i64) -> Delimiter {
        Delimiter {
            small_fam: ((code >> 20) & 0xF) as usize,
            small_char: ((code >> 12) & 0xFF) as u8,
            large_fam: ((code >> 8) & 0xF) as usize,
            large_char: (code & 0xFF) as u8,
        }
    }
}

/// One of a noad's three principal fields (§681).
#[derive(Clone, Debug, Default)]
pub enum Field {
    /// `math_type = empty`: the attribute is not present.
    #[default]
    Empty,
    /// `math_type = math_char`.
    Char(MathChar),
    /// `math_type = sub_box`: a box already built.
    Box(crate::node::BoxNode),
    /// `math_type = sub_mlist`: a formula that must be converted first. An
    /// EMPTY list here is not the same as `Empty` (§681): `$P_{}$` and `$P$`
    /// differ.
    List(Vec<Noad>),
    /// A character with no Computer Modern slot: what a document writes when
    /// it puts a `€` or a `→` inside a formula.
    ///
    /// Not a `math_type` of `tex.web`'s -- TeX has no such case, because every
    /// character it can set is in a family. It exists because this engine sets
    /// a page in whatever face the document asked for and only MEASURES in
    /// Computer Modern, so a character outside the four families still has
    /// somewhere to be drawn from. It is measured as an Ord of the text
    /// family's quad, since there is no slot to ask.
    Literal(char),
}

impl Field {
    pub fn is_empty(&self) -> bool {
        matches!(self, Field::Empty)
    }
}

/// The four styles (§688), and their cramped variants.
///
/// `tex.web` numbers them 0, 2, 4, 6 with 1 added to cramp, which makes a
/// SMALLER style a LARGER number -- backwards from Appendix G, and load-bearing
/// because §744 and §758 test `cur_style < text_style` to mean "display".
pub type Style = i64;

pub const DISPLAY_STYLE: Style = 0;
pub const TEXT_STYLE: Style = 2;
pub const SCRIPT_STYLE: Style = 4;
pub const SCRIPT_SCRIPT_STYLE: Style = 6;
pub const CRAMPED: Style = 1;

/// `cramped_style(#) == 2*(# div 2)+cramped` (§702).
pub fn cramped_style(s: Style) -> Style {
    2 * (s / 2) + CRAMPED
}
/// `sub_style(#) == 2*(# div 4)+script_style+cramped` (§702).
pub fn sub_style(s: Style) -> Style {
    2 * (s / 4) + SCRIPT_STYLE + CRAMPED
}
/// `sup_style(#) == 2*(# div 4)+script_style+(# mod 2)` (§702).
pub fn sup_style(s: Style) -> Style {
    2 * (s / 4) + SCRIPT_STYLE + (s % 2)
}
/// `num_style(#) == #+2-2*(# div 6)` (§702).
pub fn num_style(s: Style) -> Style {
    s + 2 - 2 * (s / 6)
}
/// `denom_style(#) == 2*(# div 2)+cramped+2-2*(# div 6)` (§702).
pub fn denom_style(s: Style) -> Style {
    2 * (s / 2) + CRAMPED + 2 - 2 * (s / 6)
}

/// `cur_size` for a style (§703): `text_size` above `script_style`, else
/// `16*((cur_style-text_style) div 2)` -- which is 0, 1, 2 in this port's
/// renumbering.
pub fn size_of(style: Style) -> usize {
    match style < SCRIPT_STYLE {
        true => crate::math::font::TEXT_SIZE,
        false => ((style - TEXT_STYLE) / 2) as usize,
    }
}

/// One item of an mlist: a noad, or one of the nodes §730 allows among them.
#[derive(Clone, Debug)]
pub enum Noad {
    /// A noad of one of the eight spacing classes, or one of the four that
    /// build something (§687): under, over, radical, fraction.
    Atom(Atom),
    /// A `fraction_noad` (§683). It has `numerator`, `denominator` and
    /// `thickness` where a normal noad has nucleus, sub and superscript, which
    /// is why it is not an `Atom`.
    Fraction(Fraction),
    /// A `radical_noad` (§683): a nucleus under a `left_delimiter`.
    Radical(Radical),
    /// An `over_noad` (§687): the nucleus, overlined.
    Over(Atom),
    /// An `under_noad` (§687): the nucleus, underlined.
    Under(Atom),
    /// A `left_noad` (§687). It appears only as the first item of an mlist.
    Left(Delimiter),
    /// A `right_noad` (§687). It appears only as the last.
    Right(Delimiter),
    /// A `style_node` (§688): `\displaystyle` and its three siblings.
    Style(Style),
    /// A `glue_node` in an mlist (§730), already in points rather than in mu.
    Glue(Scaled),
    /// A `kern_node` in an mlist (§730).
    Kern(Scaled),
    /// A box the formula carries whole -- what `\hbox{...}` and `\mathrm{...}`
    /// leave behind once their contents have been set as text.
    Node(Node),
}

/// A noad of one of the classes that has a nucleus and scripts (§681).
#[derive(Clone, Debug, Default)]
pub struct Atom {
    pub class: ClassOrOrd,
    pub nucleus: Field,
    pub supscr: Field,
    pub subscr: Field,
    /// `subtype` (§682), which only an `op_noad` reads.
    pub limits: Limits,
}

/// The class of an `Atom`, defaulting to Ord the way `new_noad` does (§686).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClassOrOrd(pub Class);

impl Default for ClassOrOrd {
    fn default() -> ClassOrOrd {
        ClassOrOrd(Class::Ord)
    }
}

impl Atom {
    pub fn new(class: Class, nucleus: Field) -> Atom {
        Atom {
            class: ClassOrOrd(class),
            nucleus,
            ..Atom::default()
        }
    }
    pub fn class(&self) -> Class {
        self.class.0
    }
}

/// A `fraction_noad` (§683).
#[derive(Clone, Debug)]
pub struct Fraction {
    /// `None` is §683's `default_code`: the `default_rule_thickness` of the
    /// current size, which is what `\over` and `\frac` mean.
    pub thickness: Option<Scaled>,
    pub numerator: Vec<Noad>,
    pub denominator: Vec<Noad>,
    pub left_delimiter: Delimiter,
    pub right_delimiter: Delimiter,
}

/// A `radical_noad` (§683).
#[derive(Clone, Debug)]
pub struct Radical {
    pub left_delimiter: Delimiter,
    pub nucleus: Atom,
}

/// The 64-digit string of §764, verbatim.
///
/// `"0"` no space, `"1"` a conditional thin space, `"2"` a thin space, `"3"` a
/// conditional medium space, `"4"` a conditional thick space, `"*"` an
/// impossible case -- and §766 reads it as `math_spacing[r_type*8 + t]`, which
/// is why the classes are numbered as they are.
///
/// "Conditional" is `\nonscript`: the space is inserted only when the current
/// style is above `script_style` (§766).
pub const MATH_SPACING: &[u8; 64] =
    b"0234000122*4000133**3**344*0400400*000000234000111*1111112341011";

/// What goes between a noad of class `left` and one of class `right`, as
/// §766's `case` reads §764's string.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Space {
    None,
    /// `\nonscript\mskip\thinmuskip`.
    ConditionalThin,
    /// `\mskip\thinmuskip`.
    Thin,
    /// `\nonscript\mskip\medmuskip`.
    ConditionalMedium,
    /// `\nonscript\mskip\thickmuskip`.
    ConditionalThick,
    /// `"*"`: a pair `mlist_to_hlist` never produces, because a Bin that would
    /// make one has already been turned into an Ord (§727-§729).
    Impossible,
}

/// §764's table, read.
pub fn spacing(left: Class, right: Class) -> Space {
    match MATH_SPACING[left as usize * 8 + right as usize] {
        b'0' => Space::None,
        b'1' => Space::ConditionalThin,
        b'2' => Space::Thin,
        b'3' => Space::ConditionalMedium,
        b'4' => Space::ConditionalThick,
        _ => Space::Impossible,
    }
}

/// The three math glues plain.tex sets (plain.tex:373-375), in `mu` scaled by
/// $2^{16}$ the way a `\mskip` holds them.
///
/// `\thinmuskip=3mu`, `\medmuskip=4mu plus 2mu minus 4mu`,
/// `\thickmuskip=5mu plus 5mu`.
pub const THIN_MU_SKIP: (i64, i64, i64) = (3 * 65536, 0, 0);
pub const MED_MU_SKIP: (i64, i64, i64) = (4 * 65536, 2 * 65536, 4 * 65536);
pub const THICK_MU_SKIP: (i64, i64, i64) = (5 * 65536, 5 * 65536, 0);

/// `\scriptspace=0.5pt` (plain.tex:347): the room reserved to the right of
/// every subscript and superscript (§757, §758).
pub const SCRIPT_SPACE: Scaled = 32768;

/// `\delimiterfactor=901` (plain.tex:327) and `\delimitershortfall=5pt`
/// (plain.tex:345), which together decide how tall a `\left(` grows (§762).
pub const DELIMITER_FACTOR: i64 = 901;
pub const DELIMITER_SHORTFALL: Scaled = 5 * 65536;

/// `\nulldelimiterspace=1.2pt` (plain.tex:346): the width `var_delimiter`
/// returns for a delimiter that is not there (§706).
pub const NULL_DELIMITER_SPACE: Scaled = 78643;

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the spec, so it is checked against The TeXbook's own
    /// reading of it (Chapter 18): an Ord beside a Bin takes a medium space,
    /// an Ord beside a Rel a thick one, and two Ords no space at all.
    #[test]
    fn the_spacing_table_is_the_one_in_section_764() {
        assert_eq!(MATH_SPACING.len(), 64);
        assert_eq!(spacing(Class::Ord, Class::Ord), Space::None);
        assert_eq!(spacing(Class::Ord, Class::Op), Space::Thin);
        assert_eq!(spacing(Class::Ord, Class::Bin), Space::ConditionalMedium);
        assert_eq!(spacing(Class::Ord, Class::Rel), Space::ConditionalThick);
        assert_eq!(spacing(Class::Ord, Class::Inner), Space::ConditionalThin);
        // A Bin cannot be followed by a Rel or a Close: §728 has already made
        // it an Ord by the time the second pass asks.
        assert_eq!(spacing(Class::Bin, Class::Rel), Space::Impossible);
        assert_eq!(spacing(Class::Bin, Class::Close), Space::Impossible);
        assert_eq!(spacing(Class::Open, Class::Ord), Space::None);
        assert_eq!(spacing(Class::Punct, Class::Ord), Space::ConditionalThin);
    }

    /// §702's style arithmetic, which decides the size of every subsidiary
    /// formula. The numbering runs backwards -- a smaller style is a larger
    /// number -- so these are the cases that catch a sign error.
    #[test]
    fn the_style_transitions_are_section_702s() {
        assert_eq!(sup_style(DISPLAY_STYLE), SCRIPT_STYLE);
        assert_eq!(sup_style(TEXT_STYLE), SCRIPT_STYLE);
        assert_eq!(sup_style(SCRIPT_STYLE), SCRIPT_SCRIPT_STYLE);
        assert_eq!(sup_style(SCRIPT_SCRIPT_STYLE), SCRIPT_SCRIPT_STYLE);
        // A subscript is always cramped.
        assert_eq!(sub_style(DISPLAY_STYLE), SCRIPT_STYLE + CRAMPED);
        assert_eq!(sub_style(SCRIPT_STYLE), SCRIPT_SCRIPT_STYLE + CRAMPED);
        // A numerator is one step smaller; a denominator one step smaller and
        // cramped. Neither goes below scriptscript.
        assert_eq!(num_style(DISPLAY_STYLE), TEXT_STYLE);
        assert_eq!(num_style(SCRIPT_SCRIPT_STYLE), SCRIPT_SCRIPT_STYLE);
        assert_eq!(denom_style(DISPLAY_STYLE), TEXT_STYLE + CRAMPED);
        assert_eq!(cramped_style(DISPLAY_STYLE), DISPLAY_STYLE + CRAMPED);
        assert_eq!(cramped_style(TEXT_STYLE + CRAMPED), TEXT_STYLE + CRAMPED);
    }

    /// §703: display and text are the text size, script the script size, and
    /// both cramped variants agree with their uncramped ones.
    #[test]
    fn the_size_of_a_style_is_section_703s() {
        assert_eq!(size_of(DISPLAY_STYLE), 0);
        assert_eq!(size_of(TEXT_STYLE + CRAMPED), 0);
        assert_eq!(size_of(SCRIPT_STYLE), 1);
        assert_eq!(size_of(SCRIPT_STYLE + CRAMPED), 1);
        assert_eq!(size_of(SCRIPT_SCRIPT_STYLE), 2);
        assert_eq!(size_of(SCRIPT_SCRIPT_STYLE + CRAMPED), 2);
    }
}
